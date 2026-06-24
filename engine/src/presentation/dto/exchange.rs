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

/// Query for `POST /v1/admin/exchanges/{name}/seed-from-assets`. `quotes` is
/// a comma-separated list of quote-currency symbols, e.g. `?quotes=USDT,IRT`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SeedPairsQuery {
    pub quotes: Option<String>,
}

impl SeedPairsQuery {
    pub const DEFAULT_QUOTE: &'static str = "USDT";

    /// Splits the comma-separated `quotes` param into a list, defaulting to
    /// `[DEFAULT_QUOTE]` when absent or empty.
    pub fn resolved_quotes(&self) -> Vec<String> {
        match &self.quotes {
            Some(s) if !s.trim().is_empty() => s.split(',').map(|q| q.trim().to_string()).collect(),
            _ => vec![Self::DEFAULT_QUOTE.to_string()],
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SeedPairsResponse {
    pub pairs_seeded: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_quotes_defaults_to_usdt_when_absent() {
        let q = SeedPairsQuery { quotes: None };
        assert_eq!(q.resolved_quotes(), vec!["USDT".to_string()]);
    }

    #[test]
    fn resolved_quotes_defaults_to_usdt_when_empty_string() {
        let q = SeedPairsQuery {
            quotes: Some("".to_string()),
        };
        assert_eq!(q.resolved_quotes(), vec!["USDT".to_string()]);
    }

    #[test]
    fn resolved_quotes_splits_comma_separated_list() {
        let q = SeedPairsQuery {
            quotes: Some("USDT,IRT".to_string()),
        };
        assert_eq!(
            q.resolved_quotes(),
            vec!["USDT".to_string(), "IRT".to_string()]
        );
    }

    #[test]
    fn resolved_quotes_trims_whitespace_around_each_symbol() {
        let q = SeedPairsQuery {
            quotes: Some(" USDT , IRT ".to_string()),
        };
        assert_eq!(
            q.resolved_quotes(),
            vec!["USDT".to_string(), "IRT".to_string()]
        );
    }
}
