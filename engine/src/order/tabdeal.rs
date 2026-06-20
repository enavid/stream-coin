use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::exchange::entity::ExchangeId;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderStatus, OrderType,
};

const PLACE_ORDER_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_ORDER_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// REST order adapter for Tabdeal.
///
/// Tabdeal's order API mirrors Binance's format: symbols are concatenated
/// uppercase (e.g. "USDTIRT"), sides are "BUY"/"SELL", types are "MARKET"/"LIMIT".
pub struct TabdealOrderAdapter {
    exchange_id: ExchangeId,
    base_url: String,
    api_key: String,
    http_client: reqwest::Client,
}

impl TabdealOrderAdapter {
    pub fn new(api_key: &str) -> Self {
        Self::with_base_url("https://api1.tabdeal.org", api_key)
    }

    pub fn with_base_url(base_url: &str, api_key: &str) -> Self {
        Self {
            exchange_id: ExchangeId::new("tabdeal"),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            http_client: reqwest::Client::new(),
        }
    }

    /// Converts `"USDT/IRT"` → `"USDTIRT"` — Tabdeal's symbol format.
    fn symbol(pair: &str) -> String {
        pair.replace('/', "").to_uppercase()
    }

    /// Builds the JSON body for a place-order POST request.
    pub fn build_place_order_body(req: &OrderRequest) -> Value {
        let mut body = json!({
            "symbol": Self::symbol(&req.pair),
            "side": req.side.to_string().to_uppercase(),
            "type": req.order_type.to_string().to_uppercase(),
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
                let order_id = if let Some(s) = v["orderId"].as_str() {
                    s.to_string()
                } else if let Some(n) = v["orderId"].as_u64() {
                    n.to_string()
                } else if let Some(s) = v["order_id"].as_str() {
                    s.to_string()
                } else {
                    return Err(OrderAdapterError::Serialization(
                        "place-order response missing orderId".to_string(),
                    ));
                };
                tracing::info!(
                    order_id = %order_id,
                    exchange = "tabdeal",
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
    ) -> Result<OrderStatus, OrderAdapterError> {
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
                Ok(order_status)
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
}

#[async_trait]
impl OrderAdapter for TabdealOrderAdapter {
    fn exchange_id(&self) -> ExchangeId {
        self.exchange_id.clone()
    }

    #[tracing::instrument(skip(self, req), fields(
        exchange = "tabdeal",
        pair = %req.pair,
        side = %req.side,
        client_order_id = %req.client_order_id,
    ))]
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderId, OrderAdapterError> {
        tracing::info!(
            exchange = "tabdeal",
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
                exchange = "tabdeal",
                pair = %req.pair,
                client_order_id = %req.client_order_id,
                "place_order timed out"
            );
            OrderAdapterError::NetworkTimeout("place_order timed out after 10s".to_string())
        })?
        .map_err(|e| {
            tracing::error!(error = %e, exchange = "tabdeal", "place_order network error");
            OrderAdapterError::NetworkTimeout(e.to_string())
        })?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            OrderAdapterError::Serialization(format!("failed to read response body: {e}"))
        })?;

        Self::parse_place_order_response(status, &text)
    }

    #[tracing::instrument(skip(self), fields(exchange = "tabdeal", order_id = %order_id))]
    async fn cancel_order(&self, order_id: &OrderId) -> Result<(), OrderAdapterError> {
        tracing::info!(
            exchange = "tabdeal",
            order_id = %order_id,
            "cancelling order"
        );

        let url = format!("{}/api/v1/order", self.base_url);

        let response = tokio::time::timeout(CANCEL_ORDER_TIMEOUT, async {
            self.http_client
                .delete(&url)
                .header("X-MBX-APIKEY", &self.api_key)
                .json(&json!({ "orderId": order_id.0 }))
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

    #[tracing::instrument(skip(self), fields(exchange = "tabdeal", order_id = %order_id))]
    async fn get_order_status(&self, order_id: &OrderId) -> Result<OrderStatus, OrderAdapterError> {
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
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;
    use crate::order::port::{OrderRequest, OrderSide, OrderType};

    fn buy_limit_request() -> OrderRequest {
        OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::new(100, 0),
            price: Some(Decimal::new(58_000, 0)),
            client_order_id: "test-uuid-001".to_string(),
            strategy_id: Some("spread-1".to_string()),
        }
    }

    #[test]
    fn tabdeal_order_builds_correct_request() {
        let req = buy_limit_request();
        let body = TabdealOrderAdapter::build_place_order_body(&req);

        assert_eq!(body["symbol"], "USDTIRT");
        assert_eq!(body["side"], "BUY");
        assert_eq!(body["type"], "LIMIT");
        assert_eq!(body["quantity"], "100");
        assert_eq!(body["price"], "58000");
        assert_eq!(body["timeInForce"], "GTC");
        assert_eq!(body["newClientOrderId"], "test-uuid-001");
    }

    #[test]
    fn tabdeal_order_market_buy_omits_price_and_time_in_force() {
        let req = OrderRequest {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::new(50, 0),
            price: None,
            client_order_id: "market-uuid-001".to_string(),
            strategy_id: None,
        };
        let body = TabdealOrderAdapter::build_place_order_body(&req);

        assert_eq!(body["symbol"], "USDTIRT");
        assert_eq!(body["type"], "MARKET");
        assert!(
            body["price"].is_null(),
            "market order must not have a price field"
        );
        assert!(body["timeInForce"].is_null());
    }

    #[test]
    fn tabdeal_symbol_concatenates_base_and_quote_uppercase() {
        assert_eq!(TabdealOrderAdapter::symbol("USDT/IRT"), "USDTIRT");
        assert_eq!(TabdealOrderAdapter::symbol("BTC/IRT"), "BTCIRT");
        assert_eq!(TabdealOrderAdapter::symbol("ETH/USDT"), "ETHUSDT");
    }

    #[test]
    fn adapter_returns_order_id_on_success() {
        let body = r#"{"orderId": "12345", "status": "NEW"}"#;
        let result = TabdealOrderAdapter::parse_place_order_response(200, body);

        assert!(result.is_ok(), "200 response must be Ok, got {:?}", result);
        assert_eq!(result.unwrap().to_string(), "12345");
    }

    #[test]
    fn adapter_returns_order_id_when_order_id_is_numeric() {
        let body = r#"{"orderId": 99999, "status": "NEW"}"#;
        let result = TabdealOrderAdapter::parse_place_order_response(200, body);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().to_string(), "99999");
    }

    #[test]
    fn adapter_returns_err_on_exchange_rejection() {
        let body = r#"{"code": -1121, "msg": "Invalid symbol."}"#;
        let result = TabdealOrderAdapter::parse_place_order_response(400, body);

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), OrderAdapterError::Rejected(msg) if msg.contains("Invalid symbol")),
            "400 response must produce Rejected error with exchange message"
        );
    }

    #[test]
    fn adapter_returns_auth_failed_on_401() {
        let result =
            TabdealOrderAdapter::parse_place_order_response(401, r#"{"msg":"Unauthorized"}"#);
        assert!(matches!(result, Err(OrderAdapterError::AuthFailed)));
    }

    #[test]
    fn adapter_returns_server_error_on_503() {
        let result = TabdealOrderAdapter::parse_place_order_response(503, "Service Unavailable");
        assert!(matches!(
            result,
            Err(OrderAdapterError::ServerError { status: 503, .. })
        ));
    }

    #[test]
    fn adapter_cancel_returns_ok_on_200() {
        let result =
            TabdealOrderAdapter::parse_cancel_order_response(200, r#"{"status":"CANCELED"}"#);
        assert!(result.is_ok());
    }

    #[test]
    fn adapter_cancel_returns_rejected_on_400() {
        let result =
            TabdealOrderAdapter::parse_cancel_order_response(400, r#"{"msg":"Order not found"}"#);
        assert!(matches!(result, Err(OrderAdapterError::Rejected(_))));
    }

    #[test]
    fn adapter_status_open_parses_new() {
        let body = r#"{"orderId":"123","status":"NEW","symbol":"USDTIRT"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body);
        assert_eq!(result.unwrap(), OrderStatus::Open);
    }

    #[test]
    fn adapter_status_filled_parses_filled() {
        let body = r#"{"orderId":"123","status":"FILLED"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body);
        assert_eq!(result.unwrap(), OrderStatus::Filled);
    }

    #[test]
    fn adapter_status_cancelled_parses_canceled() {
        let body = r#"{"orderId":"123","status":"CANCELED"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body);
        assert_eq!(result.unwrap(), OrderStatus::Cancelled);
    }

    #[test]
    fn adapter_exchange_id_is_tabdeal() {
        let adapter = TabdealOrderAdapter::new("test-key");
        assert_eq!(adapter.exchange_id().to_string(), "tabdeal");
    }
}
