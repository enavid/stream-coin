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

#[derive(Debug, Deserialize, ToSchema)]
pub struct SeedTopPairsQuery {
    pub count: Option<u32>,
}

impl SeedTopPairsQuery {
    /// Clamps the requested count into `1..=MAX_SEED_COUNT`, defaulting to
    /// `DEFAULT_SEED_COUNT` when absent.
    pub fn resolved_count(&self) -> usize {
        self.count
            .map(|c| (c as usize).clamp(1, MAX_SEED_COUNT))
            .unwrap_or(DEFAULT_SEED_COUNT)
    }
}

pub const DEFAULT_SEED_COUNT: usize = 20;
pub const MAX_SEED_COUNT: usize = 100;

#[derive(Debug, Serialize, ToSchema)]
pub struct SeedTopPairsResponse {
    pub pairs_seeded: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_count_defaults_to_twenty() {
        let q = SeedTopPairsQuery { count: None };
        assert_eq!(q.resolved_count(), 20);
    }

    #[test]
    fn resolved_count_clamps_to_max() {
        let q = SeedTopPairsQuery { count: Some(5_000) };
        assert_eq!(q.resolved_count(), MAX_SEED_COUNT);
    }

    #[test]
    fn resolved_count_clamps_to_one_when_zero() {
        let q = SeedTopPairsQuery { count: Some(0) };
        assert_eq!(q.resolved_count(), 1);
    }

    #[test]
    fn resolved_count_passes_through_valid_value() {
        let q = SeedTopPairsQuery { count: Some(10) };
        assert_eq!(q.resolved_count(), 10);
    }
}
