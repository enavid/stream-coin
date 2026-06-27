use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct BacktestRunRequest {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: serde_json::Value,
}
