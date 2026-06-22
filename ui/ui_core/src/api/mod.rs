//! REST client for the engine's control plane. Live price/signal/order
//! data itself arrives over the WebSocket feed, not through this client —
//! see [`crate::protocol`]. This module only issues request/response
//! round-trips: start/stop tickers and strategies, run backtests, place
//! orders, and manage users/roles/credentials.
//!
//! `reqwest` compiles for both `wasm32` (via `fetch`) and native targets,
//! so this client works unmodified across web, desktop, and mobile.

mod dto;

pub use dto::*;

use std::rc::Rc;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Returned by every authenticated call when the engine's JWT middleware
/// rejects the token (expired or otherwise invalid) — distinguishes "your
/// session is gone" from every other failure so [`ApiClient::send`] can
/// fire the unauthorized handler regardless of how the specific call site
/// handles its `Result` (several discard the error entirely today).
pub const UNAUTHORIZED_ERROR: &str = "session expired — please log in again";

/// Body shared by the start/stop ticker endpoints. A free function so the
/// `BASE/QUOTE` separator requirement can be pinned down by a unit test
/// without going through the network.
fn ticker_request_body(exchange: &str, pair: &str) -> serde_json::Value {
    serde_json::json!({ "exchange": exchange, "symbol": pair })
}

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    on_unauthorized: Option<Rc<dyn Fn()>>,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            on_unauthorized: None,
        }
    }

    /// Registers a callback fired the moment any authenticated call comes
    /// back `401 Unauthorized` — wire it to `AppState::clear_session` so an
    /// expired JWT (the engine mints 24h-lived tokens) immediately bounces
    /// the user to the login screen instead of every page silently failing
    /// with no feedback until they manually log out and back in.
    pub fn with_unauthorized_handler(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_unauthorized = Some(Rc::new(handler));
        self
    }

    /// Pure decision given a status code — kept separate from `send` so the
    /// unauthorized-handler wiring is testable without a real HTTP round
    /// trip.
    fn handle_response_status(&self, status: reqwest::StatusCode) -> Result<(), String> {
        if status == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(handler) = &self.on_unauthorized {
                handler();
            }
            return Err(UNAUTHORIZED_ERROR.to_string());
        }
        Ok(())
    }

    fn v1(&self, path: &str) -> String {
        format!("{}/v1{path}", self.base_url)
    }

    pub fn ws_url(&self) -> String {
        let base = self
            .base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{base}/v1/ws")
    }

    /// A browser's native WebSocket API can't set an `Authorization`
    /// header on the upgrade request, so the JWT travels in the URL
    /// instead — the one exemption the backend's JWT middleware makes
    /// for `/v1/ws` specifically (`engine/src/presentation/middlewares/jwt.rs`).
    pub fn ws_url_with_token(&self, token: &str) -> String {
        format!("{}?token={token}", self.ws_url())
    }

    /// `pair` must keep its `BASE/QUOTE` separator (e.g. `USDT/IRT`) — the
    /// engine's `TradingPair` deserializer (`engine/src/price/entity.rs`)
    /// rejects anything without exactly one `/`, so the concatenated form
    /// (`USDTIRT`) is a 400, not an accepted shorthand.
    pub async fn start_ticker(
        &self,
        token: &str,
        exchange: &str,
        pair: &str,
    ) -> Result<(), String> {
        self.post_json(
            "/exchanges/futures/start_kline_symbol_ticker",
            Some(token),
            &ticker_request_body(exchange, pair),
        )
        .await
    }

    /// See [`ApiClient::start_ticker`] — same `BASE/QUOTE` requirement
    /// applies to the stop endpoint's `symbol` field.
    pub async fn stop_ticker(&self, token: &str, exchange: &str, pair: &str) -> Result<(), String> {
        self.post_json(
            "/exchanges/futures/stop_kline_symbol_ticker",
            Some(token),
            &ticker_request_body(exchange, pair),
        )
        .await
    }

    // --- generic authenticated request helpers ---

    /// Unwraps the backend's `{success, message, data}` envelope
    /// (`engine/src/presentation/responses/success_response.rs`) into the
    /// `data` value on success, or `Err(message)` otherwise — error
    /// responses share `success`/`message` but omit `data` entirely.
    /// A free function (not generic over `T`) so it's testable without
    /// touching the network or the deserialize target type at all.
    fn unwrap_envelope(body: serde_json::Value) -> Result<serde_json::Value, String> {
        let success = body
            .get("success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if success {
            Ok(body.get("data").cloned().unwrap_or(serde_json::Value::Null))
        } else {
            Err(body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
                .to_string())
        }
    }

    async fn send<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, String> {
        let resp = builder.send().await.map_err(|e| e.to_string())?;
        self.handle_response_status(resp.status())?;
        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let data = Self::unwrap_envelope(body)?;
        serde_json::from_value(data).map_err(|e| e.to_string())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, token: Option<&str>) -> Result<T, String> {
        let mut req = reqwest::Client::new().get(self.v1(path));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        self.send(req).await
    }

    async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> Result<T, String> {
        let mut req = reqwest::Client::new().post(self.v1(path)).json(body);
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        self.send(req).await
    }

    async fn delete<T: DeserializeOwned>(&self, path: &str, token: &str) -> Result<T, String> {
        let req = reqwest::Client::new()
            .delete(self.v1(path))
            .bearer_auth(token);
        self.send(req).await
    }

    // --- auth ---

    pub async fn login(&self, username: &str, password: &str) -> Result<TokenResponse, String> {
        self.post_json(
            "/auth/token",
            None,
            &LoginRequest {
                username: username.to_string(),
                password: password.to_string(),
            },
        )
        .await
    }

    // --- strategies ---

    pub async fn list_strategies(&self, token: &str) -> Result<StrategyList, String> {
        self.get("/strategies", Some(token)).await
    }

    pub async fn start_strategy(
        &self,
        token: &str,
        req: StartStrategyRequest,
    ) -> Result<(), String> {
        self.post_json("/strategies/start", Some(token), &req).await
    }

    pub async fn stop_strategy(&self, token: &str, strategy_id: &str) -> Result<(), String> {
        self.post_json(
            "/strategies/stop",
            Some(token),
            &StopStrategyRequest {
                strategy_id: strategy_id.to_string(),
            },
        )
        .await
    }

    pub async fn deploy_strategy(
        &self,
        token: &str,
        req: DeployStrategyRequest,
    ) -> Result<DeployedStrategy, String> {
        self.post_json("/strategies/deploy", Some(token), &req)
            .await
    }

    // --- backtest ---

    pub async fn run_backtest(
        &self,
        token: &str,
        req: BacktestRunRequest,
    ) -> Result<BacktestResult, String> {
        self.post_json("/backtest/run", Some(token), &req).await
    }

    // --- orders ---

    pub async fn place_order(
        &self,
        token: &str,
        req: PlaceOrderRequest,
    ) -> Result<OrderPlacedResponse, String> {
        self.post_json("/orders/place", Some(token), &req).await
    }

    pub async fn cancel_order(&self, token: &str, client_order_id: &str) -> Result<(), String> {
        self.post_json(
            "/orders/cancel",
            Some(token),
            &CancelOrderRequest {
                client_order_id: client_order_id.to_string(),
            },
        )
        .await
    }

    pub async fn list_orders(&self, token: &str) -> Result<OrderListResponse, String> {
        self.get("/orders", Some(token)).await
    }

    pub async fn reset_circuit_breaker(&self, token: &str) -> Result<(), String> {
        self.post_json(
            "/admin/circuit-breaker/reset",
            Some(token),
            &serde_json::json!({}),
        )
        .await
    }

    // --- admin: users / roles / permissions ---

    pub async fn list_users(&self, token: &str) -> Result<UserListResponse, String> {
        self.get("/admin/users", Some(token)).await
    }

    pub async fn create_user(
        &self,
        token: &str,
        username: &str,
        password: &str,
        roles: Vec<String>,
    ) -> Result<UserResponse, String> {
        self.post_json(
            "/admin/users",
            Some(token),
            &CreateUserRequest {
                username: username.to_string(),
                password: password.to_string(),
                roles,
            },
        )
        .await
    }

    pub async fn assign_user_roles(
        &self,
        token: &str,
        user_id: i32,
        roles: Vec<String>,
    ) -> Result<(), String> {
        self.post_json(
            &format!("/admin/users/{user_id}/roles"),
            Some(token),
            &AssignRolesRequest { roles },
        )
        .await
    }

    pub async fn list_roles(&self, token: &str) -> Result<RoleListResponse, String> {
        self.get("/admin/roles", Some(token)).await
    }

    pub async fn create_role(
        &self,
        token: &str,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(), String> {
        self.post_json(
            "/admin/roles",
            Some(token),
            &CreateRoleRequest {
                name: name.to_string(),
                permissions,
            },
        )
        .await
    }

    pub async fn list_permissions(&self, token: &str) -> Result<PermissionListResponse, String> {
        self.get("/admin/permissions", Some(token)).await
    }

    // --- exchanges + own credentials ---

    pub async fn list_exchanges(&self, token: &str) -> Result<ExchangeListResponse, String> {
        self.get("/exchanges", Some(token)).await
    }

    pub async fn list_exchange_pairs(
        &self,
        token: &str,
        exchange: &str,
    ) -> Result<PairListResponse, String> {
        self.get(&format!("/exchanges/{exchange}/pairs"), Some(token))
            .await
    }

    pub async fn list_own_credentials(
        &self,
        token: &str,
    ) -> Result<CredentialListResponse, String> {
        self.get("/exchanges/credentials", Some(token)).await
    }

    pub async fn set_own_credentials(
        &self,
        token: &str,
        exchange: &str,
        credentials: serde_json::Value,
    ) -> Result<(), String> {
        self.post_json(
            &format!("/exchanges/{exchange}/credentials"),
            Some(token),
            &credentials,
        )
        .await
    }

    pub async fn delete_own_credentials(&self, token: &str, exchange: &str) -> Result<(), String> {
        self.delete(&format!("/exchanges/{exchange}/credentials"), token)
            .await
    }

    // --- candles ---

    /// `GET /v1/candles?exchange=&pair=&interval=&limit=` — recent in-memory
    /// candle history for the chart page (see `engine`'s `candle_handler.rs`).
    pub async fn list_candles(
        &self,
        token: &str,
        exchange: &str,
        pair: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<CandleItem>, String> {
        self.get(
            &candle_history_path(exchange, pair, interval, limit),
            Some(token),
        )
        .await
    }
}

/// A free function so the query string shape is testable without the network.
fn candle_history_path(exchange: &str, pair: &str, interval: &str, limit: u32) -> String {
    format!("/candles?exchange={exchange}&pair={pair}&interval={interval}&limit={limit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_url_builds_start_ticker_path() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.v1("/exchanges/futures/start_kline_symbol_ticker"),
            "http://localhost:8080/v1/exchanges/futures/start_kline_symbol_ticker"
        );
    }

    #[test]
    fn v1_url_builds_stop_ticker_path() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.v1("/exchanges/futures/stop_kline_symbol_ticker"),
            "http://localhost:8080/v1/exchanges/futures/stop_kline_symbol_ticker"
        );
    }

    #[test]
    fn trailing_slash_on_base_url_is_stripped() {
        let client = ApiClient::new("http://localhost:8080/");
        assert_eq!(
            client.v1("/exchanges/futures/start_kline_symbol_ticker"),
            "http://localhost:8080/v1/exchanges/futures/start_kline_symbol_ticker"
        );
    }

    #[test]
    fn ws_url_upgrades_http_to_ws() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(client.ws_url(), "ws://localhost:8080/v1/ws");
    }

    #[test]
    fn ws_url_upgrades_https_to_wss() {
        let client = ApiClient::new("https://stream-coin.example.com");
        assert_eq!(client.ws_url(), "wss://stream-coin.example.com/v1/ws");
    }

    #[test]
    fn ws_url_with_token_appends_token_query_param() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.ws_url_with_token("abc.def.ghi"),
            "ws://localhost:8080/v1/ws?token=abc.def.ghi"
        );
    }

    #[test]
    fn v1_url_builds_exchanges_path() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.v1("/exchanges"),
            "http://localhost:8080/v1/exchanges"
        );
    }

    #[test]
    fn v1_url_builds_admin_users_path() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.v1("/admin/users"),
            "http://localhost:8080/v1/admin/users"
        );
    }

    #[test]
    fn v1_url_builds_credentials_path_with_exchange_name() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.v1("/exchanges/hitobit/credentials"),
            "http://localhost:8080/v1/exchanges/hitobit/credentials"
        );
    }

    #[test]
    fn start_ticker_request_attaches_bearer_token() {
        let client = ApiClient::new("http://localhost:8080");
        let req = reqwest::Client::new()
            .post(client.v1("/exchanges/futures/start_kline_symbol_ticker"))
            .bearer_auth("test-token")
            .json(&serde_json::json!({ "exchange": "tabdeal", "symbol": "USDT/IRT" }))
            .build()
            .unwrap();

        let header = req
            .headers()
            .get("authorization")
            .expect("start ticker request must carry an authorization header")
            .to_str()
            .unwrap();

        assert_eq!(header, "Bearer test-token");
    }

    /// The engine's `TradingPair` deserializer (`engine/src/price/entity.rs`)
    /// rejects any symbol without exactly one `/` separator — sending the
    /// slash-stripped concatenated form (e.g. `USDTIRT`) is a 400, not a
    /// shorthand the backend accepts.
    #[test]
    fn ticker_request_body_keeps_the_base_quote_slash_separator() {
        let body = ticker_request_body("tabdeal", "USDT/IRT");
        assert_eq!(body["symbol"], "USDT/IRT");
    }

    #[test]
    fn ticker_request_body_does_not_strip_the_slash() {
        let body = ticker_request_body("tabdeal", "USDT/IRT");
        assert_ne!(body["symbol"], "USDTIRT");
    }

    #[test]
    fn authenticated_request_sets_bearer_header() {
        let req = reqwest::Client::new()
            .get("http://localhost:8080/v1/strategies")
            .bearer_auth("test-token")
            .build()
            .unwrap();

        let header = req
            .headers()
            .get("authorization")
            .expect("authorization header must be set")
            .to_str()
            .unwrap();

        assert_eq!(header, "Bearer test-token");
    }

    #[test]
    fn unwrap_envelope_returns_data_on_success() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"success":true,"message":"ok","data":{"token":"abc","expires_in":86400}}"#,
        )
        .unwrap();
        let data = ApiClient::unwrap_envelope(body).unwrap();
        let token: TokenResponse = serde_json::from_value(data).unwrap();
        assert_eq!(token.token, "abc");
    }

    #[test]
    fn candle_history_path_builds_query_string_with_all_params() {
        assert_eq!(
            candle_history_path("tabdeal", "USDT/IRT", "1m", 300),
            "/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m&limit=300"
        );
    }

    #[test]
    fn v1_url_builds_candles_path() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.v1("/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m&limit=300"),
            "http://localhost:8080/v1/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m&limit=300"
        );
    }

    #[test]
    fn handle_response_status_invokes_unauthorized_handler_on_401() {
        let called = std::rc::Rc::new(std::cell::RefCell::new(false));
        let called_inner = called.clone();
        let client = ApiClient::new("http://localhost:8080")
            .with_unauthorized_handler(move || *called_inner.borrow_mut() = true);

        let result = client.handle_response_status(reqwest::StatusCode::UNAUTHORIZED);

        assert_eq!(result, Err(UNAUTHORIZED_ERROR.to_string()));
        assert!(*called.borrow());
    }

    #[test]
    fn handle_response_status_does_not_invoke_handler_on_success() {
        let called = std::rc::Rc::new(std::cell::RefCell::new(false));
        let called_inner = called.clone();
        let client = ApiClient::new("http://localhost:8080")
            .with_unauthorized_handler(move || *called_inner.borrow_mut() = true);

        let result = client.handle_response_status(reqwest::StatusCode::OK);

        assert!(result.is_ok());
        assert!(!*called.borrow());
    }

    #[test]
    fn handle_response_status_returns_unauthorized_error_with_no_handler_registered() {
        let client = ApiClient::new("http://localhost:8080");

        let result = client.handle_response_status(reqwest::StatusCode::UNAUTHORIZED);

        assert_eq!(result, Err(UNAUTHORIZED_ERROR.to_string()));
    }

    #[test]
    fn unwrap_envelope_returns_message_as_err_when_unsuccessful() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"success":false,"message":"Invalid credentials","errors":[]}"#,
        )
        .unwrap();
        assert_eq!(
            ApiClient::unwrap_envelope(body),
            Err("Invalid credentials".to_string())
        );
    }
}
