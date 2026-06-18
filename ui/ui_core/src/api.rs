//! Thin REST client for the control-plane endpoints (start/stop a
//! ticker). Live price data itself arrives over the WebSocket feed, not
//! through this client — see [`crate::protocol`].
//!
//! `reqwest` compiles for both `wasm32` (via `fetch`) and native targets,
//! so this client works unmodified across web, desktop, and mobile.

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub fn start_url(&self) -> String {
        format!(
            "{}/v1/exchanges/futures/start_kline_symbol_ticker",
            self.base_url
        )
    }

    pub fn stop_url(&self) -> String {
        format!(
            "{}/v1/exchanges/futures/stop_kline_symbol_ticker",
            self.base_url
        )
    }

    pub fn ws_url(&self) -> String {
        let base = self
            .base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{base}/v1/ws")
    }

    pub async fn start_ticker(&self, exchange: &str, symbol: &str) -> Result<(), String> {
        self.post(&self.start_url(), exchange, symbol).await
    }

    /// `pair` is the display form (e.g. `USDT/IRT`); the backend's stop
    /// endpoint expects the raw symbol it was started with (`USDTIRT`),
    /// which for every current adapter is just the pair without the
    /// separating slash.
    pub async fn stop_ticker(&self, exchange: &str, pair: &str) -> Result<(), String> {
        let symbol = pair.replace('/', "");
        self.post(&self.stop_url(), exchange, &symbol).await
    }

    async fn post(&self, url: &str, exchange: &str, symbol: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(url)
            .json(&serde_json::json!({ "exchange": exchange, "symbol": symbol }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("request failed with status {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_url_is_built_from_base_url() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.start_url(),
            "http://localhost:8080/v1/exchanges/futures/start_kline_symbol_ticker"
        );
    }

    #[test]
    fn stop_url_is_built_from_base_url() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.stop_url(),
            "http://localhost:8080/v1/exchanges/futures/stop_kline_symbol_ticker"
        );
    }

    #[test]
    fn trailing_slash_on_base_url_is_stripped() {
        let client = ApiClient::new("http://localhost:8080/");
        assert_eq!(
            client.start_url(),
            "http://localhost:8080/v1/exchanges/futures/start_kline_symbol_ticker"
        );
    }

    #[test]
    fn ws_url_upgrades_http_to_ws() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(client.ws_url(), "ws://localhost:8080/v1/ws");
    }

    #[test]
    fn ws_url_upgrades_https_to_wss() {
        let client = ApiClient::new("https://stream-coin.example.com");
        assert_eq!(client.ws_url(), "wss://stream-coin.example.com/v1/ws");
    }
}
