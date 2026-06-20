use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::exchange::entity::ExchangeId;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderStatus, OrderStatusResult,
};

/// Configures how `FakeOrderAdapter` responds to place/cancel/status calls.
pub enum FakeResponse {
    /// Return `Ok(OrderId(id))`.
    Success(String),
    /// Return `Err(err)`.
    Failure(OrderAdapterError),
}

pub struct FakeOrderAdapter {
    exchange: ExchangeId,
    /// Orders recorded by `place_order`, in call order.
    pub placed_orders: Arc<Mutex<Vec<OrderRequest>>>,
    /// Canned response for `place_order`. Defaults to `Ok(OrderId("fake-order-id"))`.
    place_response: Arc<Mutex<Option<FakeResponse>>>,
    /// Canned result returned by `get_order_status`. Defaults to `Open` with no fill price.
    status_response: Arc<Mutex<OrderStatusResult>>,
}

impl FakeOrderAdapter {
    pub fn new(exchange: &str) -> Self {
        Self {
            exchange: ExchangeId::new(exchange),
            placed_orders: Arc::new(Mutex::new(vec![])),
            place_response: Arc::new(Mutex::new(None)),
            status_response: Arc::new(Mutex::new(OrderStatusResult::new(OrderStatus::Open))),
        }
    }

    /// Pre-configure the result returned by `get_order_status`.
    pub async fn will_return_status(&self, result: OrderStatusResult) {
        *self.status_response.lock().await = result;
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
}
