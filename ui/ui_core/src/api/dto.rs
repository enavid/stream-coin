//! Hand-mirrored request/response shapes for the engine's REST control
//! plane. Same approach as [`crate::protocol::PriceMessage`]: the backend
//! can't compile to `wasm32` (it links `rdkafka`/`redis` native libs), so
//! the contract is kept here rather than shared via a crate dependency.
//! Source of truth: `engine/src/presentation/dto/*.rs`.

use serde::{Deserialize, Serialize};

// --- auth ---

#[derive(Debug, Serialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct TokenResponse {
    pub token: String,
    pub expires_in: u64,
}

// --- strategies ---

#[derive(Debug, Serialize)]
pub struct StartStrategyRequest {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct StopStrategyRequest {
    pub strategy_id: String,
}

#[derive(Debug, Serialize)]
pub struct DeployStrategyRequest {
    pub name: String,
    pub code: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct DeployedStrategy {
    pub strategy_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ActiveStrategy {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StrategyList {
    pub strategies: Vec<ActiveStrategy>,
}

// --- backtest ---

#[derive(Debug, Serialize)]
pub struct BacktestRunRequest {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    /// RFC3339, e.g. `2026-05-01T00:00:00Z` — callers format the date
    /// input themselves; this client does not depend on `chrono`.
    pub from: String,
    pub to: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TradeRecord {
    pub order_id: String,
    pub side: String,
    pub quantity: u64,
    pub fill_price: u64,
    pub strategy_id: String,
    pub candle_time: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BacktestSignalRecord {
    pub signal_id: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub timestamp: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BacktestResult {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub candle_count: usize,
    pub signal_count: usize,
    pub total_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub trade_log: Vec<TradeRecord>,
    pub signal_log: Vec<BacktestSignalRecord>,
}

// --- orders ---

#[derive(Debug, Serialize)]
pub struct PlaceOrderRequest {
    pub exchange: String,
    pub pair: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub quantity: String,
    pub price: Option<String>,
    pub strategy_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CancelOrderRequest {
    pub client_order_id: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct OrderPlacedResponse {
    pub client_order_id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OrderItem {
    pub id: i64,
    pub exchange: String,
    pub pair: String,
    pub side: String,
    pub order_type: String,
    pub quantity: String,
    pub price: Option<String>,
    pub status: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: String,
    pub strategy_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct OrderListResponse {
    pub orders: Vec<OrderItem>,
}

// --- admin: users / roles / permissions ---

#[derive(Debug, Serialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct UserResponse {
    pub id: i32,
    pub username: String,
    pub roles: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct UserListResponse {
    pub users: Vec<UserResponse>,
}

#[derive(Debug, Serialize)]
pub struct AssignRolesRequest {
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RoleResponse {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct RoleListResponse {
    pub roles: Vec<RoleResponse>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct PermissionListResponse {
    pub permissions: Vec<String>,
}

// --- exchanges + own credentials ---

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ExchangeResponse {
    pub name: String,
    pub display_name: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct ExchangeListResponse {
    pub exchanges: Vec<ExchangeResponse>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PairResponse {
    pub base: String,
    pub quote: String,
    pub market_type: String,
    pub active: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct PairListResponse {
    pub pairs: Vec<PairResponse>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CredentialSummaryResponse {
    pub exchange: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct CredentialListResponse {
    pub credentials: Vec<CredentialSummaryResponse>,
}
