use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::exchange::entity::ExchangeId;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderStatus, OrderStatusResult,
    OrderType, ReconciledOrder,
};
use crate::order::signing;

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
    /// HMAC signing secret. `None` for the system adapter configured with only an
    /// API key (legacy / read-only); when present, every write request is signed.
    api_secret: Option<String>,
    http_client: reqwest::Client,
}

impl TabdealOrderAdapter {
    pub fn new(api_key: &str) -> Self {
        Self::with_base_url("https://api1.tabdeal.org", api_key)
    }

    pub fn with_base_url(base_url: &str, api_key: &str) -> Self {
        Self::with_credentials(base_url, api_key, None)
    }

    /// Constructs an adapter with an optional HMAC signing secret (C10).
    pub fn with_credentials(base_url: &str, api_key: &str, api_secret: Option<String>) -> Self {
        Self {
            exchange_id: ExchangeId::new("tabdeal"),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            api_secret,
            http_client: reqwest::Client::new(),
        }
    }

    /// Converts `"USDT/IRT"` → `"USDTIRT"` — Tabdeal's symbol format.
    fn symbol(pair: &str) -> String {
        pair.replace('/', "").to_uppercase()
    }

    /// Ordered request parameters for a place-order call (Binance-style names).
    pub fn place_order_params(req: &OrderRequest) -> Vec<(&'static str, String)> {
        let mut params = vec![
            ("symbol", Self::symbol(&req.pair)),
            ("side", req.side.to_string().to_uppercase()),
            ("type", req.order_type.to_string().to_uppercase()),
            ("quantity", req.quantity.to_string()),
            ("newClientOrderId", req.client_order_id.clone()),
        ];
        if req.order_type == OrderType::Limit {
            if let Some(price) = &req.price {
                params.push(("price", price.to_string()));
                params.push(("timeInForce", "GTC".to_string()));
            }
        }
        params
    }

    /// Encodes `params` and, when a secret is configured, appends `recvWindow`,
    /// `timestamp`, and a Binance-style HMAC `signature` over the encoded string
    /// (C10). Without a secret the params are returned unsigned (system adapter).
    /// The returned string is exactly what must be sent so the signature matches.
    fn signed_payload(&self, mut params: Vec<(&str, String)>, timestamp_ms: i64) -> String {
        match &self.api_secret {
            Some(secret) => {
                params.push(("recvWindow", "5000".to_string()));
                params.push(("timestamp", timestamp_ms.to_string()));
                let encoded = signing::encode_query(&params);
                let signature = signing::hmac_sha256_hex(secret, &encoded);
                format!("{encoded}&signature={signature}")
            }
            None => signing::encode_query(&params),
        }
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
                let filled_quantity = v["executedQty"]
                    .as_str()
                    .and_then(|s| s.parse::<rust_decimal::Decimal>().ok());
                Ok(OrderStatusResult {
                    status: order_status,
                    fill_price,
                    filled_quantity,
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
                let order_id = if let Some(s) = v["orderId"].as_str() {
                    s.to_string()
                } else if let Some(n) = v["orderId"].as_u64() {
                    n.to_string()
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
            // 404, or a Binance-compatible "order does not exist" (code -2013),
            // means the order genuinely never landed — safe to treat as no record.
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

        if self.api_secret.is_none() {
            tracing::warn!(
                exchange = "tabdeal",
                client_order_id = %req.client_order_id,
                "placing order WITHOUT request signing — no api_secret configured for this adapter"
            );
        }
        let payload = self.signed_payload(
            Self::place_order_params(req),
            signing::current_timestamp_ms(),
        );
        let url = format!("{}/api/v1/order", self.base_url);

        let response = tokio::time::timeout(PLACE_ORDER_TIMEOUT, async {
            self.http_client
                .post(&url)
                .header("X-MBX-APIKEY", &self.api_key)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(payload)
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

        let query = self.signed_payload(
            vec![("orderId", order_id.0.clone())],
            signing::current_timestamp_ms(),
        );
        let url = format!("{}/api/v1/order?{}", self.base_url, query);

        let response = tokio::time::timeout(CANCEL_ORDER_TIMEOUT, async {
            self.http_client
                .delete(&url)
                .header("X-MBX-APIKEY", &self.api_key)
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
    async fn get_order_status(
        &self,
        order_id: &OrderId,
    ) -> Result<OrderStatusResult, OrderAdapterError> {
        let query = self.signed_payload(
            vec![("orderId", order_id.0.clone())],
            signing::current_timestamp_ms(),
        );
        let url = format!("{}/api/v1/order?{}", self.base_url, query);

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

    #[tracing::instrument(skip(self), fields(exchange = "tabdeal", client_order_id = %client_order_id))]
    async fn get_order_status_by_client_id(
        &self,
        client_order_id: &str,
    ) -> Result<Option<ReconciledOrder>, OrderAdapterError> {
        let query = self.signed_payload(
            vec![("origClientOrderId", client_order_id.to_string())],
            signing::current_timestamp_ms(),
        );
        let url = format!("{}/api/v1/order?{}", self.base_url, query);

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
    fn tabdeal_order_builds_correct_params() {
        let req = buy_limit_request();
        let params = TabdealOrderAdapter::place_order_params(&req);

        assert_eq!(params[0], ("symbol", "USDTIRT".to_string()));
        assert_eq!(params[1], ("side", "BUY".to_string()));
        assert_eq!(params[2], ("type", "LIMIT".to_string()));
        assert_eq!(params[3], ("quantity", "100".to_string()));
        assert_eq!(params[4], ("newClientOrderId", "test-uuid-001".to_string()));
        assert!(params.contains(&("price", "58000".to_string())));
        assert!(params.contains(&("timeInForce", "GTC".to_string())));
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
        let params = TabdealOrderAdapter::place_order_params(&req);

        assert!(params.contains(&("type", "MARKET".to_string())));
        assert!(
            !params.iter().any(|(k, _)| *k == "price"),
            "market order must not include a price param"
        );
        assert!(!params.iter().any(|(k, _)| *k == "timeInForce"));
    }

    #[test]
    fn tabdeal_signed_payload_appends_timestamp_and_valid_signature() {
        // With a secret, the payload must carry timestamp + a signature that is the
        // HMAC over everything preceding `&signature=` (so the exchange can verify).
        let adapter =
            TabdealOrderAdapter::with_credentials("http://x", "key", Some("topsecret".to_string()));
        let payload = adapter.signed_payload(
            vec![
                ("symbol", "USDTIRT".to_string()),
                ("side", "BUY".to_string()),
            ],
            1_700_000_000_000,
        );
        assert!(payload.contains("timestamp=1700000000000"));
        assert!(payload.contains("recvWindow=5000"));
        let (signed_part, sig_part) = payload.rsplit_once("&signature=").expect("has signature");
        assert_eq!(
            sig_part,
            signing::hmac_sha256_hex("topsecret", signed_part),
            "signature must be the HMAC over the exact preceding payload"
        );
    }

    #[test]
    fn tabdeal_signed_payload_is_unsigned_without_secret() {
        // The system adapter (no secret) sends plain params — no signature/timestamp.
        let adapter = TabdealOrderAdapter::new("key");
        let payload =
            adapter.signed_payload(vec![("symbol", "USDTIRT".to_string())], 1_700_000_000_000);
        assert_eq!(payload, "symbol=USDTIRT");
        assert!(!payload.contains("signature="));
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
        let result = TabdealOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Open);
        assert!(result.fill_price.is_none());
    }

    #[test]
    fn adapter_status_filled_parses_filled() {
        let body = r#"{"orderId":"123","status":"FILLED"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
    }

    #[test]
    fn adapter_status_filled_includes_avg_price() {
        let body = r#"{"orderId":"123","status":"FILLED","avgPrice":"58000.00"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
        assert!(
            result.fill_price.is_some(),
            "avgPrice must be parsed as fill_price"
        );
    }

    #[test]
    fn adapter_status_partially_filled_extracts_executed_qty() {
        let body =
            r#"{"orderId":"1","status":"PARTIALLY_FILLED","executedQty":"40","avgPrice":"58000"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::PartiallyFilled);
        assert_eq!(result.filled_quantity, Some(Decimal::new(40, 0)));
    }

    #[test]
    fn adapter_status_without_executed_qty_has_none_filled_quantity() {
        let body = r#"{"orderId":"1","status":"NEW"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert!(result.filled_quantity.is_none());
    }

    #[test]
    fn adapter_status_cancelled_parses_canceled() {
        let body = r#"{"orderId":"123","status":"CANCELED"}"#;
        let result = TabdealOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Cancelled);
    }

    #[test]
    fn adapter_exchange_id_is_tabdeal() {
        let adapter = TabdealOrderAdapter::new("test-key");
        assert_eq!(adapter.exchange_id().to_string(), "tabdeal");
    }

    #[test]
    fn reconcile_200_returns_landed_with_id_and_status() {
        let body = r#"{"orderId":"98765","status":"FILLED","avgPrice":"58000.00"}"#;
        let reconciled = TabdealOrderAdapter::parse_reconcile_response(200, body)
            .unwrap()
            .expect("a 200 with an orderId means the order landed");
        assert_eq!(reconciled.order_id.0, "98765");
        assert_eq!(reconciled.result.status, OrderStatus::Filled);
        assert!(reconciled.result.fill_price.is_some());
    }

    #[test]
    fn reconcile_404_returns_none() {
        let result = TabdealOrderAdapter::parse_reconcile_response(404, r#"{"msg":"not found"}"#);
        assert_eq!(
            result.unwrap(),
            None,
            "a 404 means the exchange has no record — the order never landed"
        );
    }

    #[test]
    fn reconcile_order_does_not_exist_code_returns_none() {
        let body = r#"{"code":-2013,"msg":"Order does not exist."}"#;
        let result = TabdealOrderAdapter::parse_reconcile_response(400, body);
        assert_eq!(
            result.unwrap(),
            None,
            "Binance code -2013 means the order never landed"
        );
    }

    #[test]
    fn reconcile_500_is_inconclusive_error() {
        let result = TabdealOrderAdapter::parse_reconcile_response(503, "Service Unavailable");
        assert!(
            matches!(
                result,
                Err(OrderAdapterError::ServerError { status: 503, .. })
            ),
            "a 5xx during reconciliation is inconclusive and must surface as an error"
        );
    }

    #[test]
    fn reconcile_unexpected_400_is_inconclusive_error() {
        // A 4xx that is NOT a not-found must NOT be treated as "never landed".
        let body = r#"{"code":-1102,"msg":"Mandatory parameter was empty."}"#;
        let result = TabdealOrderAdapter::parse_reconcile_response(400, body);
        assert!(
            matches!(result, Err(OrderAdapterError::Rejected(_))),
            "an ambiguous 4xx must be an error, not a false 'never landed'"
        );
    }
}
