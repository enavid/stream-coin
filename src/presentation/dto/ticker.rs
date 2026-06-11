use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SymbolRequest {
    pub exchange: String,
    pub symbol: String,
}

#[derive(Serialize, ToSchema)]
pub struct TickerStarted {
    pub exchange: String,
    pub symbol: String,
}
