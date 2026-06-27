use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::exchange::entity::ExchangeId;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderStatus, OrderStatusResult,
    ReconciledOrder,
};

/// Configures how `FakeOrderAdapter` responds to place/cancel/status calls.
pub enum FakeResponse {
    /// Return `Ok(OrderId(id))`.
    Success(String),
    /// Return `Err(err)`.
    Failure(OrderAdapterError),
}

/// Configures how `FakeOrderAdapter::get_order_status_by_client_id` responds.
pub enum FakeReconcile {
    /// The order landed at the exchange: `Ok(Some(..))`.
    Landed(OrderId, OrderStatusResult),
    /// The exchange has no record of the order: `Ok(None)`.
    NotFound,
    /// Reconciliation itself failed: `Err(..)`.
    Failed(OrderAdapterError),
}

pub struct FakeOrderAdapter {
    exchange: ExchangeId,
    /// Orders recorded by `place_order`, in call order.
    pub placed_orders: Arc<Mutex<Vec<OrderRequest>>>,
    /// Canned response for `place_order`. Defaults to `Ok(OrderId("fake-order-id"))`.
    place_response: Arc<Mutex<Option<FakeResponse>>>,
    /// Canned result returned by `get_order_status`. Defaults to `Open` with no fill price.
    status_response: Arc<Mutex<OrderStatusResult>>,
    /// Canned outcome for `get_order_status_by_client_id`. `None` (the default)
    /// behaves as `NotFound` — the exchange has no record of the order.
    reconcile_response: Arc<Mutex<Option<FakeReconcile>>>,
}

impl FakeOrderAdapter {
    pub fn new(exchange: &str) -> Self {
        Self {
            exchange: ExchangeId::new(exchange),
            placed_orders: Arc::new(Mutex::new(vec![])),
            place_response: Arc::new(Mutex::new(None)),
            status_response: Arc::new(Mutex::new(OrderStatusResult::new(OrderStatus::Open))),
            reconcile_response: Arc::new(Mutex::new(None)),
        }
    }

    /// Pre-configure the result returned by `get_order_status`.
    pub async fn will_return_status(&self, result: OrderStatusResult) {
        *self.status_response.lock().await = result;
    }

    /// Pre-configure reconciliation to report the order as live at the exchange.
    pub async fn will_reconcile_to_landed(&self, order_id: &str, result: OrderStatusResult) {
        *self.reconcile_response.lock().await =
            Some(FakeReconcile::Landed(OrderId(order_id.to_string()), result));
    }

    /// Pre-configure reconciliation to report the exchange has no record (never landed).
    pub async fn will_reconcile_not_found(&self) {
        *self.reconcile_response.lock().await = Some(FakeReconcile::NotFound);
    }

    /// Pre-configure reconciliation itself to fail (true state unknown).
    pub async fn will_fail_reconciliation(&self, err: OrderAdapterError) {
        *self.reconcile_response.lock().await = Some(FakeReconcile::Failed(err));
    }

    /// Pre-configure the adapter to return an error on the next `place_order` call.
    pub async fn will_fail(&self, err: OrderAdapterError) {
        *self.place_response.lock().await = Some(FakeResponse::Failure(err));
    }

    /// Pre-configure the adapter to return a specific order id on `place_order`.
    pub async fn will_succeed_with(&self, order_id: &str) {
        *self.place_response.lock().await = Some(FakeResponse::Success(order_id.to_string()));
    }

    pub async fn placed_count(&self) -> usize {
        self.placed_orders.lock().await.len()
    }
}

#[async_trait]
impl OrderAdapter for FakeOrderAdapter {
    fn exchange_id(&self) -> ExchangeId {
        self.exchange.clone()
    }

    async fn place_order(&self, req: &OrderRequest) -> Result<OrderId, OrderAdapterError> {
        self.placed_orders.lock().await.push(req.clone());

        let mut guard = self.place_response.lock().await;
        match guard.take() {
            Some(FakeResponse::Failure(e)) => Err(e),
            Some(FakeResponse::Success(id)) => Ok(OrderId(id)),
            None => Ok(OrderId("fake-order-id".to_string())),
        }
    }

    async fn cancel_order(&self, _order_id: &OrderId) -> Result<(), OrderAdapterError> {
        Ok(())
    }

    async fn get_order_status(
        &self,
        _order_id: &OrderId,
    ) -> Result<OrderStatusResult, OrderAdapterError> {
        Ok(self.status_response.lock().await.clone())
    }

    async fn get_order_status_by_client_id(
        &self,
        _client_order_id: &str,
    ) -> Result<Option<ReconciledOrder>, OrderAdapterError> {
        match self.reconcile_response.lock().await.take() {
            Some(FakeReconcile::Landed(order_id, result)) => {
                Ok(Some(ReconciledOrder { order_id, result }))
            }
            Some(FakeReconcile::Failed(e)) => Err(e),
            Some(FakeReconcile::NotFound) | None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn sample_request() -> OrderRequest {
        use crate::order::port::{OrderSide, OrderType};
        OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::new(100, 0),
            price: Some(Decimal::new(58_000, 0)),
            client_order_id: "uuid-1234".to_string(),
            strategy_id: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_records_placed_orders() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        let req = sample_request();

        adapter.place_order(&req).await.unwrap();

        assert_eq!(adapter.placed_count().await, 1);
        let orders = adapter.placed_orders.lock().await;
        assert_eq!(orders[0].client_order_id, "uuid-1234");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_returns_default_order_id_on_success() {
        let adapter = FakeOrderAdapter::new("tabdeal");

        let result = adapter.place_order(&sample_request()).await.unwrap();

        assert_eq!(result.to_string(), "fake-order-id");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_returns_configured_order_id() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter.will_succeed_with("exchange-99").await;

        let result = adapter.place_order(&sample_request()).await.unwrap();

        assert_eq!(result.to_string(), "exchange-99");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_returns_configured_error() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail(OrderAdapterError::InsufficientFunds)
            .await;

        let result = adapter.place_order(&sample_request()).await;

        assert!(matches!(result, Err(OrderAdapterError::InsufficientFunds)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_exchange_id_matches_constructor() {
        let adapter = FakeOrderAdapter::new("hitobit");
        assert_eq!(adapter.exchange_id().to_string(), "hitobit");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_cancel_order_returns_ok() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        let result = adapter.cancel_order(&OrderId("12345".to_string())).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_get_order_status_returns_open_by_default() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        let result = adapter
            .get_order_status(&OrderId("12345".to_string()))
            .await
            .unwrap();
        assert_eq!(result.status, OrderStatus::Open);
        assert!(result.fill_price.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_get_order_status_returns_configured_fill_price() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        let price = rust_decimal::Decimal::new(58_000, 0);
        adapter
            .will_return_status(OrderStatusResult::filled(price))
            .await;

        let result = adapter
            .get_order_status(&OrderId("12345".to_string()))
            .await
            .unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
        assert_eq!(result.fill_price, Some(price));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_reconcile_defaults_to_not_found() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        let result = adapter
            .get_order_status_by_client_id("uuid-1234")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "an unconfigured reconciliation must report no record (never landed)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_reconcile_landed_returns_exchange_id_and_status() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_reconcile_to_landed("exch-55", OrderStatusResult::new(OrderStatus::Open))
            .await;

        let result = adapter
            .get_order_status_by_client_id("uuid-1234")
            .await
            .unwrap()
            .expect("must report the order as landed");
        assert_eq!(result.order_id.0, "exch-55");
        assert_eq!(result.result.status, OrderStatus::Open);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_adapter_reconcile_can_fail() {
        let adapter = FakeOrderAdapter::new("tabdeal");
        adapter
            .will_fail_reconciliation(OrderAdapterError::NetworkTimeout("down".to_string()))
            .await;

        let result = adapter.get_order_status_by_client_id("uuid-1234").await;
        assert!(matches!(result, Err(OrderAdapterError::NetworkTimeout(_))));
    }
}
