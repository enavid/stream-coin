use std::sync::Arc;
use tokio::sync::Mutex;
use redis::AsyncCommands;
use std::collections::HashMap;
use crate::infrastructure::websocket::kucoin_ws::ExchangeClient;

#[derive(Clone)]
pub struct ExchangeService {
    redis_client: redis::Client,
    active_connections: Arc<Mutex<HashMap<String, bool>>>, // Store connection statuses
}
impl ExchangeService {
    pub fn new(redis_client: redis::Client) -> Self {
        Self {
            redis_client,
            active_connections: Mutex::new(HashMap::new()).into(),
        }
    }

    /// **Connect to WebSocket of the exchange**
    pub async fn connect(&self, exchange_name: &str, symbols: Vec<String>) -> Result<(), String> {
        let mut redis_conn = self.redis_client.get_multiplexed_tokio_connection().await.map_err(|e| e.to_string())?;

        // Check if already connected
        let key = format!("ws_connection:{}", exchange_name);
        let is_connected: Option<bool> = redis_conn.get(&key).await.map_err(|e| e.to_string())?;

        if let Some(true) = is_connected {
            return Err(format!("Connection to {} already established!", exchange_name));
        }

        // Connect WebSocket
        let client = ExchangeClient::new(exchange_name.to_string(), symbols);
        client.connect().await.map_err(|e| e.to_string())?;

        // Store status in Redis
        redis_conn.set(&key, true).await.map_err(|e| e.to_string())?;

        // Store in memory
        let mut active = self.active_connections.lock().await;
        active.insert(exchange_name.to_string(), true);

        Ok(())
    }

    /// **Disconnect WebSocket from the exchange**
    pub async fn disconnect(&self, exchange_name: &str) -> Result<(), String> {
        let mut redis_conn = self.redis_client.get_multiplexed_tokio_connection().await.map_err(|e| e.to_string())?;

        // Check connection status
        let key = format!("ws_connection:{}", exchange_name);
        let is_connected: Option<bool> = redis_conn.get(&key).await.map_err(|e| e.to_string())?;

        if is_connected.is_none() || is_connected == Some(false) {
            return Err(format!("No connection found for {}", exchange_name));
        }

        // Disconnect WebSocket
        let client = ExchangeClient::new(exchange_name.to_string(), vec![]);
        client.disconnect().await.map_err(|e| e.to_string())?;

        // Remove status from Redis
        redis_conn.del(&key).await.map_err(|e| e.to_string())?;

        // Remove from memory
        let mut active = self.active_connections.lock().await;
        active.remove(exchange_name);

        Ok(())
    }
}
