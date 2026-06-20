use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct StartStrategyRequest {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
    pub params: serde_json::Value,
}

#[derive(Deserialize)]
pub struct StopStrategyRequest {
    pub strategy_id: String,
}

#[derive(Deserialize)]
pub struct RegisterStrategyRequest {
    pub strategy_id: String,
    pub name: String,
    pub strategy_type: String,
}

#[derive(Serialize)]
pub struct ActiveStrategy {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
}

#[derive(Serialize)]
pub struct StrategyList {
    pub strategies: Vec<ActiveStrategy>,
}
