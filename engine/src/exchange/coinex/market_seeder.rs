use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::exchange::entity::ExchangeId;
use crate::exchange::market_seed_port::{
    rank_markets_by_quote_volume, MarketSeederError, MarketVolume, TopMarketSource,
};

const BASE_URL: &str = "https://api.coinex.com/v2";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Parses a `GET /spot/ticker` response body (`{"code":0,"data":[...]}`)
/// into `MarketVolume`s. CoinEx's public ticker endpoint only ever lists
/// tradable markets, so every parsed entry gets `status: "online"`.
fn parse_ticker_response(body: &str) -> Result<Vec<MarketVolume>, MarketSeederError> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| MarketSeederError::Serialization(format!("invalid json: {e}")))?;

    let data = v["data"]
        .as_array()
        .ok_or_else(|| MarketSeederError::Serialization("missing data array".into()))?;

    data.iter()
        .map(|item| {
            let market = item["market"]
                .as_str()
                .ok_or_else(|| MarketSeederError::Serialization("missing market field".into()))?
                .to_string();
            let value_str = item["value"]
                .as_str()
                .ok_or_else(|| MarketSeederError::Serialization("missing value field".into()))?;
            let quote_volume =
                super::parse_minor_units(value_str).map_err(MarketSeederError::Serialization)?;
            Ok(MarketVolume {
                market,
                quote_volume,
                status: "online".to_string(),
            })
        })
        .collect()
}

fn classify_http_status(status: u16, body: &str) -> Option<MarketSeederError> {
    match super::classify_http_status(status, body) {
        super::HttpStatusClass::Success => None,
        super::HttpStatusClass::Transient { status, body } => {
            Some(MarketSeederError::ServerError { status, body })
        }
        super::HttpStatusClass::Permanent { status, body } => {
            Some(MarketSeederError::ClientError { status, body })
        }
    }
}

/// Top-market-by-volume source for CoinEx — `GET /spot/ticker`. Public
/// market data endpoint; no API key required.
pub struct CoinexMarketSeeder {
    base_url: String,
    http_client: reqwest::Client,
}

impl CoinexMarketSeeder {
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL)
    }

    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            http_client: reqwest::Client::new(),
        }
    }
}

impl Default for CoinexMarketSeeder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TopMarketSource for CoinexMarketSeeder {
    fn exchange_id(&self) -> ExchangeId {
        ExchangeId::new("coinex")
    }

    async fn fetch_top_markets(
        &self,
        count: usize,
    ) -> Result<Vec<MarketVolume>, MarketSeederError> {
        let url = format!("{}/spot/ticker", self.base_url);

        let response = tokio::time::timeout(REQUEST_TIMEOUT, self.http_client.get(&url).send())
            .await
            .map_err(|_| {
                tracing::error!(%url, "coinex ticker request timed out");
                MarketSeederError::NetworkTimeout("fetch_top_markets timed out".to_string())
            })?
            .map_err(|e| {
                tracing::error!(error = %e, "coinex ticker network error");
                MarketSeederError::NetworkTimeout(e.to_string())
            })?;

        let status = response.status().as_u16();
        let body = response.text().await.map_err(|e| {
            MarketSeederError::Serialization(format!("failed to read response body: {e}"))
        })?;

        if let Some(err) = classify_http_status(status, &body) {
            tracing::warn!(
                status,
                transient = err.is_transient(),
                "coinex ticker request failed"
            );
            return Err(err);
        }

        let markets = parse_ticker_response(&body)?;
        let ranked = rank_markets_by_quote_volume(&markets);

        tracing::info!(
            exchange = "coinex",
            total_markets = markets.len(),
            requested_count = count,
            "ranked coinex markets by quote volume"
        );

        Ok(ranked.into_iter().take(count).collect())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ticker_body(entries: Vec<(&str, &str)>) -> String {
        let data: Vec<Value> = entries
            .into_iter()
            .map(|(market, value)| json!({ "market": market, "value": value, "volume": "1" }))
            .collect();
        json!({ "code": 0, "data": data, "message": "OK" }).to_string()
    }

    #[test]
    fn parse_ticker_response_extracts_market_and_quote_volume() {
        let body = ticker_body(vec![("BTCUSDT", "1000000.50")]);
        let markets = parse_ticker_response(&body).unwrap();
        assert_eq!(markets.len(), 1);
        assert_eq!(markets[0].market, "BTCUSDT");
        assert_eq!(markets[0].quote_volume, 1_000_000);
    }

    #[test]
    fn parse_ticker_response_sets_status_online() {
        let body = ticker_body(vec![("BTCUSDT", "100")]);
        let markets = parse_ticker_response(&body).unwrap();
        assert_eq!(markets[0].status, "online");
    }

    #[test]
    fn parse_ticker_response_missing_data_array_is_error() {
        let body = json!({ "code": 0, "message": "OK" }).to_string();
        assert!(matches!(
            parse_ticker_response(&body),
            Err(MarketSeederError::Serialization(_))
        ));
    }

    #[test]
    fn parse_ticker_response_missing_value_field_is_error() {
        let body = json!({
            "code": 0,
            "data": [{ "market": "BTCUSDT" }],
        })
        .to_string();
        assert!(matches!(
            parse_ticker_response(&body),
            Err(MarketSeederError::Serialization(_))
        ));
    }

    #[test]
    fn parse_ticker_response_negative_value_is_error() {
        let body = ticker_body(vec![("BTCUSDT", "-5")]);
        assert!(matches!(
            parse_ticker_response(&body),
            Err(MarketSeederError::Serialization(_))
        ));
    }

    #[test]
    fn parse_ticker_response_handles_many_markets() {
        let entries: Vec<(&str, &str)> = vec![
            ("AUSDT", "1"),
            ("BUSDT", "2"),
            ("CUSDT", "3"),
            ("DUSDT", "4"),
        ];
        let body = ticker_body(entries);
        let markets = parse_ticker_response(&body).unwrap();
        assert_eq!(markets.len(), 4);
    }

    #[test]
    fn classify_http_status_2xx_returns_none() {
        assert!(classify_http_status(200, "{}").is_none());
    }

    #[test]
    fn classify_http_status_5xx_is_transient() {
        let err = classify_http_status(503, "down").unwrap();
        assert!(err.is_transient());
    }

    #[test]
    fn classify_http_status_4xx_is_permanent() {
        let err = classify_http_status(404, "not found").unwrap();
        assert!(!err.is_transient());
    }

    #[test]
    fn coinex_market_seeder_exchange_id_is_coinex() {
        let seeder = CoinexMarketSeeder::new();
        assert_eq!(seeder.exchange_id().to_string(), "coinex");
    }
}
