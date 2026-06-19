use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::exchange::entity::ExchangeId;
use crate::price::entity::TradingPair;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SymbolRequest {
    #[schema(value_type = String, example = "tabdeal")]
    pub exchange: ExchangeId,
    #[schema(value_type = String, example = "USDT/IRT")]
    pub symbol: TradingPair,
}

#[derive(Serialize, ToSchema)]
pub struct TickerStarted {
    pub exchange: String,
    pub pair: String,
}

#[derive(Serialize, ToSchema)]
pub struct TickerStopped {
    pub exchange: String,
    pub pair: String,
}

#[derive(Serialize, ToSchema)]
pub struct ActiveTicker {
    pub exchange: String,
    pub pair: String,
}

#[derive(Serialize, ToSchema)]
pub struct TickerList {
    pub tickers: Vec<ActiveTicker>,
}
