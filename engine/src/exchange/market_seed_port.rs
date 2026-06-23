use async_trait::async_trait;
use thiserror::Error;

use crate::exchange::entity::ExchangeId;

/// One market's 24h quote-currency volume, as reported by an exchange's
/// public ticker endpoint. `status` defaults to `"online"` for exchanges
/// whose ticker endpoint only ever lists tradable markets; it exists so the
/// ranking step can still defensively exclude a delisted market if a future
/// source ever reports one.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketVolume {
    pub market: String,
    pub quote_volume: u64,
    pub status: String,
}

/// Errors produced by a `TopMarketSource`.
///
/// Classified transient/permanent per the project's error-handling rule —
/// callers must never retry a permanent error.
#[derive(Debug, Error)]
pub enum MarketSeederError {
    #[error("network timeout: {0}")]
    NetworkTimeout(String),

    #[error("server error: status={status}, body={body}")]
    ServerError { status: u16, body: String },

    #[error("client error: status={status}, body={body}")]
    ClientError { status: u16, body: String },

    #[error("serialization error: {0}")]
    Serialization(String),
}

impl MarketSeederError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::NetworkTimeout(_) | Self::ServerError { .. })
    }
}

/// Filters out non-"online" markets and sorts the remainder by quote-currency
/// volume descending. Quote volume (not raw base-asset `volume`) is the
/// comparable ranking metric across markets with different base assets —
/// the same metric CoinEx's own site ranks by.
pub fn rank_markets_by_quote_volume(markets: &[MarketVolume]) -> Vec<MarketVolume> {
    let mut ranked: Vec<MarketVolume> = markets
        .iter()
        .filter(|m| m.status == "online")
        .cloned()
        .collect();
    ranked.sort_by(|a, b| b.quote_volume.cmp(&a.quote_volume));
    ranked
}

/// Port for ranking and selecting an exchange's top markets by traded
/// volume. Separate from `HistoricalCandleSource`/`ExchangeAdapter`: this is
/// a one-shot admin action over a ticker/volume endpoint, not a streaming or
/// kline capability, and not every exchange exposes one.
#[async_trait]
pub trait TopMarketSource: Send + Sync {
    fn exchange_id(&self) -> ExchangeId;

    /// Fetches and ranks all markets, returning the top `count` by quote volume.
    async fn fetch_top_markets(&self, count: usize)
        -> Result<Vec<MarketVolume>, MarketSeederError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market(name: &str, quote_volume: u64, status: &str) -> MarketVolume {
        MarketVolume {
            market: name.to_string(),
            quote_volume,
            status: status.to_string(),
        }
    }

    #[test]
    fn rank_markets_by_quote_volume_descending() {
        let markets = vec![
            market("ETHUSDT", 500, "online"),
            market("BTCUSDT", 1000, "online"),
            market("XRPUSDT", 100, "online"),
        ];
        let ranked = rank_markets_by_quote_volume(&markets);
        assert_eq!(
            ranked.iter().map(|m| m.market.as_str()).collect::<Vec<_>>(),
            vec!["BTCUSDT", "ETHUSDT", "XRPUSDT"]
        );
    }

    #[test]
    fn rank_markets_excludes_non_online_status() {
        let markets = vec![
            market("BTCUSDT", 1000, "online"),
            market("DELISTEDUSDT", 9_999_999, "delisted"),
        ];
        let ranked = rank_markets_by_quote_volume(&markets);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].market, "BTCUSDT");
    }

    #[test]
    fn rank_markets_empty_input_returns_empty() {
        assert!(rank_markets_by_quote_volume(&[]).is_empty());
    }

    #[test]
    fn rank_markets_all_delisted_returns_empty() {
        let markets = vec![
            market("A", 100, "delisted"),
            market("B", 200, "maintenance"),
        ];
        assert!(rank_markets_by_quote_volume(&markets).is_empty());
    }

    #[test]
    fn rank_markets_ties_preserve_relative_order() {
        let markets = vec![
            market("AAAUSDT", 500, "online"),
            market("BBBUSDT", 500, "online"),
        ];
        let ranked = rank_markets_by_quote_volume(&markets);
        assert_eq!(ranked[0].market, "AAAUSDT");
        assert_eq!(ranked[1].market, "BBBUSDT");
    }

    #[test]
    fn market_seeder_error_server_error_is_transient() {
        let err = MarketSeederError::ServerError {
            status: 503,
            body: "unavailable".to_string(),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn market_seeder_error_client_error_is_not_transient() {
        let err = MarketSeederError::ClientError {
            status: 400,
            body: "bad request".to_string(),
        };
        assert!(!err.is_transient());
    }
}
