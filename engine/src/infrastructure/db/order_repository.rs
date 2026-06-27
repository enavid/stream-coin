use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct OrderRecord {
    pub id: Option<i64>,
    /// Owning user. `None` for a system / signal-driven order (no owning user);
    /// `Some(id)` scopes the order to one user for per-user position limits (M8).
    pub user_id: Option<i32>,
    pub exchange: String,
    pub pair: String,
    pub side: String,
    pub order_type: String,
    pub quantity: Decimal,
    /// Cumulative base-currency quantity that has actually executed. `ZERO` for a
    /// freshly placed order; updated by the fill poller as (partial) fills arrive.
    /// Retained when an order ends so a partial-fill-then-cancel keeps its real
    /// residual inventory in the position accounting.
    pub filled_quantity: Decimal,
    pub price: Option<Decimal>,
    pub status: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: String,
    pub strategy_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum OrderRepositoryError {
    #[error("database error: {0}")]
    Database(String),
    #[error("order not found: client_order_id={0}")]
    NotFound(String),
}

#[async_trait]
pub trait OrderRepository: Send + Sync {
    /// Persists a new order record. Returns the assigned row `id`.
    async fn insert(&self, record: &OrderRecord) -> Result<i64, OrderRepositoryError>;

    /// Updates the status (and optional exchange_order_id / fill_price /
    /// filled_quantity) for an order identified by `client_order_id`.
    ///
    /// `filled_quantity`: `Some(q)` overwrites the cumulative executed quantity;
    /// `None` leaves the stored value untouched — so cancelling an order preserves
    /// whatever had already filled before the cancel.
    async fn update_status(
        &self,
        client_order_id: &str,
        status: &str,
        exchange_order_id: Option<&str>,
        fill_price: Option<Decimal>,
        filled_quantity: Option<Decimal>,
    ) -> Result<(), OrderRepositoryError>;

    /// Returns the signed **net position** for one user bucket + exchange + pair:
    /// the sum of buy quantities minus sell quantities over every order that
    /// carries real or pending exposure. A positive value is net long, a negative
    /// value net short.
    ///
    /// `user_id` selects the bucket (M8): `None` sums only system / signal-driven
    /// orders (rows with no owning user); `Some(id)` sums only that user's orders,
    /// so one user's exposure never counts against another's position limit.
    ///
    /// Each order contributes (M7):
    /// - `open` / `filled` / `partially_filled` → full `quantity` (the whole order
    ///   is committed: the unfilled remainder can still execute, so the cap stays
    ///   conservative).
    /// - `cancelled` / `failed` → `filled_quantity` (the order is over, but whatever
    ///   actually executed before it ended is still held inventory — counting this
    ///   as zero would silently lose a partial-fill-then-cancel position).
    /// - `dry_run` → nothing (no real exposure).
    async fn net_position(
        &self,
        user_id: Option<i32>,
        exchange: &str,
        pair: &str,
    ) -> Result<Decimal, OrderRepositoryError>;

    /// Fetches a single order by its client-assigned idempotency key.
    async fn get_by_client_order_id(
        &self,
        client_order_id: &str,
    ) -> Result<OrderRecord, OrderRepositoryError>;

    /// Lists orders, optionally filtered by exchange and/or pair.
    async fn list(
        &self,
        exchange: Option<&str>,
        pair: Option<&str>,
    ) -> Result<Vec<OrderRecord>, OrderRepositoryError>;
}

// ---------------------------------------------------------------------------
// In-memory fake — used in unit and integration tests

pub struct FakeOrderRepository {
    records: Mutex<Vec<OrderRecord>>,
}

impl Default for FakeOrderRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeOrderRepository {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(vec![]),
        }
    }

    pub async fn all_records(&self) -> Vec<OrderRecord> {
        self.records.lock().await.clone()
    }
}

#[async_trait]
impl OrderRepository for FakeOrderRepository {
    async fn insert(&self, record: &OrderRecord) -> Result<i64, OrderRepositoryError> {
        let mut recs = self.records.lock().await;
        let id = (recs.len() + 1) as i64;
        let mut r = record.clone();
        r.id = Some(id);
        recs.push(r);
        Ok(id)
    }

    async fn update_status(
        &self,
        client_order_id: &str,
        status: &str,
        exchange_order_id: Option<&str>,
        fill_price: Option<Decimal>,
        filled_quantity: Option<Decimal>,
    ) -> Result<(), OrderRepositoryError> {
        let _ = fill_price;
        let mut recs = self.records.lock().await;
        let record = recs
            .iter_mut()
            .find(|r| r.client_order_id == client_order_id)
            .ok_or_else(|| OrderRepositoryError::NotFound(client_order_id.to_string()))?;
        record.status = status.to_string();
        if let Some(eid) = exchange_order_id {
            record.exchange_order_id = Some(eid.to_string());
        }
        if let Some(q) = filled_quantity {
            record.filled_quantity = q;
        }
        record.updated_at = Utc::now();
        Ok(())
    }

    async fn net_position(
        &self,
        user_id: Option<i32>,
        exchange: &str,
        pair: &str,
    ) -> Result<Decimal, OrderRepositoryError> {
        let recs = self.records.lock().await;
        let net = recs
            .iter()
            .filter(|r| r.user_id == user_id && r.exchange == exchange && r.pair == pair)
            .fold(Decimal::ZERO, |acc, r| {
                let exposure = match r.status.as_str() {
                    "open" | "filled" | "partially_filled" => r.quantity,
                    "cancelled" | "failed" => r.filled_quantity,
                    _ => Decimal::ZERO,
                };
                match r.side.as_str() {
                    "sell" => acc - exposure,
                    _ => acc + exposure,
                }
            });
        Ok(net)
    }

    async fn get_by_client_order_id(
        &self,
        client_order_id: &str,
    ) -> Result<OrderRecord, OrderRepositoryError> {
        self.records
            .lock()
            .await
            .iter()
            .find(|r| r.client_order_id == client_order_id)
            .cloned()
            .ok_or_else(|| OrderRepositoryError::NotFound(client_order_id.to_string()))
    }

    async fn list(
        &self,
        exchange: Option<&str>,
        pair: Option<&str>,
    ) -> Result<Vec<OrderRecord>, OrderRepositoryError> {
        let recs = self.records.lock().await;
        Ok(recs
            .iter()
            .filter(|r| {
                exchange.is_none_or(|e| r.exchange == e) && pair.is_none_or(|p| r.pair == p)
            })
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn make_record(client_order_id: &str, status: &str, quantity: Decimal) -> OrderRecord {
        make_record_side(client_order_id, status, quantity, "buy")
    }

    fn make_record_side(
        client_order_id: &str,
        status: &str,
        quantity: Decimal,
        side: &str,
    ) -> OrderRecord {
        let now = Utc::now();
        OrderRecord {
            id: None,
            user_id: None,
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: side.to_string(),
            order_type: "market".to_string(),
            quantity,
            filled_quantity: Decimal::ZERO,
            price: None,
            status: status.to_string(),
            exchange_order_id: None,
            client_order_id: client_order_id.to_string(),
            strategy_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_insert_assigns_sequential_ids() {
        let repo = FakeOrderRepository::new();
        let id1 = repo
            .insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let id2 = repo
            .insert(&make_record("uuid-2", "open", Decimal::new(200, 0)))
            .await
            .unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_update_status_changes_status() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.update_status("uuid-1", "filled", Some("exch-001"), None, None)
            .await
            .unwrap();
        let recs = repo.all_records().await;
        assert_eq!(recs[0].status, "filled");
        assert_eq!(recs[0].exchange_order_id.as_deref(), Some("exch-001"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_update_status_unknown_id_returns_not_found() {
        let repo = FakeOrderRepository::new();
        let result = repo
            .update_status("nonexistent", "filled", None, None, None)
            .await;
        assert!(matches!(result, Err(OrderRepositoryError::NotFound(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_counts_filled_inventory_not_just_open() {
        // C6: a filled order is held inventory and MUST count toward the position.
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-2", "filled", Decimal::new(50, 0)))
            .await
            .unwrap();
        repo.insert(&make_record(
            "uuid-3",
            "partially_filled",
            Decimal::new(20, 0),
        ))
        .await
        .unwrap();
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::new(170, 0),
            "open + filled + partially_filled buys all count (100+50+20)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_is_side_aware_buys_minus_sells() {
        // C7: sells reduce the net position; they must not be added to it.
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record_side(
            "uuid-1",
            "filled",
            Decimal::new(100, 0),
            "buy",
        ))
        .await
        .unwrap();
        repo.insert(&make_record_side(
            "uuid-2",
            "filled",
            Decimal::new(30, 0),
            "sell",
        ))
        .await
        .unwrap();
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(net, Decimal::new(70, 0), "100 buy - 30 sell = 70 net");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_excludes_terminal_and_dry_run() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "cancelled", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-2", "failed", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-3", "dry_run", Decimal::new(100, 0)))
            .await
            .unwrap();
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::ZERO,
            "cancelled/failed/dry_run carry no real exposure"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_update_status_sets_filled_quantity_when_some() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.update_status(
            "uuid-1",
            "partially_filled",
            None,
            None,
            Some(Decimal::new(40, 0)),
        )
        .await
        .unwrap();
        let recs = repo.all_records().await;
        assert_eq!(recs[0].filled_quantity, Decimal::new(40, 0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_update_status_none_filled_quantity_preserves_existing() {
        // A cancel passes None for filled_quantity — the partial fill recorded by
        // the poller before the cancel must survive.
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.update_status(
            "uuid-1",
            "partially_filled",
            None,
            None,
            Some(Decimal::new(40, 0)),
        )
        .await
        .unwrap();
        repo.update_status("uuid-1", "cancelled", None, None, None)
            .await
            .unwrap();
        let recs = repo.all_records().await;
        assert_eq!(recs[0].status, "cancelled");
        assert_eq!(
            recs[0].filled_quantity,
            Decimal::new(40, 0),
            "cancelling must not erase the quantity that already filled"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_counts_filled_inventory_of_cancelled_order() {
        // M7: the headline money-safety bug. A buy that partially fills (40) and is
        // then cancelled still leaves 40 units held — net_position must reflect
        // that, not drop it to zero just because the order is `cancelled`.
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.update_status(
            "uuid-1",
            "partially_filled",
            None,
            None,
            Some(Decimal::new(40, 0)),
        )
        .await
        .unwrap();
        repo.update_status("uuid-1", "cancelled", None, None, None)
            .await
            .unwrap();
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::new(40, 0),
            "partial-fill-then-cancel inventory must remain in the net position"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_cancelled_with_no_fill_counts_zero() {
        // Regression guard: a cancelled order that never filled carries no exposure.
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "cancelled", Decimal::new(100, 0)))
            .await
            .unwrap();
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(net, Decimal::ZERO);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_counts_partially_filled_at_full_quantity() {
        // While still working, a partially-filled order counts at FULL quantity:
        // the unfilled remainder can still execute, so the cap stays conservative.
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.update_status(
            "uuid-1",
            "partially_filled",
            None,
            None,
            Some(Decimal::new(40, 0)),
        )
        .await
        .unwrap();
        let net = repo
            .net_position(None, "tabdeal", "USDT/IRT")
            .await
            .unwrap();
        assert_eq!(
            net,
            Decimal::new(100, 0),
            "an active partial fill is still committed to its full quantity"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_is_scoped_per_user() {
        // M8: one user's exposure must never count against another's limit, and a
        // system order (user_id None) is its own bucket separate from both.
        let repo = FakeOrderRepository::new();
        let mut u1 = make_record("u1-buy", "filled", Decimal::new(100, 0));
        u1.user_id = Some(1);
        let mut u2 = make_record("u2-buy", "filled", Decimal::new(30, 0));
        u2.user_id = Some(2);
        let sys = make_record("sys-buy", "filled", Decimal::new(7, 0)); // user_id None
        repo.insert(&u1).await.unwrap();
        repo.insert(&u2).await.unwrap();
        repo.insert(&sys).await.unwrap();

        assert_eq!(
            repo.net_position(Some(1), "tabdeal", "USDT/IRT")
                .await
                .unwrap(),
            Decimal::new(100, 0),
            "user 1 sees only their own 100"
        );
        assert_eq!(
            repo.net_position(Some(2), "tabdeal", "USDT/IRT")
                .await
                .unwrap(),
            Decimal::new(30, 0),
            "user 2 sees only their own 30"
        );
        assert_eq!(
            repo.net_position(None, "tabdeal", "USDT/IRT")
                .await
                .unwrap(),
            Decimal::new(7, 0),
            "the system bucket sees only the user-less order"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_net_position_filters_by_pair() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let net = repo.net_position(None, "tabdeal", "BTC/IRT").await.unwrap();
        assert_eq!(net, Decimal::ZERO, "different pair must not count");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_list_returns_all_without_filter() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-2", "filled", Decimal::new(200, 0)))
            .await
            .unwrap();
        let all = repo.list(None, None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_get_by_client_order_id_returns_record() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-target", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let rec = repo.get_by_client_order_id("uuid-target").await.unwrap();
        assert_eq!(rec.client_order_id, "uuid-target");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_get_by_client_order_id_returns_not_found() {
        let repo = FakeOrderRepository::new();
        let result = repo.get_by_client_order_id("nonexistent").await;
        assert!(matches!(result, Err(OrderRepositoryError::NotFound(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_list_filters_by_exchange() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let results = repo.list(Some("hitobit"), None).await.unwrap();
        assert!(
            results.is_empty(),
            "different exchange must be filtered out"
        );
    }
}
