use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::candle::entity::Interval;

/// Default and maximum candle counts for `GET /v1/candles?limit=`.
pub const DEFAULT_CANDLE_LIMIT: usize = 300;
pub const MAX_CANDLE_LIMIT: usize = 1000;

#[derive(Debug, Deserialize, ToSchema)]
pub struct BackfillRequest {
    pub exchange: String,
    /// `"BASE/QUOTE"`, e.g. `"BTC/USDT"`.
    pub pair: String,
    pub interval: Interval,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BackfillResponse {
    pub candles_written: usize,
}

#[derive(Debug, Deserialize)]
pub struct CandleHistoryQuery {
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub limit: Option<u32>,
}

impl CandleHistoryQuery {
    /// Clamps the requested limit into `1..=MAX_CANDLE_LIMIT`, defaulting to
    /// `DEFAULT_CANDLE_LIMIT` when absent — keeps an unbounded `?limit=` query
    /// from forcing the handler to clone an arbitrarily large history vector.
    pub fn resolved_limit(&self) -> usize {
        self.limit
            .map(|l| (l as usize).clamp(1, MAX_CANDLE_LIMIT))
            .unwrap_or(DEFAULT_CANDLE_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(limit: Option<u32>) -> CandleHistoryQuery {
        CandleHistoryQuery {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            limit,
        }
    }

    #[test]
    fn resolved_limit_defaults_when_absent() {
        assert_eq!(query(None).resolved_limit(), DEFAULT_CANDLE_LIMIT);
    }

    #[test]
    fn resolved_limit_clamps_to_max_when_too_large() {
        assert_eq!(query(Some(5_000)).resolved_limit(), MAX_CANDLE_LIMIT);
    }

    #[test]
    fn resolved_limit_clamps_to_one_when_zero() {
        assert_eq!(query(Some(0)).resolved_limit(), 1);
    }

    #[test]
    fn resolved_limit_passes_through_valid_value() {
        assert_eq!(query(Some(50)).resolved_limit(), 50);
    }
}
