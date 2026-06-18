use reqwest::Client;
use serde_json::Value;

use crate::config::Config;

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

    pub async fn ticker_start(&self, exchange: &str, symbol: &str) -> Result<Value, String> {
        self.post(
            &self.ticker_start_url(),
            serde_json::json!({ "exchange": exchange, "symbol": symbol }),
        )
        .await
    }

    pub async fn ticker_stop(&self, exchange: &str, symbol: &str) -> Result<Value, String> {
        self.post(
            &self.ticker_stop_url(),
            serde_json::json!({ "exchange": exchange, "symbol": symbol }),
        )
        .await
    }

    pub async fn ticker_list(&self) -> Result<Value, String> {
        self.get(&self.ticker_list_url()).await
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
    fn client_uses_custom_server_url() {
        let client = make_client("http://prod.example.com:9000");
        assert!(client
            .ticker_start_url()
            .starts_with("http://prod.example.com:9000"));
    }
}
