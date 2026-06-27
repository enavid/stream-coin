use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::exchange::entity::ExchangeId;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderSide, OrderStatus,
    OrderStatusResult, OrderType, ReconciledOrder,
};

const PLACE_ORDER_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_ORDER_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// REST order adapter for Hitobit.
///
/// Hitobit uses a Binance-compatible REST API. Symbols are uppercase-concatenated
/// (e.g. "USDTIRT"), sides are "BUY"/"SELL", types are "MARKET"/"LIMIT".
/// Limit orders require `timeInForce` = "GTC".
pub struct HitobitOrderAdapter {
    exchange_id: ExchangeId,
    base_url: String,
    api_key: String,
    http_client: reqwest::Client,
}

impl HitobitOrderAdapter {
    pub fn new(api_key: &str) -> Self {
        Self::with_base_url("https://api.hitobit.com", api_key)
    }

    pub fn with_base_url(base_url: &str, api_key: &str) -> Self {
        Self {
            exchange_id: ExchangeId::new("hitobit"),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            http_client: reqwest::Client::new(),
        }
    }

    /// Converts `"USDT/IRT"` → `"USDTIRT"` — Hitobit's Binance-compatible format.
    fn symbol(pair: &str) -> String {
        pair.replace('/', "").to_uppercase()
    }

    /// Builds the JSON body for a place-order POST request.
    pub fn build_place_order_body(req: &OrderRequest) -> Value {
        let mut body = json!({
            "symbol": Self::symbol(&req.pair),
            "side": match req.side {
                OrderSide::Buy => "BUY",
                OrderSide::Sell => "SELL",
            },
            "type": match req.order_type {
                OrderType::Limit => "LIMIT",
                OrderType::Market => "MARKET",
            },
            "quantity": req.quantity.to_string(),
            "newClientOrderId": req.client_order_id,
        });

        if req.order_type == OrderType::Limit {
            if let Some(price) = &req.price {
                body["price"] = json!(price.to_string());
                body["timeInForce"] = json!("GTC");
            }
        }

        body
    }

    /// Parses the HTTP response from a place-order call.
    pub fn parse_place_order_response(
        status: u16,
        body: &str,
    ) -> Result<OrderId, OrderAdapterError> {
        match status {
            200 | 201 => {
                let v: Value = serde_json::from_str(body).map_err(|e| {
                    OrderAdapterError::Serialization(format!("invalid place-order response: {e}"))
                })?;
                let order_id = if let Some(n) = v["orderId"].as_u64() {
                    n.to_string()
                } else if let Some(s) = v["orderId"].as_str() {
                    s.to_string()
                } else {
                    return Err(OrderAdapterError::Serialization(
                        "place-order response missing orderId".to_string(),
                    ));
                };
                tracing::info!(
                    order_id = %order_id,
                    exchange = "hitobit",
                    "order placed successfully"
                );
                Ok(OrderId(order_id))
            }
            400 | 422 => {
                let msg = serde_json::from_str::<Value>(body)
                    .ok()
                    .and_then(|v| {
                        v["msg"]
                            .as_str()
                            .or_else(|| v["message"].as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| body.to_string());
                Err(OrderAdapterError::Rejected(msg))
            }
            401 | 403 => Err(OrderAdapterError::AuthFailed),
            _ if status >= 500 => Err(OrderAdapterError::ServerError {
                status,
                body: body.to_string(),
            }),
            _ => Err(OrderAdapterError::Rejected(format!(
                "unexpected status {status}: {body}"
            ))),
        }
    }

    /// Parses the HTTP response from a cancel-order call.
    pub fn parse_cancel_order_response(status: u16, body: &str) -> Result<(), OrderAdapterError> {
        match status {
            200 | 204 => Ok(()),
            400 | 422 => {
                let msg = serde_json::from_str::<Value>(body)
                    .ok()
                    .and_then(|v| v["msg"].as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| body.to_string());
                Err(OrderAdapterError::Rejected(msg))
            }
            401 | 403 => Err(OrderAdapterError::AuthFailed),
            _ if status >= 500 => Err(OrderAdapterError::ServerError {
                status,
                body: body.to_string(),
            }),
            _ => Err(OrderAdapterError::Rejected(format!(
                "unexpected status {status}"
            ))),
        }
    }

    /// Parses the HTTP response from a get-order-status call.
    pub fn parse_order_status_response(
        status: u16,
        body: &str,
    ) -> Result<OrderStatusResult, OrderAdapterError> {
        match status {
            200 => {
                let v: Value = serde_json::from_str(body).map_err(|e| {
                    OrderAdapterError::Serialization(format!("invalid status response: {e}"))
                })?;
                let raw = v["status"].as_str().unwrap_or("").to_uppercase();
                let order_status = match raw.as_str() {
                    "NEW" | "OPEN" => OrderStatus::Open,
                    "FILLED" => OrderStatus::Filled,
                    "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
                    "CANCELED" | "CANCELLED" => OrderStatus::Cancelled,
                    _ => OrderStatus::Failed,
                };
                let fill_price = v["avgPrice"]
                    .as_str()
                    .or_else(|| v["price"].as_str())
                    .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
                    .filter(|p| *p > rust_decimal::Decimal::ZERO);
                Ok(OrderStatusResult {
                    status: order_status,
                    fill_price,
                })
            }
            401 | 403 => Err(OrderAdapterError::AuthFailed),
            _ if status >= 500 => Err(OrderAdapterError::ServerError {
                status,
                body: body.to_string(),
            }),
            _ => Err(OrderAdapterError::Rejected(format!(
                "unexpected status {status}"
            ))),
        }
    }

    /// Parses a reconciliation response (`GET ...?origClientOrderId=`).
    /// `Ok(None)` means the exchange has no record of the order (it never
    /// landed); `Err(_)` means the lookup was inconclusive (state still unknown).
    pub fn parse_reconcile_response(
        status: u16,
        body: &str,
    ) -> Result<Option<ReconciledOrder>, OrderAdapterError> {
        match status {
            200 => {
                let v: Value = serde_json::from_str(body).map_err(|e| {
                    OrderAdapterError::Serialization(format!("invalid reconcile response: {e}"))
                })?;
                let order_id = if let Some(n) = v["orderId"].as_u64() {
                    n.to_string()
                } else if let Some(s) = v["orderId"].as_str() {
                    s.to_string()
                } else {
                    return Err(OrderAdapterError::Serialization(
                        "reconcile response missing orderId".to_string(),
                    ));
                };
                let result = Self::parse_order_status_response(200, body)?;
                Ok(Some(ReconciledOrder {
                    order_id: OrderId(order_id),
                    result,
                }))
            }
            404 => Ok(None),
            400 | 422 => {
                if crate::order::port::is_order_not_found_body(body) {
                    Ok(None)
                } else {
                    Err(OrderAdapterError::Rejected(format!(
                        "unexpected reconcile status {status}: {body}"
                    )))
                }
            }
            401 | 403 => Err(OrderAdapterError::AuthFailed),
            _ if status >= 500 => Err(OrderAdapterError::ServerError {
                status,
                body: body.to_string(),
            }),
            _ => Err(OrderAdapterError::Rejected(format!(
                "unexpected reconcile status {status}"
            ))),
        }
    }
}

#[async_trait]
impl OrderAdapter for HitobitOrderAdapter {
    fn exchange_id(&self) -> ExchangeId {
        self.exchange_id.clone()
    }

    #[tracing::instrument(skip(self, req), fields(
        exchange = "hitobit",
        pair = %req.pair,
        side = %req.side,
        client_order_id = %req.client_order_id,
    ))]
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderId, OrderAdapterError> {
        tracing::info!(
            exchange = "hitobit",
            pair = %req.pair,
            side = %req.side,
            order_type = %req.order_type,
            quantity = %req.quantity,
            client_order_id = %req.client_order_id,
            strategy_id = ?req.strategy_id,
            "placing order"
        );

        let body = Self::build_place_order_body(req);
        let url = format!("{}/api/v1/order", self.base_url);

        let response = tokio::time::timeout(PLACE_ORDER_TIMEOUT, async {
            self.http_client
                .post(&url)
                .header("X-MBX-APIKEY", &self.api_key)
                .json(&body)
                .send()
                .await
        })
        .await
        .map_err(|_| {
            tracing::error!(
                exchange = "hitobit",
                pair = %req.pair,
                client_order_id = %req.client_order_id,
                "place_order timed out"
            );
            OrderAdapterError::NetworkTimeout("place_order timed out after 10s".to_string())
        })?
        .map_err(|e| {
            tracing::error!(error = %e, exchange = "hitobit", "place_order network error");
            OrderAdapterError::NetworkTimeout(e.to_string())
        })?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            OrderAdapterError::Serialization(format!("failed to read response body: {e}"))
        })?;

        Self::parse_place_order_response(status, &text)
    }

    #[tracing::instrument(skip(self), fields(exchange = "hitobit", order_id = %order_id))]
    async fn cancel_order(&self, order_id: &OrderId) -> Result<(), OrderAdapterError> {
        tracing::info!(
            exchange = "hitobit",
            order_id = %order_id,
            "cancelling order"
        );

        let url = format!("{}/api/v1/order", self.base_url);

        let response = tokio::time::timeout(CANCEL_ORDER_TIMEOUT, async {
            self.http_client
                .delete(&url)
                .header("X-MBX-APIKEY", &self.api_key)
                .json(&json!({ "orderId": &order_id.0 }))
                .send()
                .await
        })
        .await
        .map_err(|_| OrderAdapterError::NetworkTimeout("cancel_order timed out".to_string()))?
        .map_err(|e| OrderAdapterError::NetworkTimeout(e.to_string()))?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            OrderAdapterError::Serialization(format!("failed to read response body: {e}"))
        })?;

        Self::parse_cancel_order_response(status, &text)
    }

    #[tracing::instrument(skip(self), fields(exchange = "hitobit", order_id = %order_id))]
    async fn get_order_status(
        &self,
        order_id: &OrderId,
    ) -> Result<OrderStatusResult, OrderAdapterError> {
        let url = format!("{}/api/v1/order?orderId={}", self.base_url, order_id.0);

        let response = tokio::time::timeout(STATUS_TIMEOUT, async {
            self.http_client
                .get(&url)
                .header("X-MBX-APIKEY", &self.api_key)
                .send()
                .await
        })
        .await
        .map_err(|_| OrderAdapterError::NetworkTimeout("get_order_status timed out".to_string()))?
        .map_err(|e| OrderAdapterError::NetworkTimeout(e.to_string()))?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            OrderAdapterError::Serialization(format!("failed to read response body: {e}"))
        })?;

        Self::parse_order_status_response(status, &text)
    }

    #[tracing::instrument(skip(self), fields(exchange = "hitobit", client_order_id = %client_order_id))]
    async fn get_order_status_by_client_id(
        &self,
        client_order_id: &str,
    ) -> Result<Option<ReconciledOrder>, OrderAdapterError> {
        let url = format!(
            "{}/api/v1/order?origClientOrderId={}",
            self.base_url, client_order_id
        );

        let response = tokio::time::timeout(STATUS_TIMEOUT, async {
            self.http_client
                .get(&url)
                .header("X-MBX-APIKEY", &self.api_key)
                .send()
                .await
        })
        .await
        .map_err(|_| {
            OrderAdapterError::NetworkTimeout("get_order_status_by_client_id timed out".to_string())
        })?
        .map_err(|e| OrderAdapterError::NetworkTimeout(e.to_string()))?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            OrderAdapterError::Serialization(format!("failed to read response body: {e}"))
        })?;

        Self::parse_reconcile_response(status, &text)
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;
    use crate::order::port::{OrderRequest, OrderSide, OrderType};

    fn buy_limit_request() -> OrderRequest {
        OrderRequest {
            exchange: "hitobit".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::new(100, 0),
            price: Some(Decimal::new(58_000, 0)),
            client_order_id: "test-uuid-hitobit-001".to_string(),
            strategy_id: None,
        }
    }

    #[test]
    fn hitobit_order_builds_correct_request() {
        let req = buy_limit_request();
        let body = HitobitOrderAdapter::build_place_order_body(&req);

        assert_eq!(body["symbol"], "USDTIRT");
        assert_eq!(body["side"], "BUY");
        assert_eq!(body["type"], "LIMIT");
        assert_eq!(body["quantity"], "100");
        assert_eq!(body["price"], "58000");
        assert_eq!(body["timeInForce"], "GTC");
        assert_eq!(body["newClientOrderId"], "test-uuid-hitobit-001");
    }

    #[test]
    fn hitobit_order_market_sell_omits_price_and_time_in_force() {
        let req = OrderRequest {
            exchange: "hitobit".to_string(),
            pair: "BTC/IRT".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: Decimal::new(1, 3),
            price: None,
            client_order_id: "market-uuid".to_string(),
            strategy_id: None,
        };
        let body = HitobitOrderAdapter::build_place_order_body(&req);

        assert_eq!(body["symbol"], "BTCIRT");
        assert_eq!(body["side"], "SELL");
        assert_eq!(body["type"], "MARKET");
        assert!(body["price"].is_null());
        assert!(body["timeInForce"].is_null());
    }

    #[test]
    fn hitobit_symbol_concatenates_uppercase() {
        assert_eq!(HitobitOrderAdapter::symbol("BTC/USDT"), "BTCUSDT");
        assert_eq!(HitobitOrderAdapter::symbol("eth/irt"), "ETHIRT");
    }

    #[test]
    fn adapter_returns_order_id_on_success() {
        let body = r#"{"orderId": 87654321, "status": "NEW", "symbol": "USDTIRT"}"#;
        let result = HitobitOrderAdapter::parse_place_order_response(200, body);

        assert!(result.is_ok(), "200 response must be Ok");
        assert_eq!(result.unwrap().to_string(), "87654321");
    }

    #[test]
    fn adapter_returns_err_on_exchange_rejection() {
        let body = r#"{"code": -1013, "msg": "Filter failure: NOTIONAL."}"#;
        let result = HitobitOrderAdapter::parse_place_order_response(400, body);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OrderAdapterError::Rejected(msg) if msg.contains("NOTIONAL")
        ));
    }

    #[test]
    fn adapter_returns_auth_failed_on_401() {
        let result =
            HitobitOrderAdapter::parse_place_order_response(401, r#"{"msg":"Unauthorized"}"#);
        assert!(matches!(result, Err(OrderAdapterError::AuthFailed)));
    }

    #[test]
    fn adapter_returns_server_error_on_500() {
        let result = HitobitOrderAdapter::parse_place_order_response(500, "Internal Server Error");
        assert!(matches!(
            result,
            Err(OrderAdapterError::ServerError { status: 500, .. })
        ));
    }

    #[test]
    fn adapter_status_new_maps_to_open() {
        let body = r#"{"orderId":12345,"status":"NEW","symbol":"USDTIRT"}"#;
        let result = HitobitOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Open);
        assert!(result.fill_price.is_none());
    }

    #[test]
    fn adapter_status_partially_filled_maps_to_partially_filled() {
        let body = r#"{"orderId":12345,"status":"PARTIALLY_FILLED"}"#;
        let result = HitobitOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::PartiallyFilled);
    }

    #[test]
    fn adapter_status_filled_includes_avg_price() {
        let body = r#"{"orderId":12345,"status":"FILLED","avgPrice":"58000.00"}"#;
        let result = HitobitOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
        assert!(
            result.fill_price.is_some(),
            "avgPrice must be parsed as fill_price"
        );
    }

    #[test]
    fn adapter_exchange_id_is_hitobit() {
        let adapter = HitobitOrderAdapter::new("test-key");
        assert_eq!(adapter.exchange_id().to_string(), "hitobit");
    }

    #[test]
    fn reconcile_200_returns_landed_with_id_and_status() {
        let body = r#"{"orderId":424242,"status":"NEW"}"#;
        let reconciled = HitobitOrderAdapter::parse_reconcile_response(200, body)
            .unwrap()
            .expect("a 200 with an orderId means the order landed");
        assert_eq!(reconciled.order_id.0, "424242");
        assert_eq!(reconciled.result.status, OrderStatus::Open);
    }

    #[test]
    fn reconcile_404_returns_none() {
        let result = HitobitOrderAdapter::parse_reconcile_response(404, r#"{"msg":"not found"}"#);
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn reconcile_order_does_not_exist_code_returns_none() {
        let body = r#"{"code":-2013,"msg":"Order does not exist."}"#;
        assert_eq!(
            HitobitOrderAdapter::parse_reconcile_response(400, body).unwrap(),
            None
        );
    }

    #[test]
    fn reconcile_500_is_inconclusive_error() {
        let result = HitobitOrderAdapter::parse_reconcile_response(500, "boom");
        assert!(matches!(
            result,
            Err(OrderAdapterError::ServerError { status: 500, .. })
        ));
    }
}
