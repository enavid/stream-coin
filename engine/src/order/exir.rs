use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::exchange::entity::ExchangeId;
use crate::order::port::{
    OrderAdapter, OrderAdapterError, OrderId, OrderRequest, OrderStatus, OrderStatusResult,
    OrderType, ReconciledOrder,
};
use crate::order::signing;

const PLACE_ORDER_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_ORDER_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// REST order adapter for Exir.
///
/// Exir uses lowercase hyphen-separated symbols (e.g. "usdt-irt"), lowercase
/// sides ("buy"/"sell"), and lowercase types ("limit"/"market").
/// The quantity field is named `size` and the idempotency key is `client_order_id`.
pub struct ExirOrderAdapter {
    exchange_id: ExchangeId,
    base_url: String,
    api_key: String,
    /// HollaEx HMAC secret. `None` falls back to the legacy `Bearer` token; when
    /// present, requests are signed with `api-key`/`api-expires`/`api-signature` (C10).
    api_secret: Option<String>,
    http_client: reqwest::Client,
}

impl ExirOrderAdapter {
    /// Production REST base URL. Overridable per deployment via
    /// `EXIR_REST_BASE_URL` (see `engine/bin/http.rs`).
    pub const DEFAULT_BASE_URL: &'static str = "https://api.exir.io";

    pub fn new(api_key: &str) -> Self {
        Self::with_base_url(Self::DEFAULT_BASE_URL, api_key)
    }

    pub fn with_base_url(base_url: &str, api_key: &str) -> Self {
        Self::with_credentials(base_url, api_key, None)
    }

    /// Constructs an adapter with an optional HollaEx HMAC signing secret (C10).
    pub fn with_credentials(base_url: &str, api_key: &str, api_secret: Option<String>) -> Self {
        Self {
            exchange_id: ExchangeId::new("exir"),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            api_secret,
            http_client: reqwest::Client::new(),
        }
    }

    /// HollaEx request signature: `HMAC-SHA256(secret, VERB + path + expires + body)`.
    /// `path` includes any query string; `body` is the exact JSON sent ("" for GET).
    pub fn hollaex_signature(
        secret: &str,
        verb: &str,
        path: &str,
        expires: i64,
        body: &str,
    ) -> String {
        signing::hmac_sha256_hex(secret, &format!("{verb}{path}{expires}{body}"))
    }

    /// Applies authentication to a request builder. With a secret: HollaEx HMAC
    /// headers signing `VERB + path + expires + body`. Without: legacy Bearer token.
    fn authed(
        &self,
        builder: reqwest::RequestBuilder,
        verb: &str,
        path: &str,
        body: &str,
    ) -> reqwest::RequestBuilder {
        match &self.api_secret {
            Some(secret) => {
                // HollaEx expires is a Unix *seconds* deadline.
                let expires = signing::current_timestamp_ms() / 1000 + 60;
                let signature = Self::hollaex_signature(secret, verb, path, expires, body);
                builder
                    .header("api-key", &self.api_key)
                    .header("api-expires", expires.to_string())
                    .header("api-signature", signature)
            }
            None => builder.header("Authorization", format!("Bearer {}", self.api_key)),
        }
    }

    /// Converts `"USDT/IRT"` → `"usdt-irt"` — Exir's symbol format.
    fn symbol(pair: &str) -> String {
        pair.replace('/', "-").to_lowercase()
    }

    /// Builds the JSON body for a place-order POST request.
    pub fn build_place_order_body(req: &OrderRequest) -> Value {
        let mut body = json!({
            "symbol": Self::symbol(&req.pair),
            "side": req.side.to_string(),
            "type": req.order_type.to_string(),
            "size": req.quantity.to_string(),
            "client_order_id": req.client_order_id,
        });

        if req.order_type == OrderType::Limit {
            if let Some(price) = &req.price {
                body["price"] = json!(price.to_string());
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
                let order_id = if let Some(s) = v["id"].as_str() {
                    s.to_string()
                } else if let Some(n) = v["id"].as_u64() {
                    n.to_string()
                } else if let Some(s) = v["order_id"].as_str() {
                    s.to_string()
                } else {
                    return Err(OrderAdapterError::Serialization(
                        "place-order response missing id".to_string(),
                    ));
                };
                tracing::info!(
                    order_id = %order_id,
                    exchange = "exir",
                    "order placed successfully"
                );
                Ok(OrderId(order_id))
            }
            400 | 422 => {
                let msg = serde_json::from_str::<Value>(body)
                    .ok()
                    .and_then(|v| {
                        v["message"]
                            .as_str()
                            .or_else(|| v["msg"].as_str())
                            .or_else(|| v["error"].as_str())
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
                    .and_then(|v| {
                        v["message"]
                            .as_str()
                            .or_else(|| v["error"].as_str())
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
                let raw = v["status"].as_str().unwrap_or("").to_lowercase();
                let order_status = match raw.as_str() {
                    "new" | "open" | "pending" => OrderStatus::Open,
                    "filled" | "completed" => OrderStatus::Filled,
                    "partially_filled" | "partial" => OrderStatus::PartiallyFilled,
                    "cancelled" | "canceled" => OrderStatus::Cancelled,
                    _ => OrderStatus::Failed,
                };
                let fill_price = v["filled_average_price"]
                    .as_str()
                    .or_else(|| v["average_price"].as_str())
                    .or_else(|| v["price"].as_str())
                    .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
                    .filter(|p| *p > rust_decimal::Decimal::ZERO);
                // Exir reports the executed amount in `filled`, which may arrive as
                // a JSON number or a string.
                let filled_quantity = v["filled"]
                    .as_str()
                    .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
                    .or_else(|| {
                        v["filled"]
                            .as_f64()
                            .and_then(rust_decimal::Decimal::from_f64_retain)
                    });
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

    /// Parses a reconciliation response (`GET ...?client_order_id=`).
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
                let order_id = if let Some(s) = v["id"].as_str() {
                    s.to_string()
                } else if let Some(n) = v["id"].as_u64() {
                    n.to_string()
                } else if let Some(s) = v["order_id"].as_str() {
                    s.to_string()
                } else {
                    return Err(OrderAdapterError::Serialization(
                        "reconcile response missing id".to_string(),
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
impl OrderAdapter for ExirOrderAdapter {
    fn exchange_id(&self) -> ExchangeId {
        self.exchange_id.clone()
    }

    #[tracing::instrument(skip(self, req), fields(
        exchange = "exir",
        pair = %req.pair,
        side = %req.side,
        client_order_id = %req.client_order_id,
    ))]
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderId, OrderAdapterError> {
        tracing::info!(
            exchange = "exir",
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
                exchange = "exir",
                client_order_id = %req.client_order_id,
                "placing order WITHOUT request signing — no api_secret configured for this adapter"
            );
        }
        let body = Self::build_place_order_body(req).to_string();
        let path = "/v2/order";
        let url = format!("{}{}", self.base_url, path);

        let response = tokio::time::timeout(PLACE_ORDER_TIMEOUT, async {
            self.authed(self.http_client.post(&url), "POST", path, &body)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await
        })
        .await
        .map_err(|_| {
            tracing::error!(
                exchange = "exir",
                pair = %req.pair,
                client_order_id = %req.client_order_id,
                "place_order timed out"
            );
            OrderAdapterError::NetworkTimeout("place_order timed out after 10s".to_string())
        })?
        .map_err(|e| {
            tracing::error!(error = %e, exchange = "exir", "place_order network error");
            OrderAdapterError::NetworkTimeout(e.to_string())
        })?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            OrderAdapterError::Serialization(format!("failed to read response body: {e}"))
        })?;

        Self::parse_place_order_response(status, &text)
    }

    #[tracing::instrument(skip(self), fields(exchange = "exir", order_id = %order_id))]
    async fn cancel_order(&self, order_id: &OrderId) -> Result<(), OrderAdapterError> {
        tracing::info!(
            exchange = "exir",
            order_id = %order_id,
            "cancelling order"
        );

        let body = json!({ "order_id": &order_id.0 }).to_string();
        let path = "/v2/order";
        let url = format!("{}{}", self.base_url, path);

        let response = tokio::time::timeout(CANCEL_ORDER_TIMEOUT, async {
            self.authed(self.http_client.delete(&url), "DELETE", path, &body)
                .header("Content-Type", "application/json")
                .body(body.clone())
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

    #[tracing::instrument(skip(self), fields(exchange = "exir", order_id = %order_id))]
    async fn get_order_status(
        &self,
        order_id: &OrderId,
    ) -> Result<OrderStatusResult, OrderAdapterError> {
        let path = format!("/v2/order/{}", order_id.0);
        let url = format!("{}{}", self.base_url, path);

        let response = tokio::time::timeout(STATUS_TIMEOUT, async {
            self.authed(self.http_client.get(&url), "GET", &path, "")
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

    #[tracing::instrument(skip(self), fields(exchange = "exir", client_order_id = %client_order_id))]
    async fn get_order_status_by_client_id(
        &self,
        client_order_id: &str,
    ) -> Result<Option<ReconciledOrder>, OrderAdapterError> {
        let path = format!("/v2/order?client_order_id={client_order_id}");
        let url = format!("{}{}", self.base_url, path);

        let response = tokio::time::timeout(STATUS_TIMEOUT, async {
            self.authed(self.http_client.get(&url), "GET", &path, "")
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
            exchange: "exir".to_string(),
            pair: "USDT/IRT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::new(100, 0),
            price: Some(Decimal::new(58_000, 0)),
            client_order_id: "test-uuid-exir-001".to_string(),
            strategy_id: None,
        }
    }

    #[test]
    fn exir_order_builds_correct_request() {
        let req = buy_limit_request();
        let body = ExirOrderAdapter::build_place_order_body(&req);

        assert_eq!(body["symbol"], "usdt-irt");
        assert_eq!(body["side"], "buy");
        assert_eq!(body["type"], "limit");
        assert_eq!(body["size"], "100");
        assert_eq!(body["price"], "58000");
        assert_eq!(body["client_order_id"], "test-uuid-exir-001");
    }

    #[test]
    fn exir_order_market_buy_omits_price() {
        let req = OrderRequest {
            exchange: "exir".to_string(),
            pair: "BTC/IRT".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: Decimal::new(1, 3),
            price: None,
            client_order_id: "market-exir".to_string(),
            strategy_id: None,
        };
        let body = ExirOrderAdapter::build_place_order_body(&req);

        assert_eq!(body["symbol"], "btc-irt");
        assert_eq!(body["side"], "sell");
        assert_eq!(body["type"], "market");
        assert!(
            body["price"].is_null(),
            "market order must not include price"
        );
    }

    #[test]
    fn exir_symbol_is_lowercase_hyphenated() {
        assert_eq!(ExirOrderAdapter::symbol("USDT/IRT"), "usdt-irt");
        assert_eq!(ExirOrderAdapter::symbol("BTC/USDT"), "btc-usdt");
        assert_eq!(ExirOrderAdapter::symbol("eth/irt"), "eth-irt");
    }

    #[test]
    fn adapter_returns_order_id_on_success() {
        let body = r#"{"id": "abc-xyz-123", "type": "limit", "side": "buy", "status": "new"}"#;
        let result = ExirOrderAdapter::parse_place_order_response(200, body);

        assert!(result.is_ok(), "200 response must be Ok, got {:?}", result);
        assert_eq!(result.unwrap().to_string(), "abc-xyz-123");
    }

    #[test]
    fn adapter_returns_err_on_exchange_rejection() {
        let body = r#"{"message": "Minimum order size not met"}"#;
        let result = ExirOrderAdapter::parse_place_order_response(400, body);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OrderAdapterError::Rejected(msg) if msg.contains("Minimum order size")
        ));
    }

    #[test]
    fn adapter_returns_auth_failed_on_403() {
        let result = ExirOrderAdapter::parse_place_order_response(403, r#"{"error":"Forbidden"}"#);
        assert!(matches!(result, Err(OrderAdapterError::AuthFailed)));
    }

    #[test]
    fn adapter_returns_server_error_on_500() {
        let result = ExirOrderAdapter::parse_place_order_response(500, "Internal Server Error");
        assert!(matches!(
            result,
            Err(OrderAdapterError::ServerError { status: 500, .. })
        ));
    }

    #[test]
    fn adapter_status_open_maps_from_new() {
        let body = r#"{"id":"abc","status":"new","side":"buy"}"#;
        let result = ExirOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Open);
        assert!(result.fill_price.is_none());
    }

    #[test]
    fn adapter_status_filled_maps_from_completed() {
        let body = r#"{"id":"abc","status":"completed"}"#;
        let result = ExirOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
    }

    #[test]
    fn adapter_status_filled_includes_fill_price() {
        let body = r#"{"id":"abc","status":"filled","filled_average_price":"58000.50"}"#;
        let result = ExirOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
        assert!(
            result.fill_price.is_some(),
            "filled_average_price must be parsed as fill_price"
        );
    }

    #[test]
    fn adapter_status_partially_filled_extracts_filled_from_string() {
        let body = r#"{"id":"abc","status":"partial","filled":"40"}"#;
        let result = ExirOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.status, OrderStatus::PartiallyFilled);
        assert_eq!(result.filled_quantity, Some(Decimal::new(40, 0)));
    }

    #[test]
    fn adapter_status_partially_filled_extracts_filled_from_number() {
        let body = r#"{"id":"abc","status":"partial","filled":40}"#;
        let result = ExirOrderAdapter::parse_order_status_response(200, body).unwrap();
        assert_eq!(result.filled_quantity, Some(Decimal::new(40, 0)));
    }

    #[test]
    fn adapter_cancel_returns_ok_on_200() {
        let result =
            ExirOrderAdapter::parse_cancel_order_response(200, r#"{"status":"cancelled"}"#);
        assert!(result.is_ok());
    }

    #[test]
    fn adapter_cancel_returns_rejected_on_400() {
        let result = ExirOrderAdapter::parse_cancel_order_response(
            400,
            r#"{"message":"Order already filled"}"#,
        );
        assert!(matches!(result, Err(OrderAdapterError::Rejected(_))));
    }

    #[test]
    fn adapter_exchange_id_is_exir() {
        let adapter = ExirOrderAdapter::new("test-key");
        assert_eq!(adapter.exchange_id().to_string(), "exir");
    }

    #[test]
    fn hollaex_signature_signs_verb_path_expires_body() {
        // HollaEx signs VERB + path + expires + body with HMAC-SHA256; pin the
        // exact concatenation so the wire signature is reproducible.
        let sig = ExirOrderAdapter::hollaex_signature(
            "sekret",
            "POST",
            "/v2/order",
            1_700_000_000,
            r#"{"symbol":"usdt-irt"}"#,
        );
        let expected =
            signing::hmac_sha256_hex("sekret", "POST/v2/order1700000000{\"symbol\":\"usdt-irt\"}");
        assert_eq!(sig, expected);
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn hollaex_signature_differs_for_get_with_empty_body() {
        let post = ExirOrderAdapter::hollaex_signature("s", "POST", "/v2/order", 1, "{}");
        let get = ExirOrderAdapter::hollaex_signature("s", "GET", "/v2/order", 1, "");
        assert_ne!(post, get, "verb and body must affect the signature");
    }

    #[test]
    fn reconcile_200_returns_landed_with_id_and_status() {
        let body = r#"{"id":"exir-abc","status":"new"}"#;
        let reconciled = ExirOrderAdapter::parse_reconcile_response(200, body)
            .unwrap()
            .expect("a 200 with an id means the order landed");
        assert_eq!(reconciled.order_id.0, "exir-abc");
        assert_eq!(reconciled.result.status, OrderStatus::Open);
    }

    #[test]
    fn reconcile_404_returns_none() {
        let result = ExirOrderAdapter::parse_reconcile_response(404, r#"{"message":"not found"}"#);
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn reconcile_order_not_found_message_returns_none() {
        let body = r#"{"message":"Order does not exist"}"#;
        assert_eq!(
            ExirOrderAdapter::parse_reconcile_response(400, body).unwrap(),
            None
        );
    }

    #[test]
    fn reconcile_500_is_inconclusive_error() {
        let result = ExirOrderAdapter::parse_reconcile_response(502, "bad gateway");
        assert!(matches!(
            result,
            Err(OrderAdapterError::ServerError { status: 502, .. })
        ));
    }
}
