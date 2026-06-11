use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait WebSocketClient: Send + Sync {
    async fn connect(&mut self) -> Result<(), String>;
    async fn disconnect(&mut self) -> Result<(), String>;
    async fn subscribe(&mut self, stream_type: &str, params: Value) -> Result<(), String>;
    async fn unsubscribe(&mut self, stream_type: &str) -> Result<(), String>;
    fn is_connected(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct StubClient {
        connected: bool,
    }

    #[async_trait]
    impl WebSocketClient for StubClient {
        async fn connect(&mut self) -> Result<(), String> {
            self.connected = true;
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), String> {
            self.connected = false;
            Ok(())
        }

        async fn subscribe(&mut self, _stream_type: &str, _params: Value) -> Result<(), String> {
            Ok(())
        }

        async fn unsubscribe(&mut self, _stream_type: &str) -> Result<(), String> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            self.connected
        }
    }

    #[tokio::test]
    async fn connect_sets_client_as_connected() {
        let mut client = StubClient { connected: false };
        client.connect().await.unwrap();
        assert!(client.is_connected());
    }

    #[tokio::test]
    async fn disconnect_sets_client_as_disconnected() {
        let mut client = StubClient { connected: true };
        client.disconnect().await.unwrap();
        assert!(!client.is_connected());
    }

    #[tokio::test]
    async fn subscribe_returns_ok() {
        let mut client = StubClient { connected: true };
        let result = client
            .subscribe("kline", json!({"symbol": "USDT_IRT"}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unsubscribe_returns_ok() {
        let mut client = StubClient { connected: true };
        let result = client.unsubscribe("kline").await;
        assert!(result.is_ok());
    }
}
