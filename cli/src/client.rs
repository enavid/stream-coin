use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::config::Config;
use crate::response::{
    ApiError, ApiSuccess, BackfillData, SeedPairsData, TickerData, TickerListData,
};

pub struct ApiClient {
    inner: Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(config: &Config) -> Self {
        Self {
            inner: Client::new(),
            base_url: config.server.url.trim_end_matches('/').to_string(),
            token: config.auth.token.clone(),
        }
    }

    pub fn ticker_start_url(&self) -> String {
        format!(
            "{}/v1/exchanges/futures/start_kline_symbol_ticker",
            self.base_url
        )
    }

    pub fn ticker_stop_url(&self) -> String {
        format!(
            "{}/v1/exchanges/futures/stop_kline_symbol_ticker",
            self.base_url
        )
    }

    pub fn ticker_list_url(&self) -> String {
        format!("{}/v1/exchanges/futures/tickers", self.base_url)
    }

    pub fn candle_backfill_url(&self) -> String {
        format!("{}/v1/candles/backfill", self.base_url)
    }

    pub fn exchange_seed_from_assets_url(&self, exchange: &str, quotes: &str) -> String {
        format!(
            "{}/v1/admin/exchanges/{exchange}/seed-from-assets?quotes={quotes}",
            self.base_url
        )
    }

    async fn post(&self, url: &str, body: Value) -> Result<Value, String> {
        let mut req = self.inner.post(url).json(&body);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }

    async fn get(&self, url: &str) -> Result<Value, String> {
        let mut req = self.inner.get(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }

    pub async fn ticker_start(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<ApiSuccess<TickerData>, ApiError> {
        let value = self
            .post(
                &self.ticker_start_url(),
                serde_json::json!({ "exchange": exchange, "symbol": symbol }),
            )
            .await
            .map_err(|e| ApiError {
                success: false,
                message: e,
                errors: vec![],
            })?;
        parse_response(value)
    }

    pub async fn ticker_stop(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<ApiSuccess<TickerData>, ApiError> {
        let value = self
            .post(
                &self.ticker_stop_url(),
                serde_json::json!({ "exchange": exchange, "symbol": symbol }),
            )
            .await
            .map_err(|e| ApiError {
                success: false,
                message: e,
                errors: vec![],
            })?;
        parse_response(value)
    }

    pub async fn ticker_list(&self) -> Result<ApiSuccess<TickerListData>, ApiError> {
        let value = self
            .get(&self.ticker_list_url())
            .await
            .map_err(|e| ApiError {
                success: false,
                message: e,
                errors: vec![],
            })?;
        parse_response(value)
    }

    pub async fn exchange_seed_from_assets(
        &self,
        exchange: &str,
        quotes: &str,
    ) -> Result<ApiSuccess<SeedPairsData>, ApiError> {
        let value = self
            .post(
                &self.exchange_seed_from_assets_url(exchange, quotes),
                Value::Null,
            )
            .await
            .map_err(|e| ApiError {
                success: false,
                message: e,
                errors: vec![],
            })?;
        parse_response(value)
    }

    /// `from`/`to` are passed through verbatim as RFC3339 timestamps — the
    /// engine validates and parses them; the CLI does not duplicate that logic.
    pub async fn candle_backfill(
        &self,
        exchange: &str,
        pair: &str,
        interval: &str,
        from: &str,
        to: &str,
    ) -> Result<ApiSuccess<BackfillData>, ApiError> {
        let value = self
            .post(
                &self.candle_backfill_url(),
                serde_json::json!({
                    "exchange": exchange,
                    "pair": pair,
                    "interval": interval,
                    "from": from,
                    "to": to,
                }),
            )
            .await
            .map_err(|e| ApiError {
                success: false,
                message: e,
                errors: vec![],
            })?;
        parse_response(value)
    }
}

fn parse_response<T: DeserializeOwned>(value: Value) -> Result<ApiSuccess<T>, ApiError> {
    if value["success"].as_bool().unwrap_or(false) {
        serde_json::from_value(value).map_err(|e| ApiError {
            success: false,
            message: e.to_string(),
            errors: vec![],
        })
    } else {
        Err(serde_json::from_value(value).unwrap_or_else(|_| ApiError {
            success: false,
            message: "unexpected response format".to_string(),
            errors: vec![],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_client(url: &str) -> ApiClient {
        let mut config = crate::config::Config::default();
        config.set_url(url);
        ApiClient::new(&config)
    }

    #[test]
    fn client_ticker_start_url_uses_base_url() {
        let client = make_client("http://localhost:8080");
        assert_eq!(
            client.ticker_start_url(),
            "http://localhost:8080/v1/exchanges/futures/start_kline_symbol_ticker"
        );
    }

    #[test]
    fn client_ticker_stop_url_uses_base_url() {
        let client = make_client("http://localhost:8080");
        assert_eq!(
            client.ticker_stop_url(),
            "http://localhost:8080/v1/exchanges/futures/stop_kline_symbol_ticker"
        );
    }

    #[test]
    fn client_ticker_list_url_uses_base_url() {
        let client = make_client("http://localhost:8080");
        assert_eq!(
            client.ticker_list_url(),
            "http://localhost:8080/v1/exchanges/futures/tickers"
        );
    }

    #[test]
    fn client_strips_trailing_slash_from_base_url() {
        let client = make_client("http://localhost:8080/");
        assert_eq!(
            client.ticker_list_url(),
            "http://localhost:8080/v1/exchanges/futures/tickers"
        );
    }

    #[test]
    fn client_candle_backfill_url_uses_base_url() {
        let client = make_client("http://localhost:8080");
        assert_eq!(
            client.candle_backfill_url(),
            "http://localhost:8080/v1/candles/backfill"
        );
    }

    #[test]
    fn client_exchange_seed_from_assets_url_includes_exchange_and_quotes() {
        let client = make_client("http://localhost:8080");
        assert_eq!(
            client.exchange_seed_from_assets_url("coinex", "USDT"),
            "http://localhost:8080/v1/admin/exchanges/coinex/seed-from-assets?quotes=USDT"
        );
    }

    #[test]
    fn client_uses_custom_server_url() {
        let client = make_client("http://prod.example.com:9000");
        assert!(client
            .ticker_start_url()
            .starts_with("http://prod.example.com:9000"));
    }

    #[test]
    fn parse_response_success_extracts_data() {
        let value = serde_json::json!({
            "success": true,
            "message": "Ticker started",
            "data": { "exchange": "tabdeal", "pair": "USDT/IRT" }
        });
        let result: Result<ApiSuccess<TickerData>, _> = parse_response(value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().data.pair, "USDT/IRT");
    }

    #[test]
    fn parse_response_error_returns_api_error() {
        let value = serde_json::json!({
            "success": false,
            "message": "Exchange not supported",
            "errors": [{ "field": "exchange", "message": "unsupported exchange" }]
        });
        let result: Result<ApiSuccess<TickerData>, _> = parse_response(value);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().message, "Exchange not supported");
    }

    #[test]
    fn parse_response_error_carries_field_errors() {
        let value = serde_json::json!({
            "success": false,
            "message": "Validation failed",
            "errors": [{ "field": "symbol", "message": "must be BASE/QUOTE format" }]
        });
        let result: Result<ApiSuccess<TickerData>, _> = parse_response(value);
        let err = result.unwrap_err();
        assert_eq!(err.errors[0].field, "symbol");
    }

    #[test]
    fn parse_response_backfill_success_extracts_candles_written() {
        let value = serde_json::json!({
            "success": true,
            "message": "Backfill complete",
            "data": { "candles_written": 7 }
        });
        let result: Result<ApiSuccess<BackfillData>, _> = parse_response(value);
        assert_eq!(result.unwrap().data.candles_written, 7);
    }

    #[test]
    fn parse_response_backfill_error_for_unsupported_exchange() {
        let value = serde_json::json!({
            "success": false,
            "message": "exchange 'tabdeal' has no historical candle source",
            "errors": []
        });
        let result: Result<ApiSuccess<BackfillData>, _> = parse_response(value);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .message
            .contains("historical candle source"));
    }

    #[test]
    fn parse_response_seed_pairs_success_extracts_count() {
        let value = serde_json::json!({
            "success": true,
            "message": "Pairs seeded",
            "data": { "pairs_seeded": 20 }
        });
        let result: Result<ApiSuccess<SeedPairsData>, _> = parse_response(value);
        assert_eq!(result.unwrap().data.pairs_seeded, 20);
    }
}
