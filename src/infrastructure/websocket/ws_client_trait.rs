use serde_json::Value;
use async_trait::async_trait;

#[async_trait]
pub trait WebSocketClient: Send + Sync {
    async fn connect(&mut self) -> Result<(), String>;
    async fn disconnect(&mut self) -> Result<(), String>;
    async fn subscribe(&mut self, stream_type: &str, params: Value) -> Result<(), String>;
    async fn unsubscribe(&mut self, stream_type: &str) -> Result<(), String>;
    fn is_connected(&self) -> bool;
}
