use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct StartStrategyRequest {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
    #[schema(value_type = Object)]
    pub params: serde_json::Value,
}

#[derive(Deserialize, ToSchema)]
pub struct StopStrategyRequest {
    pub strategy_id: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RegisterStrategyRequest {
    pub strategy_id: String,
    pub name: String,
    pub strategy_type: String,
}

#[derive(Serialize, ToSchema)]
pub struct ActiveStrategy {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
}

#[derive(Serialize, ToSchema)]
pub struct StrategyList {
    pub strategies: Vec<ActiveStrategy>,
}

#[derive(Deserialize, ToSchema)]
pub struct DeployStrategyRequest {
    /// Human-readable name for this strategy.
    pub name: String,
    /// Complete Python strategy code. Must read candle JSON from stdin and write
    /// signal JSON to stdout (one per line). The engine prepends the seccomp preamble.
    pub code: String,
    /// Strategy-specific parameters passed via the `STRATEGY_PARAMS` env variable.
    #[serde(default = "serde_json::Value::default")]
    pub params: serde_json::Value,
}

#[derive(Serialize, ToSchema)]
pub struct DeployedStrategy {
    pub strategy_id: String,
    pub name: String,
}
