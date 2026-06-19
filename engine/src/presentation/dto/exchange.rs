use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::price::entity::MarketType;

#[derive(Serialize, ToSchema)]
pub struct ExchangeResponse {
    pub name: String,
    pub display_name: String,
    pub enabled: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ExchangeListResponse {
    pub exchanges: Vec<ExchangeResponse>,
}

#[derive(Serialize, ToSchema)]
pub struct PairResponse {
    pub base: String,
    pub quote: String,
    #[schema(value_type = String, example = "spot")]
    pub market_type: MarketType,
    pub active: bool,
}

#[derive(Serialize, ToSchema)]
pub struct PairListResponse {
    pub pairs: Vec<PairResponse>,
}

#[derive(Deserialize, ToSchema)]
pub struct ExchangeNameRequest {
    pub exchange: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PairListQuery {
    pub market_type: Option<MarketType>,
}
