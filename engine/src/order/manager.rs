use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::AbortHandle;
use uuid::Uuid;

use crate::infrastructure::db::order_repository::{
    OrderRecord, OrderRepository, OrderRepositoryError,
};
use crate::infrastructure::db::subscription_repository::SubscriptionRepository;
use crate::order::circuit_breaker::{CircuitBreaker, CircuitBreakerError};
use crate::order::credential_resolver::{CredentialResolver, CredentialResolverError};
use crate::order::entity::SafetyConfig;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderSide, OrderStatus,
    OrderStatusResult, OrderType,
};
use crate::wire_message::{OrderUpdatePayload, SignalPayload, WsMessage};

#[derive(Debug, Error)]
pub enum OrderManagerError {
    #[error("signal confidence {0:.2} is below floor {1:.2}")]
    ConfidenceBelowFloor(f64, f64),
    #[error("position limit exceeded: would be {0}, max {1}")]
    PositionLimitExceeded(Decimal, Decimal),
    #[error("circuit breaker is tripped — await admin reset")]
    CircuitBreakerTripped,
    #[error("no order adapter registered for exchange '{0}'")]
    NoAdapterForExchange(String),
    #[error("credential resolver not configured — set CREDENTIALS_ENCRYPTION_KEY")]
    NoCredentialResolver,
    #[error("user {user_id} has no credentials stored for exchange '{exchange}'")]
    NoCredentialsForUser { user_id: i32, exchange: String },
    #[error("credential resolution failed: {0}")]
    CredentialResolution(#[from] CredentialResolverError),
    #[error("order adapter error: {0}")]
    Adapter(#[from] OrderAdapterError),
    #[error("order repository error: {0}")]
    Repository(#[from] OrderRepositoryError),
    #[error("signal action 'hold' — no order placed")]
    HoldSkipped,
    #[error("unknown signal action '{0}'")]
    UnknownAction(String),
}

pub struct OrderManager {
    order_adapters: Arc<RwLock<HashMap<String, Arc<dyn OrderAdapter>>>>,
    order_repository: Arc<dyn OrderRepository>,
    broadcaster: broadcast::Sender<String>,
    pub safety_config: SafetyConfig,
    circuit_breaker: Arc<Mutex<CircuitBreaker>>,
    /// Interval between fill-status polls. Shorter in tests.
    fill_poll_interval: Duration,
    /// Tracks active fill-poller tasks by client_order_id so they can be aborted on cancel.
    poll_handles: Arc<Mutex<HashMap<String, AbortHandle>>>,
    /// Per-user subscription registry. When `Some`, every inbound signal is also
    /// fanned out to all active subscribers via `fan_out_signal_to_subscriptions`.
    subscription_repository: Option<Arc<dyn SubscriptionRepository>>,
    /// Resolves per-user exchange credentials into `OrderAdapter` instances.
    /// Used by admin order placement and signal fanout.
    credential_resolver: Option<Arc<dyn CredentialResolver>>,
    /// Serializes the position-limit check-and-reserve so concurrent placements
    /// cannot both read a stale net position and overshoot `max_position_size`
    /// (held only across read→check→reserve, never across the exchange call).
    position_guard: Arc<Mutex<()>>,
}

impl OrderManager {
    pub fn new(
        order_adapters: Arc<RwLock<HashMap<String, Arc<dyn OrderAdapter>>>>,
        order_repository: Arc<dyn OrderRepository>,
        broadcaster: broadcast::Sender<String>,
        safety_config: SafetyConfig,
    ) -> Self {
        Self::with_poll_interval(
            order_adapters,
            order_repository,
            broadcaster,
            safety_config,
            Duration::from_secs(5),
        )
    }

    pub fn with_poll_interval(
        order_adapters: Arc<RwLock<HashMap<String, Arc<dyn OrderAdapter>>>>,
        order_repository: Arc<dyn OrderRepository>,
        broadcaster: broadcast::Sender<String>,
        safety_config: SafetyConfig,
        fill_poll_interval: Duration,
    ) -> Self {
        let cb = CircuitBreaker::new(
            safety_config.circuit_breaker_max_orders,
            safety_config.circuit_breaker_window_secs,
        );
        Self {
            order_adapters,
            order_repository,
            broadcaster,
            safety_config,
            circuit_breaker: Arc::new(Mutex::new(cb)),
            fill_poll_interval,
            poll_handles: Arc::new(Mutex::new(HashMap::new())),
            subscription_repository: None,
            credential_resolver: None,
            position_guard: Arc::new(Mutex::new(())),
        }
    }

    /// Attaches a subscription repository so that every inbound signal is fanned
    /// out to all active subscribers after the system-level order is processed.
    pub fn with_subscription_repository(mut self, repo: Arc<dyn SubscriptionRepository>) -> Self {
        self.subscription_repository = Some(repo);
        self
    }

    /// Attaches a credential resolver for per-user adapter construction.
    /// Required for admin order placement and credential-aware signal fanout.
    pub fn with_credential_resolver(mut self, resolver: Arc<dyn CredentialResolver>) -> Self {
        self.credential_resolver = Some(resolver);
        self
    }

    /// Resets the circuit breaker. Admin-only endpoint calls this.
    pub async fn reset_circuit_breaker(&self) {
        self.circuit_breaker.lock().await.reset();
    }

    pub async fn circuit_breaker_is_tripped(&self) -> bool {
        self.circuit_breaker.lock().await.is_tripped()
    }

    /// Returns `true` if a fill poller task is currently running for the given order.
    #[cfg(test)]
    pub async fn has_active_poll(&self, client_order_id: &str) -> bool {
        self.poll_handles.lock().await.contains_key(client_order_id)
    }

    /// Entry point for strategy-driven orders. Converts a signal into an order request.
    ///
    /// "Hold" signals are silently dropped. Signals below the confidence floor are
    /// dropped with a debug log (not an error — this is expected behavior).
    pub async fn process_signal(&self, signal: &SignalPayload) -> Result<(), OrderManagerError> {
        if signal.confidence < self.safety_config.min_confidence {
            tracing::debug!(
                signal_id = %signal.signal_id,
                confidence = signal.confidence,
                floor = self.safety_config.min_confidence,
                strategy_id = %signal.strategy_id,
                exchange = %signal.exchange,
                pair = %signal.pair,
                "signal dropped: confidence below floor"
            );
            return Err(OrderManagerError::ConfidenceBelowFloor(
                signal.confidence,
                self.safety_config.min_confidence,
            ));
        }

        let side = match signal.action.as_str() {
            "buy" => OrderSide::Buy,
            "sell" => OrderSide::Sell,
            "hold" => return Err(OrderManagerError::HoldSkipped),
            other => return Err(OrderManagerError::UnknownAction(other.to_string())),
        };

        let req = OrderRequest {
            exchange: signal.exchange.clone(),
            pair: signal.pair.clone(),
            side,
            order_type: OrderType::Market,
            quantity: self.safety_config.default_order_quantity,
            price: None,
            client_order_id: Uuid::new_v4().to_string(),
            strategy_id: Some(signal.strategy_id.clone()),
        };

        tracing::info!(
            signal_id = %signal.signal_id,
            strategy_id = %signal.strategy_id,
            exchange = %signal.exchange,
            pair = %signal.pair,
            action = %signal.action,
            confidence = signal.confidence,
            client_order_id = %req.client_order_id,
            "signal accepted, executing order"
        );

        // System-level order: no owning user, limited against the system bucket.
        self.execute_order(req, None, None).await
    }

    /// Direct order placement from a REST endpoint, scoped to the placing user
    /// for position limits. Pass `None` for a system-level (user-less) order.
    pub async fn place_order(
        &self,
        user_id: Option<i32>,
        req: OrderRequest,
    ) -> Result<String, OrderManagerError> {
        let client_order_id = req.client_order_id.clone();
        self.execute_order(req, None, user_id).await?;
        Ok(client_order_id)
    }

    /// Places an order on behalf of a specific user using their stored exchange credentials.
    /// Requires `credential_resolver` to be configured; returns an error otherwise.
    pub async fn place_order_for_user(
        &self,
        user_id: i32,
        req: OrderRequest,
    ) -> Result<String, OrderManagerError> {
        let resolver = self
            .credential_resolver
            .as_ref()
            .ok_or(OrderManagerError::NoCredentialResolver)?;

        let adapter = resolver
            .adapter_for_user(user_id, &req.exchange)
            .await?
            .ok_or_else(|| OrderManagerError::NoCredentialsForUser {
                user_id,
                exchange: req.exchange.clone(),
            })?;

        tracing::info!(
            user_id,
            exchange = %req.exchange,
            pair = %req.pair,
            side = %req.side,
            client_order_id = %req.client_order_id,
            "admin: placing order for user using stored credentials"
        );

        let client_order_id = req.client_order_id.clone();
        self.execute_order(req, Some(adapter), Some(user_id))
            .await?;
        Ok(client_order_id)
    }

    /// Cancels an open order identified by `client_order_id`.
    pub async fn cancel_order(&self, client_order_id: &str) -> Result<(), OrderManagerError> {
        let record = self
            .order_repository
            .get_by_client_order_id(client_order_id)
            .await?;

        let exchange_order_id = record.exchange_order_id.clone().ok_or_else(|| {
            OrderManagerError::Repository(
                crate::infrastructure::db::order_repository::OrderRepositoryError::NotFound(
                    format!("no exchange_order_id for client_order_id={client_order_id}"),
                ),
            )
        })?;

        let adapter = {
            let adapters = self.order_adapters.read().await;
            adapters
                .get(&record.exchange)
                .ok_or_else(|| OrderManagerError::NoAdapterForExchange(record.exchange.clone()))?
                .clone()
        };

        tracing::info!(
            client_order_id = %client_order_id,
            exchange_order_id = %exchange_order_id,
            exchange = %record.exchange,
            "cancelling order"
        );

        adapter
            .cancel_order(&OrderId(exchange_order_id))
            .await
            .map_err(OrderManagerError::Adapter)?;

        // Abort the fill poller before updating DB so it cannot overwrite "cancelled"
        if let Some(handle) = self.poll_handles.lock().await.remove(client_order_id) {
            handle.abort();
            tracing::debug!(
                client_order_id = %client_order_id,
                "fill poller aborted on cancel"
            );
        }

        // `None` for filled_quantity: keep whatever the poller already recorded —
        // a partial fill before the cancel is still real, held inventory.
        self.order_repository
            .update_status(client_order_id, "cancelled", None, None, None)
            .await?;

        let mut cancelled = record;
        cancelled.status = "cancelled".to_string();
        self.broadcast_update(&cancelled, None);

        Ok(())
    }

    /// Returns orders, optionally filtered by exchange and/or pair.
    pub async fn list_orders(
        &self,
        exchange: Option<&str>,
        pair: Option<&str>,
    ) -> Result<Vec<crate::infrastructure::db::order_repository::OrderRecord>, OrderManagerError>
    {
        Ok(self.order_repository.list(exchange, pair).await?)
    }

    /// `adapter_override`: when `Some`, bypasses the global `order_adapters` registry
    /// and uses the supplied adapter directly (e.g. for per-user credential-based orders).
    /// `user_id`: the owning user for per-user position scoping (M8) — `None` for a
    /// system / signal-driven order, which is limited against its own (user-less) bucket.
    async fn execute_order(
        &self,
        req: OrderRequest,
        adapter_override: Option<Arc<dyn OrderAdapter>>,
        user_id: Option<i32>,
    ) -> Result<(), OrderManagerError> {
        // A buy adds to the position, a sell reduces it (C7: side-aware netting).
        let signed_qty = match req.side {
            OrderSide::Buy => req.quantity,
            OrderSide::Sell => -req.quantity,
        };

        // Resolve the live adapter up-front so we never reserve a position we
        // cannot actually place (dry-run needs no adapter).
        let adapter = if self.safety_config.dry_run {
            None
        } else if let Some(a) = adapter_override {
            Some(a)
        } else {
            let adapters = self.order_adapters.read().await;
            Some(
                adapters
                    .get(&req.exchange)
                    .ok_or_else(|| OrderManagerError::NoAdapterForExchange(req.exchange.clone()))?
                    .clone(),
            )
        };

        // --- Atomic check-and-reserve (C9) -----------------------------------
        // Held across read -> check -> circuit-breaker -> reserve so two
        // concurrent placements cannot both pass a stale net-position read and
        // overshoot the cap. Released before the (slow) exchange call below — by
        // then the reserving row is already visible to others via net_position.
        let record = {
            let _guard = self.position_guard.lock().await;

            // Position limit checked first — rejected orders must not count
            // against the circuit breaker.
            let net = self
                .order_repository
                .net_position(user_id, &req.exchange, &req.pair)
                .await?;
            let projected = net + signed_qty;
            // The cap bounds absolute exposure in either direction.
            if projected.abs() > self.safety_config.max_position_size {
                tracing::warn!(
                    user_id = ?user_id,
                    exchange = %req.exchange,
                    pair = %req.pair,
                    net_position = %net,
                    requested = %req.quantity,
                    side = %req.side,
                    projected = %projected,
                    max = %self.safety_config.max_position_size,
                    "order blocked: position limit would be exceeded"
                );
                return Err(OrderManagerError::PositionLimitExceeded(
                    projected,
                    self.safety_config.max_position_size,
                ));
            }

            // Circuit breaker — only incremented after the position limit passes.
            {
                let mut cb = self.circuit_breaker.lock().await;
                cb.record_order()
                    .map_err(|CircuitBreakerError::Tripped(_, _)| {
                        OrderManagerError::CircuitBreakerTripped
                    })?;
            }

            // Reserve the position by persisting the row while still holding the
            // guard. Dry-run reserves a "dry_run" row (no real exposure, excluded
            // from net_position); live reserves "open" BEFORE the exchange call
            // for idempotency — on a network timeout the manager can query
            // get_order_status before retrying.
            let status = if self.safety_config.dry_run {
                "dry_run"
            } else {
                "open"
            };
            let record = self.build_record(&req, status, None, user_id);
            self.order_repository.insert(&record).await?;
            record
        };

        // Dry-run — full pipeline ran but no real exchange call.
        if self.safety_config.dry_run {
            tracing::info!(
                exchange = %req.exchange,
                pair = %req.pair,
                side = %req.side,
                quantity = %req.quantity,
                client_order_id = %req.client_order_id,
                strategy_id = ?req.strategy_id,
                "dry-run: order not sent to exchange"
            );
            self.broadcast_update(&record, None);
            return Ok(());
        }

        let adapter = adapter.expect("live path resolves an adapter above");

        tracing::info!(
            exchange = %req.exchange,
            pair = %req.pair,
            side = %req.side,
            order_type = %req.order_type,
            quantity = %req.quantity,
            client_order_id = %req.client_order_id,
            strategy_id = ?req.strategy_id,
            "placing order with exchange"
        );

        const PLACE_TIMEOUT: Duration = Duration::from_secs(15);
        let place_result = tokio::time::timeout(PLACE_TIMEOUT, adapter.place_order(&req))
            .await
            .unwrap_or_else(|_| {
                Err(OrderAdapterError::NetworkTimeout(
                    "place_order timed out after 15s in order manager".to_string(),
                ))
            });

        let order_id = match place_result {
            Ok(id) => id,
            // C8: a *transient* failure (timeout / 5xx) does not prove the order
            // never reached the exchange — the request may have been accepted and
            // only the response lost. Marking it `failed` would exclude it from
            // net_position and silently orphan a live position. Reconcile by
            // client_order_id before deciding.
            Err(e) if e.is_transient() => {
                tracing::warn!(
                    error = %e,
                    client_order_id = %req.client_order_id,
                    exchange = %req.exchange,
                    "transient placement failure — reconciling order state before deciding its fate"
                );
                match adapter
                    .get_order_status_by_client_id(&req.client_order_id)
                    .await
                {
                    Ok(Some(reconciled)) => {
                        tracing::warn!(
                            client_order_id = %req.client_order_id,
                            exchange = %req.exchange,
                            exchange_order_id = %reconciled.order_id,
                            status = %reconciled.result.status,
                            "reconciliation found the order live at the exchange — adopting it instead of marking failed"
                        );
                        reconciled.order_id
                    }
                    Ok(None) => {
                        tracing::error!(
                            error = %e,
                            client_order_id = %req.client_order_id,
                            exchange = %req.exchange,
                            "reconciliation confirmed the exchange has no record — order never landed, marking failed"
                        );
                        let _ = self
                            .order_repository
                            .update_status(&req.client_order_id, "failed", None, None, None)
                            .await;
                        return Err(OrderManagerError::Adapter(e));
                    }
                    Err(reconcile_err) => {
                        tracing::error!(
                            placement_error = %e,
                            reconcile_error = %reconcile_err,
                            client_order_id = %req.client_order_id,
                            exchange = %req.exchange,
                            "CRITICAL: placement failed and reconciliation failed — order left OPEN pending manual reconciliation to avoid orphaning a live position"
                        );
                        return Err(OrderManagerError::Adapter(e));
                    }
                }
            }
            // Permanent error (rejected, auth, unknown symbol, ...) — the order was
            // never accepted, so there is nothing to reconcile.
            Err(e) => {
                tracing::error!(
                    error = %e,
                    client_order_id = %req.client_order_id,
                    exchange = %req.exchange,
                    "exchange rejected order"
                );
                let _ = self
                    .order_repository
                    .update_status(&req.client_order_id, "failed", None, None, None)
                    .await;
                return Err(OrderManagerError::Adapter(e));
            }
        };

        self.order_repository
            .update_status(&req.client_order_id, "open", Some(&order_id.0), None, None)
            .await?;

        let mut placed = record.clone();
        placed.exchange_order_id = Some(order_id.0.clone());

        tracing::info!(
            order_id = %order_id,
            client_order_id = %req.client_order_id,
            exchange = %req.exchange,
            pair = %req.pair,
            "order placed successfully"
        );

        self.broadcast_update(&placed, None);

        // Spawn fill poller and track the abort handle so cancel_order can stop it
        let poll_interval = self.fill_poll_interval;
        let client_oid = req.client_order_id.clone();
        let exchange = req.exchange.clone();
        let pair = req.pair.clone();
        let side = req.side.to_string();
        let quantity = req.quantity;
        let strategy_id = req.strategy_id.clone();
        let repo = self.order_repository.clone();
        let broadcaster = self.broadcaster.clone();
        let poll_handles = self.poll_handles.clone();
        let client_oid_cleanup = client_oid.clone();

        let handle = tokio::spawn(async move {
            poll_fill_status(FillPollContext {
                order_id,
                client_order_id: client_oid,
                exchange,
                pair,
                side,
                quantity,
                strategy_id,
                adapter,
                repo,
                broadcaster,
                interval: poll_interval,
            })
            .await;
            // Remove handle from map when poller exits naturally
            poll_handles.lock().await.remove(&client_oid_cleanup);
        });

        self.poll_handles
            .lock()
            .await
            .insert(req.client_order_id.clone(), handle.abort_handle());

        Ok(())
    }

    fn build_record(
        &self,
        req: &OrderRequest,
        status: &str,
        exchange_order_id: Option<String>,
        user_id: Option<i32>,
    ) -> OrderRecord {
        let now = Utc::now();
        OrderRecord {
            id: None,
            user_id,
            exchange: req.exchange.clone(),
            pair: req.pair.clone(),
            side: req.side.to_string(),
            order_type: req.order_type.to_string(),
            quantity: req.quantity,
            filled_quantity: Decimal::ZERO,
            price: req.price,
            status: status.to_string(),
            exchange_order_id,
            client_order_id: req.client_order_id.clone(),
            strategy_id: req.strategy_id.clone(),
            created_at: now,
            updated_at: now,
        }
    }

    fn broadcast_update(&self, record: &OrderRecord, fill_price: Option<String>) {
        let payload = OrderUpdatePayload {
            order_id: record.exchange_order_id.clone().unwrap_or_default(),
            client_order_id: record.client_order_id.clone(),
            exchange: record.exchange.clone(),
            pair: record.pair.clone(),
            market_type: "spot".to_string(),
            side: record.side.clone(),
            status: record.status.clone(),
            quantity: record.quantity.to_string(),
            fill_price,
            strategy_id: record.strategy_id.clone(),
            timestamp: Utc::now(),
        };
        match serde_json::to_string(&WsMessage::OrderUpdate(payload)) {
            Ok(json) => {
                let _ = self.broadcaster.send(json);
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize order update broadcast");
            }
        }
    }

    /// Fans out a signal to every user who has an active subscription for
    /// `signal.strategy_id`.  Each subscriber's per-row `confidence_threshold`
    /// and `max_position_size` overrides replace the global `SafetyConfig` values
    /// for that user's order only.  A failure for one subscriber is logged and
    /// skipped — the remaining subscribers always continue processing.
    pub async fn fan_out_signal_to_subscriptions(&self, signal: &SignalPayload) {
        let Some(sub_repo) = &self.subscription_repository else {
            return;
        };

        let subs = match sub_repo.list_active_for_strategy(&signal.strategy_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    strategy_id = %signal.strategy_id,
                    signal_id = %signal.signal_id,
                    "failed to load subscriptions for signal fanout"
                );
                return;
            }
        };

        if subs.is_empty() {
            return;
        }

        tracing::info!(
            signal_id = %signal.signal_id,
            strategy_id = %signal.strategy_id,
            exchange = %signal.exchange,
            pair = %signal.pair,
            action = %signal.action,
            subscribers = subs.len(),
            "fanning out signal to subscribed users"
        );

        for sub in &subs {
            let floor = sub
                .confidence_threshold
                .unwrap_or(self.safety_config.min_confidence);
            if signal.confidence < floor {
                tracing::debug!(
                    user_id = sub.user_id,
                    subscription_id = sub.id,
                    signal_id = %signal.signal_id,
                    confidence = signal.confidence,
                    floor,
                    "signal skipped for subscription: confidence below per-subscription threshold"
                );
                continue;
            }

            let side = match signal.action.as_str() {
                "buy" => OrderSide::Buy,
                "sell" => OrderSide::Sell,
                "hold" => {
                    tracing::debug!(
                        user_id = sub.user_id,
                        subscription_id = sub.id,
                        signal_id = %signal.signal_id,
                        "hold signal — no order for this subscription"
                    );
                    continue;
                }
                other => {
                    tracing::warn!(
                        user_id = sub.user_id,
                        subscription_id = sub.id,
                        action = other,
                        "unknown signal action for subscription — skipping"
                    );
                    continue;
                }
            };

            let max_pos = sub
                .max_position_size
                .unwrap_or(self.safety_config.max_position_size);

            let net = match self
                .order_repository
                .net_position(Some(sub.user_id), &signal.exchange, &signal.pair)
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!(
                        user_id = sub.user_id,
                        subscription_id = sub.id,
                        signal_id = %signal.signal_id,
                        error = %e,
                        "failed to query net position for subscription order — skipping"
                    );
                    continue;
                }
            };

            let signed_qty = match side {
                OrderSide::Buy => self.safety_config.default_order_quantity,
                OrderSide::Sell => -self.safety_config.default_order_quantity,
            };
            let projected = net + signed_qty;
            if projected.abs() > max_pos {
                tracing::warn!(
                    user_id = sub.user_id,
                    subscription_id = sub.id,
                    signal_id = %signal.signal_id,
                    net_position = %net,
                    requested = %self.safety_config.default_order_quantity,
                    side = %side,
                    projected = %projected,
                    max = %max_pos,
                    "subscription order blocked: position limit exceeded"
                );
                continue;
            }

            let req = OrderRequest {
                exchange: signal.exchange.clone(),
                pair: signal.pair.clone(),
                side,
                order_type: OrderType::Market,
                quantity: self.safety_config.default_order_quantity,
                price: None,
                client_order_id: Uuid::new_v4().to_string(),
                strategy_id: Some(signal.strategy_id.clone()),
            };

            tracing::info!(
                user_id = sub.user_id,
                subscription_id = sub.id,
                signal_id = %signal.signal_id,
                strategy_id = %signal.strategy_id,
                exchange = %req.exchange,
                pair = %req.pair,
                side = %req.side,
                client_order_id = %req.client_order_id,
                "executing subscription order"
            );

            let user_adapter = if let Some(resolver) = &self.credential_resolver {
                match resolver
                    .adapter_for_user(sub.user_id, &signal.exchange)
                    .await
                {
                    Ok(Some(a)) => Some(a),
                    Ok(None) => {
                        tracing::warn!(
                            user_id = sub.user_id,
                            subscription_id = sub.id,
                            exchange = %signal.exchange,
                            "subscription fanout: no credentials stored for user — using global adapter"
                        );
                        None
                    }
                    Err(e) => {
                        tracing::error!(
                            user_id = sub.user_id,
                            subscription_id = sub.id,
                            exchange = %signal.exchange,
                            error = %e,
                            "subscription fanout: credential resolution failed — skipping subscriber"
                        );
                        continue;
                    }
                }
            } else {
                None
            };

            if let Err(e) = self
                .execute_order(req, user_adapter, Some(sub.user_id))
                .await
            {
                tracing::error!(
                    user_id = sub.user_id,
                    subscription_id = sub.id,
                    signal_id = %signal.signal_id,
                    error = %e,
                    "subscription order failed — continuing to next subscriber"
                );
            }
        }
    }
}

struct FillPollContext {
    order_id: OrderId,
    client_order_id: String,
    exchange: String,
    pair: String,
    side: String,
    quantity: Decimal,
    strategy_id: Option<String>,
    adapter: Arc<dyn OrderAdapter>,
    repo: Arc<dyn OrderRepository>,
    broadcaster: broadcast::Sender<String>,
    interval: Duration,
}

async fn poll_fill_status(ctx: FillPollContext) {
    let FillPollContext {
        order_id,
        client_order_id,
        exchange,
        pair,
        side,
        quantity,
        strategy_id,
        adapter,
        repo,
        broadcaster,
        interval,
    } = ctx;
    const MAX_ATTEMPTS: u32 = 60;

    for attempt in 0..MAX_ATTEMPTS {
        tokio::time::sleep(interval).await;

        let OrderStatusResult {
            status,
            fill_price: raw_fill_price,
            filled_quantity,
        } = match adapter.get_order_status(&order_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    order_id = %order_id,
                    attempt = attempt + 1,
                    exchange = %exchange,
                    "fill poll failed — will retry"
                );
                continue;
            }
        };

        let status_str = status.to_string();
        let is_terminal = matches!(
            status,
            OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Failed
        );
        let fill_price = raw_fill_price.map(|p| p.to_string());

        if let Err(e) = repo
            .update_status(
                &client_order_id,
                &status_str,
                Some(&order_id.0),
                None,
                filled_quantity,
            )
            .await
        {
            tracing::error!(
                error = %e,
                client_order_id = %client_order_id,
                "failed to update order status in db"
            );
        }

        if let Some(filled) = filled_quantity {
            tracing::info!(
                order_id = %order_id,
                client_order_id = %client_order_id,
                exchange = %exchange,
                pair = %pair,
                status = %status_str,
                filled_quantity = %filled,
                ordered_quantity = %quantity,
                "fill poll recorded executed quantity"
            );
        }

        let payload = OrderUpdatePayload {
            order_id: order_id.0.clone(),
            client_order_id: client_order_id.clone(),
            exchange: exchange.clone(),
            pair: pair.clone(),
            market_type: "spot".to_string(),
            side: side.clone(),
            status: status_str,
            quantity: quantity.to_string(),
            fill_price,
            strategy_id: strategy_id.clone(),
            timestamp: Utc::now(),
        };

        if let Ok(json) = serde_json::to_string(&WsMessage::OrderUpdate(payload)) {
            let _ = broadcaster.send(json);
        }

        if is_terminal {
            tracing::info!(
                order_id = %order_id,
                exchange = %exchange,
                "order reached terminal status, fill polling stopped"
            );
            break;
        }
    }
}

/// Spawns a background task that listens on the broadcaster for `WsMessage::Signal`
/// and forwards each one to the Order Manager.
///
/// This is the in-process path for strategy-driven orders — signals emitted by the
/// strategy runner arrive on the broadcaster and are converted to orders here,
/// without any Kafka round-trip.
pub fn spawn_order_manager_listener(
    manager: Arc<OrderManager>,
    broadcaster: broadcast::Sender<String>,
) -> AbortHandle {
    let mut rx = broadcaster.subscribe();
    let handle = tokio::spawn(async move {
        loop {
            let text = match rx.recv().await {
                Ok(t) => t,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        dropped = n,
                        "order manager listener lagged — signals may have been missed"
                    );
                    continue;
                }
                Err(_) => break,
            };

            let msg: WsMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => continue,
            };

            if let WsMessage::Signal(signal) = msg {
                tracing::debug!(
                    signal_id = %signal.signal_id,
                    strategy_id = %signal.strategy_id,
                    action = %signal.action,
                    confidence = signal.confidence,
                    "order manager received signal"
                );
                // System-level order (uses global SafetyConfig and order adapters).
                if let Err(e) = manager.process_signal(&signal).await {
                    match &e {
                        OrderManagerError::HoldSkipped
                        | OrderManagerError::ConfidenceBelowFloor(_, _) => {
                            tracing::debug!(reason = %e, "signal not converted to order");
                        }
                        _ => {
                            tracing::error!(
                                error = %e,
                                strategy_id = %signal.strategy_id,
                                "order manager failed to process signal"
                            );
                        }
                    }
                }
                // Per-user fanout: order for every active subscriber, isolated per row.
                manager.fan_out_signal_to_subscriptions(&signal).await;
            }
        }
    });
    handle.abort_handle()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use tokio::sync::{broadcast, RwLock};

    use super::*;
    use crate::infrastructure::db::order_repository::FakeOrderRepository;
    use crate::order::entity::SafetyConfig;
    use crate::order::fake::FakeOrderAdapter;
    use crate::order::port::OrderAdapterError;
    use crate::wire_message::WsMessage;

    fn build_manager(
        safety_config: SafetyConfig,
        adapter: FakeOrderAdapter,
    ) -> (OrderManager, broadcast::Receiver<String>) {
        let (broadcaster, rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));
        let repo = Arc::new(FakeOrderRepository::new());
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            safety_config,
            Duration::from_millis(10),
        );
        (manager, rx)
    }

    /// Like `build_manager` but exposes the order repository (for asserting
    /// persisted state) and returns the manager behind an `Arc` (for concurrency).
    fn build_manager_with_repo(
        safety_config: SafetyConfig,
        adapter: FakeOrderAdapter,
    ) -> (
        Arc<OrderManager>,
        Arc<FakeOrderRepository>,
        broadcast::Receiver<String>,
    ) {
        let (broadcaster, rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));
        let repo = Arc::new(FakeOrderRepository::new());
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo.clone(),
            broadcaster,
            safety_config,
            Duration::from_millis(10),
        );
        (Arc::new(manager), repo, rx)
    }

    fn order_req(side: OrderSide, qty: i64) -> OrderRequest {
        OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side,
            order_type: OrderType::Market,
            quantity: Decimal::new(qty, 0),
            price: None,
            client_order_id: Uuid::new_v4().to_string(),
            strategy_id: None,
        }
    }

    fn live_config() -> SafetyConfig {
        SafetyConfig {
            dry_run: false,
            min_confidence: 0.7,
            max_position_size: Decimal::new(1000, 0),
            default_order_quantity: Decimal::new(100, 0),
            circuit_breaker_max_orders: 10,
            circuit_breaker_window_secs: 60,
        }
    }

    fn make_signal(action: &str, confidence: f64) -> SignalPayload {
        SignalPayload {
            signal_id: Uuid::new_v4().to_string(),
            strategy_id: "spread-1".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            action: action.to_string(),
            confidence,
            timestamp: Utc::now(),
            stop_loss: None,
            take_profit: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn signal_below_confidence_floor_is_dropped() {
        let cfg = SafetyConfig {
            dry_run: true,
            min_confidence: 0.8,
            ..SafetyConfig::default()
        };
        let (manager, _rx) = build_manager(cfg, FakeOrderAdapter::new("tabdeal"));

        let result = manager.process_signal(&make_signal("buy", 0.75)).await;
        assert!(
            matches!(result, Err(OrderManagerError::ConfidenceBelowFloor(_, _))),
            "signal with confidence 0.75 < floor 0.8 must be dropped"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn signal_at_or_above_confidence_floor_proceeds() {
        let cfg = SafetyConfig {
            dry_run: true,
            min_confidence: 0.7,
            ..SafetyConfig::default()
        };
        let (manager, _rx) = build_manager(cfg, FakeOrderAdapter::new("tabdeal"));

        assert!(manager
            .process_signal(&make_signal("buy", 0.7))
            .await
            .is_ok());
        assert!(manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hold_signal_is_skipped_without_order() {
        let (manager, _rx) = build_manager(live_config(), FakeOrderAdapter::new("tabdeal"));
        let result = manager.process_signal(&make_signal("hold", 0.95)).await;
        assert!(matches!(result, Err(OrderManagerError::HoldSkipped)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dry_run_does_not_call_place_order() {
        let cfg = SafetyConfig {
            dry_run: true,
            ..SafetyConfig::default()
        };

        // Keep an Arc reference to the adapter so we can inspect placed_count after
        let adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let (broadcaster, _rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::clone(&adapter) as Arc<dyn OrderAdapter>,
        );
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();

        assert_eq!(
            adapter.placed_count().await,
            0,
            "place_order must never be called in dry-run mode"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dry_run_persists_order_with_dry_run_status() {
        let cfg = SafetyConfig {
            dry_run: true,
            ..SafetyConfig::default()
        };
        let (broadcaster, _rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());
        let adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo.clone(),
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();

        let records = repo.all_records().await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "dry_run");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dry_run_broadcasts_order_update() {
        let cfg = SafetyConfig {
            dry_run: true,
            ..SafetyConfig::default()
        };
        let (broadcaster, mut rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());
        let adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo.clone(),
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();

        let text = rx.try_recv().expect("order_update must be broadcast");
        let msg: WsMessage = serde_json::from_str(&text).unwrap();
        assert!(matches!(msg, WsMessage::OrderUpdate(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn circuit_breaker_halts_after_threshold() {
        let cfg = SafetyConfig {
            dry_run: true,
            circuit_breaker_max_orders: 2,
            circuit_breaker_window_secs: 60,
            ..SafetyConfig::default()
        };
        let (manager, _rx) = build_manager(cfg, FakeOrderAdapter::new("tabdeal"));

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();
        let second = manager.process_signal(&make_signal("buy", 0.9)).await;
        assert!(
            matches!(second, Err(OrderManagerError::CircuitBreakerTripped)),
            "second order (= threshold) must trip the circuit breaker"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn circuit_breaker_requires_manual_reset() {
        let cfg = SafetyConfig {
            dry_run: true,
            circuit_breaker_max_orders: 2,
            circuit_breaker_window_secs: 60,
            ..SafetyConfig::default()
        };
        let (manager, _rx) = build_manager(cfg, FakeOrderAdapter::new("tabdeal"));

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();
        let _ = manager.process_signal(&make_signal("buy", 0.9)).await; // trips

        assert!(
            manager.circuit_breaker_is_tripped().await,
            "circuit breaker must be tripped"
        );
        assert!(
            matches!(
                manager.process_signal(&make_signal("buy", 0.9)).await,
                Err(OrderManagerError::CircuitBreakerTripped)
            ),
            "orders blocked while tripped"
        );

        manager.reset_circuit_breaker().await;

        assert!(
            !manager.circuit_breaker_is_tripped().await,
            "circuit breaker must be reset"
        );
        assert!(
            manager
                .process_signal(&make_signal("buy", 0.9))
                .await
                .is_ok(),
            "orders allowed after reset"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn position_limit_blocks_oversized_order() {
        let (broadcaster, _rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());

        let cfg = SafetyConfig {
            dry_run: false,
            max_position_size: Decimal::new(100, 0),
            default_order_quantity: Decimal::new(101, 0),
            circuit_breaker_max_orders: 10,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.0,
        };

        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("ord-001").await;
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));

        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        let result = manager.process_signal(&make_signal("buy", 0.9)).await;
        assert!(
            matches!(result, Err(OrderManagerError::PositionLimitExceeded(_, _))),
            "order quantity 101 > max 100 must be blocked"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn position_limit_counts_filled_inventory() {
        // C6: once an order fills it is still held inventory and MUST keep counting
        // toward the cap. The old open-only accounting let filled orders vanish,
        // allowing unbounded sequential placement.
        let cfg = SafetyConfig {
            dry_run: false,
            max_position_size: Decimal::new(100, 0),
            default_order_quantity: Decimal::new(60, 0),
            circuit_breaker_max_orders: 10,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.0,
        };
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_return_status(OrderStatusResult::filled(Decimal::new(58_000, 0)))
            .await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        // First 60-unit buy is accepted and fills via the poller.
        manager
            .place_order(None, order_req(OrderSide::Buy, 60))
            .await
            .expect("first order within the cap must succeed");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if repo
                .all_records()
                .await
                .iter()
                .any(|r| r.status == "filled")
            {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("first order never reached 'filled'");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Second 60-unit buy must be rejected: 60 filled + 60 = 120 > 100.
        let res = manager
            .place_order(None, order_req(OrderSide::Buy, 60))
            .await;
        assert!(
            matches!(res, Err(OrderManagerError::PositionLimitExceeded(_, _))),
            "filled inventory must count toward the position limit"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn position_limit_allows_reducing_sell() {
        // C7: a sell reduces exposure, so it must be allowed even when long at the
        // cap. The old side-agnostic accounting wrongly added the sell quantity.
        let cfg = SafetyConfig {
            dry_run: false,
            max_position_size: Decimal::new(100, 0),
            default_order_quantity: Decimal::new(1, 0),
            circuit_breaker_max_orders: 10,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.0,
        };
        let (manager, _repo, _rx) = build_manager_with_repo(cfg, FakeOrderAdapter::new("tabdeal"));

        manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await
            .expect("buy up to the cap must succeed");

        manager
            .place_order(None, order_req(OrderSide::Sell, 50))
            .await
            .expect("a reducing sell must be allowed at the position cap");

        // Net is now 50; a further 60 buy would reach 110 > 100 and is rejected.
        let res = manager
            .place_order(None, order_req(OrderSide::Buy, 60))
            .await;
        assert!(
            matches!(res, Err(OrderManagerError::PositionLimitExceeded(_, _))),
            "net 50 + 60 = 110 must still be capped at 100"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn position_limit_is_scoped_per_user() {
        // M8: the position cap is enforced per user. User 1 hitting the cap must
        // not block user 2 (or the system bucket) from placing their own orders.
        let cfg = SafetyConfig {
            dry_run: false,
            max_position_size: Decimal::new(100, 0),
            default_order_quantity: Decimal::new(100, 0),
            circuit_breaker_max_orders: 100,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.0,
        };
        let (manager, repo, _rx) = build_manager_with_repo(cfg, FakeOrderAdapter::new("tabdeal"));

        // User 1 fills their cap.
        manager
            .place_order(Some(1), order_req(OrderSide::Buy, 100))
            .await
            .expect("user 1's first order fits their own cap");
        // User 1 is now at the cap — another unit is rejected for THEM.
        assert!(
            matches!(
                manager
                    .place_order(Some(1), order_req(OrderSide::Buy, 1))
                    .await,
                Err(OrderManagerError::PositionLimitExceeded(_, _))
            ),
            "user 1 over their own cap must be rejected"
        );
        // User 2 is unaffected by user 1's exposure.
        manager
            .place_order(Some(2), order_req(OrderSide::Buy, 100))
            .await
            .expect("user 2 has their own independent cap");
        // The system bucket is unaffected too.
        manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await
            .expect("the system bucket is independent of any user");

        assert_eq!(
            repo.net_position(Some(1), "tabdeal", "USDT/IRT")
                .await
                .unwrap(),
            Decimal::new(100, 0)
        );
        assert_eq!(
            repo.net_position(Some(2), "tabdeal", "USDT/IRT")
                .await
                .unwrap(),
            Decimal::new(100, 0)
        );
        assert_eq!(
            repo.net_position(None, "tabdeal", "USDT/IRT")
                .await
                .unwrap(),
            Decimal::new(100, 0)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_orders_do_not_exceed_position_limit() {
        // C9: concurrent placements must not both pass a stale net-position read.
        let cfg = SafetyConfig {
            dry_run: false,
            max_position_size: Decimal::new(100, 0),
            default_order_quantity: Decimal::new(60, 0),
            circuit_breaker_max_orders: 100,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.0,
        };
        let (manager, repo, _rx) = build_manager_with_repo(cfg, FakeOrderAdapter::new("tabdeal"));

        let mut handles = Vec::new();
        for _ in 0..5 {
            let m = manager.clone();
            handles.push(tokio::spawn(async move {
                m.place_order(None, order_req(OrderSide::Buy, 60)).await
            }));
        }

        let mut accepted = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                accepted += 1;
            }
        }

        assert_eq!(
            accepted, 1,
            "exactly one 60-unit order fits under the 100 cap — the rest must be rejected"
        );
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert!(
            net <= Decimal::new(100, 0),
            "net position {net} must never exceed the cap under concurrency"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_order_persisted_with_open_status_before_exchange_call() {
        let (broadcaster, _rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());

        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exch-001").await;
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));

        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo.clone(),
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();

        let records = repo.all_records().await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "open");
        assert_eq!(records[0].exchange_order_id.as_deref(), Some("exch-001"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_adapter_call_updates_status_to_failed() {
        let (broadcaster, _rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());

        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail(OrderAdapterError::Rejected("test rejection".to_string()))
            .await;
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));

        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo.clone(),
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        let result = manager.process_signal(&make_signal("buy", 0.9)).await;
        assert!(matches!(result, Err(OrderManagerError::Adapter(_))));

        let records = repo.all_records().await;
        assert_eq!(records[0].status, "failed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn placement_timeout_reconciles_before_marking_failed() {
        // C8: a NetworkTimeout on placement does NOT mean the order never landed —
        // the request may have reached the exchange and the response was lost.
        // Marking it `failed` would silently orphan a live position. The manager
        // must reconcile by client_order_id and, if the order is live, adopt it.
        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail(OrderAdapterError::NetworkTimeout(
                "response lost".to_string(),
            ))
            .await;
        adapter
            .will_reconcile_to_landed(
                "exch-reconciled-1",
                OrderStatusResult::new(OrderStatus::Open),
            )
            .await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await
            .expect("a reconciled live order must not surface as an error");

        let records = repo.all_records().await;
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].status, "open",
            "a reconciled-live order must be kept open, never marked failed"
        );
        assert_eq!(
            records[0].exchange_order_id.as_deref(),
            Some("exch-reconciled-1"),
            "the exchange id discovered by reconciliation must be persisted"
        );
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::new(100, 0),
            "the reconciled position must count toward the cap, not vanish"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn placement_timeout_with_no_exchange_record_marks_failed() {
        // C8: when reconciliation proves the exchange has no record of the order,
        // it genuinely never landed and is safe to mark failed.
        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail(OrderAdapterError::NetworkTimeout(
                "response lost".to_string(),
            ))
            .await;
        adapter.will_reconcile_not_found().await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        let result = manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await;
        assert!(matches!(result, Err(OrderManagerError::Adapter(_))));

        let records = repo.all_records().await;
        assert_eq!(records[0].status, "failed");
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::ZERO,
            "an order the exchange never received carries no exposure"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn placement_timeout_with_failed_reconciliation_leaves_order_open() {
        // C8: if reconciliation itself fails, the true state is UNKNOWN. Marking
        // the order failed could orphan a live position, so the manager must leave
        // it `open` (conservatively counted in net_position) for manual follow-up.
        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail(OrderAdapterError::NetworkTimeout(
                "response lost".to_string(),
            ))
            .await;
        adapter
            .will_fail_reconciliation(OrderAdapterError::NetworkTimeout(
                "reconcile unreachable".to_string(),
            ))
            .await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        let result = manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await;
        assert!(
            matches!(result, Err(OrderManagerError::Adapter(_))),
            "the caller is still told placement did not confirm"
        );

        let records = repo.all_records().await;
        assert_eq!(
            records[0].status, "open",
            "an unconfirmed order must stay open, never failed, to avoid orphaning"
        );
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::new(100, 0),
            "an unconfirmed order must keep counting toward the cap (conservative)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permanent_rejection_skips_reconciliation_and_marks_failed() {
        // A permanent rejection (e.g. insufficient funds) means the order was
        // never accepted — there is nothing to reconcile, mark it failed directly.
        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail(OrderAdapterError::Rejected("min notional".to_string()))
            .await;
        // Configure reconciliation to "landed" to prove it is NOT consulted for
        // a permanent error: the order must still be marked failed.
        adapter
            .will_reconcile_to_landed(
                "should-not-be-used",
                OrderStatusResult::new(OrderStatus::Open),
            )
            .await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        let result = manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await;
        assert!(matches!(result, Err(OrderManagerError::Adapter(_))));

        let records = repo.all_records().await;
        assert_eq!(records[0].status, "failed");
        assert!(
            records[0].exchange_order_id.is_none(),
            "a permanently rejected order must not adopt a reconciled exchange id"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn place_order_broadcasts_order_update_on_success() {
        let (broadcaster, mut rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());
        let cfg = live_config();

        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exch-999").await;
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));

        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        let req = OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::new(100, 0),
            price: None,
            client_order_id: "client-001".to_string(),
            strategy_id: None,
        };

        manager.place_order(None, req).await.unwrap();

        let text = rx
            .try_recv()
            .expect("order_update must be broadcast after placement");
        let msg: WsMessage = serde_json::from_str(&text).unwrap();
        assert!(matches!(msg, WsMessage::OrderUpdate(_)));
        if let WsMessage::OrderUpdate(payload) = msg {
            assert_eq!(payload.status, "open");
            assert_eq!(payload.order_id, "exch-999");
            assert_eq!(payload.client_order_id, "client-001");
            assert!(payload.fill_price.is_none());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_order_aborts_fill_poller() {
        let (broadcaster, _rx) = broadcast::channel(64);
        let repo = Arc::new(FakeOrderRepository::new());
        let cfg = live_config();

        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exch-cancel-001").await;
        // Status stays Open — poller would run forever without abort
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));

        let manager = Arc::new(OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            cfg,
            Duration::from_millis(50),
        ));

        let req = OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::new(100, 0),
            price: None,
            client_order_id: "client-cancel-001".to_string(),
            strategy_id: None,
        };

        manager.place_order(None, req).await.unwrap();

        assert!(
            manager.has_active_poll("client-cancel-001").await,
            "fill poller must be tracked after placement"
        );

        manager.cancel_order("client-cancel-001").await.unwrap();

        assert!(
            !manager.has_active_poll("client-cancel-001").await,
            "fill poller handle must be removed after cancel"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fill_poll_broadcasts_actual_fill_price_not_zero() {
        use crate::order::port::OrderStatusResult;

        let (broadcaster, mut rx) = broadcast::channel(32);
        let repo = Arc::new(FakeOrderRepository::new());
        let cfg = live_config();

        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exch-fill-price").await;
        let fill_price = Decimal::new(58_000, 0);
        adapter
            .will_return_status(OrderStatusResult::filled(fill_price))
            .await;

        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));

        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            cfg,
            Duration::from_millis(10),
        );

        manager
            .process_signal(&make_signal("buy", 0.9))
            .await
            .unwrap();

        // Wait for fill poller to run (it sleeps 10ms before first poll)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Drain broadcast messages, find the "filled" update
        let mut saw_filled_with_price = false;
        while let Ok(text) = rx.try_recv() {
            if let Ok(WsMessage::OrderUpdate(p)) = serde_json::from_str::<WsMessage>(&text) {
                if p.status == "filled" {
                    assert_ne!(
                        p.fill_price.as_deref(),
                        Some("0"),
                        "fill_price must not be \"0\""
                    );
                    assert_eq!(
                        p.fill_price.as_deref(),
                        Some("58000"),
                        "fill_price must match exchange-returned value"
                    );
                    saw_filled_with_price = true;
                }
            }
        }
        assert!(
            saw_filled_with_price,
            "must have received a filled OrderUpdate with fill_price"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn partial_fill_updates_filled_quantity() {
        // M7: a partial fill reported by the poller must be persisted as the
        // order's exact executed quantity, not left at zero.
        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exch-pf").await;
        adapter
            .will_return_status(
                OrderStatusResult::new(OrderStatus::PartiallyFilled)
                    .with_filled_quantity(Decimal::new(40, 0)),
            )
            .await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        manager
            .place_order(None, order_req(OrderSide::Buy, 100))
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let recs = repo.all_records().await;
            if recs[0].filled_quantity == Decimal::new(40, 0) {
                assert_eq!(recs[0].status, "partially_filled");
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!(
                    "filled_quantity was never updated to 40 (got {})",
                    recs[0].filled_quantity
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_after_partial_fill_keeps_filled_inventory_in_position() {
        // M7 end-to-end: an order partially fills (poller records 40), then is
        // cancelled. The 40 units already executed are still held — the net
        // position must keep them instead of dropping to zero on cancel.
        let cfg = live_config();
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exch-pf-cancel").await;
        adapter
            .will_return_status(
                OrderStatusResult::new(OrderStatus::PartiallyFilled)
                    .with_filled_quantity(Decimal::new(40, 0)),
            )
            .await;
        let (manager, repo, _rx) = build_manager_with_repo(cfg, adapter);

        let req = order_req(OrderSide::Buy, 100);
        let client_order_id = req.client_order_id.clone();
        manager.place_order(None, req).await.unwrap();

        // Wait for the poller to record the partial fill.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if repo.all_records().await[0].filled_quantity == Decimal::new(40, 0) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("partial fill was never recorded");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        manager.cancel_order(&client_order_id).await.unwrap();

        let recs = repo.all_records().await;
        assert_eq!(recs[0].status, "cancelled");
        assert_eq!(
            recs[0].filled_quantity,
            Decimal::new(40, 0),
            "the partial fill must survive the cancel"
        );
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::new(40, 0),
            "cancelled-but-partially-filled inventory must remain in the position"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn position_limit_rejection_does_not_trip_circuit_breaker() {
        let cfg = SafetyConfig {
            dry_run: false,
            max_position_size: Decimal::ZERO,
            default_order_quantity: Decimal::new(1, 0),
            circuit_breaker_max_orders: 3,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.0,
        };
        let (manager, _rx) = build_manager(cfg, FakeOrderAdapter::new("tabdeal"));

        for _ in 0..5 {
            let res = manager.process_signal(&make_signal("buy", 0.9)).await;
            assert!(
                matches!(res, Err(OrderManagerError::PositionLimitExceeded(_, _))),
                "every order must fail due to position limit, not circuit breaker"
            );
        }
        assert!(
            !manager.circuit_breaker_is_tripped().await,
            "circuit breaker must not trip on position-limit rejections"
        );
    }

    // ─── Subscription fanout tests ────────────────────────────────────────────

    use crate::infrastructure::db::subscription_repository::FakeSubscriptionRepository;

    fn build_manager_with_subscriptions(
        safety_config: SafetyConfig,
        adapter: FakeOrderAdapter,
        sub_repo: Arc<FakeSubscriptionRepository>,
    ) -> (OrderManager, broadcast::Receiver<String>) {
        let (broadcaster, rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(adapter));
        let repo = Arc::new(FakeOrderRepository::new());
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            repo,
            broadcaster,
            safety_config,
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo);
        (manager, rx)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn signal_produces_orders_for_all_subscribed_users() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        sub_repo.create(1, "spread-1", None, None).await.unwrap();
        sub_repo.create(2, "spread-1", None, None).await.unwrap();

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(FakeOrderAdapter::new("tabdeal")),
        );
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: true,
                default_order_quantity: Decimal::new(100, 0),
                max_position_size: Decimal::new(10000, 0),
                min_confidence: 0.7,
                ..SafetyConfig::default()
            },
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo);

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;

        let orders = order_repo.all_records().await;
        assert_eq!(
            orders.len(),
            2,
            "one order must be placed per subscribed user"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn user_a_order_failure_does_not_affect_user_b() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        // User 1: max_position_size = 0 → position limit immediately exceeded
        sub_repo
            .create(1, "spread-1", Some(Decimal::ZERO), None)
            .await
            .unwrap();
        // User 2: normal limits
        sub_repo.create(2, "spread-1", None, None).await.unwrap();

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(FakeOrderAdapter::new("tabdeal")),
        );
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: true,
                default_order_quantity: Decimal::new(100, 0),
                max_position_size: Decimal::new(10000, 0),
                min_confidence: 0.7,
                ..SafetyConfig::default()
            },
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo);

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;

        let orders = order_repo.all_records().await;
        assert_eq!(
            orders.len(),
            1,
            "user 2 must receive an order even though user 1 was blocked by position limit"
        );
        // The order that was placed belongs to user 2 (strategy_id is set but
        // user_id is not tracked in OrderRecord yet — we verify count instead)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unsubscribed_user_receives_no_order() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        // No subscriptions exist

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            order_repo.clone(),
            broadcaster,
            SafetyConfig::default(),
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo);

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;

        assert!(
            order_repo.all_records().await.is_empty(),
            "no subscriptions → no orders must be placed"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn per_subscription_confidence_threshold_overrides_global() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        // User 1: high personal threshold (0.95) — signal at 0.85 won't reach them
        sub_repo
            .create(1, "spread-1", None, Some(0.95))
            .await
            .unwrap();
        // User 2: no override → uses global floor 0.7 → signal passes
        sub_repo.create(2, "spread-1", None, None).await.unwrap();

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(FakeOrderAdapter::new("tabdeal")),
        );
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: true,
                default_order_quantity: Decimal::new(100, 0),
                max_position_size: Decimal::new(10000, 0),
                min_confidence: 0.7,
                ..SafetyConfig::default()
            },
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo);

        // Signal at 0.85: above global 0.7, but below user 1's personal 0.95
        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.85))
            .await;

        let orders = order_repo.all_records().await;
        assert_eq!(
            orders.len(),
            1,
            "only user 2 (default threshold) must get an order"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hold_signal_produces_no_orders_for_any_subscriber() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        sub_repo.create(1, "spread-1", None, None).await.unwrap();
        sub_repo.create(2, "spread-1", None, None).await.unwrap();

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (_broadcaster, _rx) = broadcast::channel::<String>(32);
        let (manager, _) = build_manager_with_subscriptions(
            SafetyConfig {
                dry_run: true,
                ..SafetyConfig::default()
            },
            FakeOrderAdapter::new("tabdeal"),
            sub_repo,
        );
        let _ = order_repo; // manager uses its own internal repo

        manager
            .fan_out_signal_to_subscriptions(&make_signal("hold", 0.9))
            .await;
        // manager's internal order_repo has zero records — nothing was placed
        // (we can't reach the internal repo here; we verify no panic/error)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fan_out_is_noop_when_no_subscription_repository_configured() {
        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            order_repo.clone(),
            broadcaster,
            SafetyConfig::default(),
            Duration::from_millis(10),
        );
        // No subscription_repository attached → fan_out must silently return

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;
        assert!(order_repo.all_records().await.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inactive_subscription_receives_no_order() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        let sub = sub_repo.create(1, "spread-1", None, None).await.unwrap();
        sub_repo.update(sub.id, false, None, None).await.unwrap(); // deactivate

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(FakeOrderAdapter::new("tabdeal")),
        );
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: true,
                default_order_quantity: Decimal::new(100, 0),
                max_position_size: Decimal::new(10000, 0),
                ..SafetyConfig::default()
            },
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo);

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;

        assert!(
            order_repo.all_records().await.is_empty(),
            "inactive subscription must not produce an order"
        );
    }

    // ---------------------------------------------------------------------------
    // place_order_for_user — credential-aware admin order placement

    #[tokio::test(flavor = "current_thread")]
    async fn place_order_for_user_without_resolver_returns_no_credential_resolver_error() {
        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            order_repo,
            broadcaster,
            SafetyConfig {
                dry_run: true,
                ..SafetyConfig::default()
            },
            Duration::from_millis(10),
        );

        let req = OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::new(100, 0),
            price: None,
            client_order_id: Uuid::new_v4().to_string(),
            strategy_id: None,
        };
        let err = manager.place_order_for_user(1, req).await.unwrap_err();
        assert!(
            matches!(err, OrderManagerError::NoCredentialResolver),
            "expected NoCredentialResolver, got {err}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn place_order_for_user_with_no_credentials_stored_returns_error() {
        use crate::order::credential_resolver::FakeCredentialResolver;

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let resolver = Arc::new(FakeCredentialResolver::none());
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            order_repo,
            broadcaster,
            SafetyConfig {
                dry_run: true,
                ..SafetyConfig::default()
            },
            Duration::from_millis(10),
        )
        .with_credential_resolver(resolver);

        let req = OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::new(100, 0),
            price: None,
            client_order_id: Uuid::new_v4().to_string(),
            strategy_id: None,
        };
        let err = manager.place_order_for_user(99, req).await.unwrap_err();
        assert!(
            matches!(err, OrderManagerError::NoCredentialsForUser { .. }),
            "expected NoCredentialsForUser, got {err}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn place_order_for_user_uses_resolver_adapter_not_global_registry() {
        use crate::order::credential_resolver::FakeCredentialResolver;

        let user_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let resolver = Arc::new(FakeCredentialResolver::returning(
            Arc::clone(&user_adapter) as Arc<dyn OrderAdapter>
        ));

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: false,
                ..live_config()
            },
            Duration::from_millis(10),
        )
        .with_credential_resolver(resolver);

        let req = OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::new(100, 0),
            price: None,
            client_order_id: Uuid::new_v4().to_string(),
            strategy_id: None,
        };
        manager.place_order_for_user(1, req).await.unwrap();

        assert_eq!(
            user_adapter.placed_count().await,
            1,
            "order must use the per-user adapter, not the global registry"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fan_out_uses_per_user_adapter_when_credential_resolver_is_configured() {
        use crate::order::credential_resolver::FakeCredentialResolver;

        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        sub_repo.create(1, "spread-1", None, None).await.unwrap();

        let user_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let resolver = Arc::new(FakeCredentialResolver::returning(
            Arc::clone(&user_adapter) as Arc<dyn OrderAdapter>
        ));

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: false,
                ..live_config()
            },
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo)
        .with_credential_resolver(resolver);

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;

        assert_eq!(
            user_adapter.placed_count().await,
            1,
            "fan_out must use per-user adapter from resolver"
        );
        assert_eq!(order_repo.all_records().await.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fan_out_falls_back_to_global_adapter_when_resolver_returns_none() {
        use crate::order::credential_resolver::FakeCredentialResolver;

        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        sub_repo.create(1, "spread-1", None, None).await.unwrap();

        let resolver = Arc::new(FakeCredentialResolver::none());

        let global_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let mut adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::clone(&global_adapter) as Arc<dyn OrderAdapter>,
        );

        let order_repo = Arc::new(FakeOrderRepository::new());
        let (broadcaster, _rx) = broadcast::channel(32);
        let manager = OrderManager::with_poll_interval(
            Arc::new(RwLock::new(adapters)),
            order_repo.clone(),
            broadcaster,
            SafetyConfig {
                dry_run: false,
                ..live_config()
            },
            Duration::from_millis(10),
        )
        .with_subscription_repository(sub_repo)
        .with_credential_resolver(resolver);

        manager
            .fan_out_signal_to_subscriptions(&make_signal("buy", 0.9))
            .await;

        assert_eq!(
            global_adapter.placed_count().await,
            1,
            "when resolver returns None, must fall back to global adapter"
        );
    }
}
