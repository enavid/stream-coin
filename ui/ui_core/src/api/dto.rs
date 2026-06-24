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

#[derive(Debug, Clone, Serialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, PartialEq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TradeSide {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TradeOutcome {
    Win,
    Loss,
    Breakeven,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClosedTrade {
    pub strategy_id: String,
    pub side: TradeSide,
    pub entry_price: u64,
    pub exit_price: u64,
    pub stop_loss: Option<u64>,
    pub take_profit: Option<u64>,
    pub quantity: u64,
    pub entry_time: String,
    pub exit_time: String,
    pub pnl: i64,
    pub pnl_pct: f64,
    pub rr: Option<f64>,
    pub outcome: TradeOutcome,
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
    pub closed_trades: Vec<ClosedTrade>,
    pub win_rate: f64,
    pub avg_rr: Option<f64>,
}

#[cfg(test)]
mod backtest_result_tests {
    use super::*;

    #[test]
    fn backtest_result_dto_deserializes_closed_trades_from_engine_payload() {
        let json = r#"{"strategy_id":"s1","exchange":"tabdeal","pair":"USDT/IRT","interval":"1m","candle_count":10,"signal_count":2,"total_return_pct":1.5,"max_drawdown_pct":0.2,"trade_log":[],"signal_log":[],"closed_trades":[{"strategy_id":"s1","side":"long","entry_price":100000,"exit_price":110000,"stop_loss":null,"take_profit":null,"quantity":1,"entry_time":"2026-01-01T00:00:00Z","exit_time":"2026-01-01T00:01:00Z","pnl":10000,"pnl_pct":10.0,"rr":null,"outcome":"win"}],"win_rate":1.0,"avg_rr":null}"#;
        let result: BacktestResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.win_rate, 1.0);
        assert_eq!(result.avg_rr, None);
        assert_eq!(result.closed_trades.len(), 1);
        let trade = &result.closed_trades[0];
        assert_eq!(trade.side, TradeSide::Long);
        assert_eq!(trade.outcome, TradeOutcome::Win);
        assert_eq!(trade.entry_price, 100_000);
        assert_eq!(trade.pnl, 10_000);
        assert_eq!(trade.rr, None);
    }
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

// --- candles ---

/// Mirrors `engine`'s `CandlePayload`. `GET /v1/candles` returns its `data`
/// as a bare array (no wrapper struct), unlike most other list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandleItem {
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub time: String,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
    pub volume: u64,
}

/// Mirrors `engine`'s `BackfillRequest` (`engine/src/presentation/dto/
/// candle.rs`). `from`/`to` stay RFC3339 strings, same "never depend on
/// chrono in this client" rule as `BacktestRunRequest`.
#[derive(Debug, Serialize)]
pub struct BackfillRequest {
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BackfillResponse {
    pub candles_written: usize,
}
