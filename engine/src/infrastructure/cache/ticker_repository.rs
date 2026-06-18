use async_trait::async_trait;
use chrono::Utc;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

use crate::ticker::port::{TickerError, TickerRepository};

pub struct RedisTickerRepository {
    conn: MultiplexedConnection,
}

impl RedisTickerRepository {
    pub fn new(conn: MultiplexedConnection) -> Self {
        Self { conn }
    }

    fn key(exchange: &str, symbol: &str) -> String {
        format!("ticker:{}:{}", exchange, symbol)
    }
}

#[async_trait]
impl TickerRepository for RedisTickerRepository {
    async fn exists(&self, exchange: &str, symbol: &str) -> Result<bool, TickerError> {
        let mut conn = self.conn.clone();
        conn.exists(Self::key(exchange, symbol))
            .await
            .map_err(|e| TickerError::StorageError(e.to_string()))
    }

    async fn register(&self, exchange: &str, symbol: &str) -> Result<(), TickerError> {
        let mut conn = self.conn.clone();
        let info = serde_json::json!({
            "exchange": exchange,
            "symbol": symbol,
            "status": "running",
            "started_at": Utc::now().timestamp()
        });
        conn.set_ex(Self::key(exchange, symbol), info.to_string(), 120)
            .await
            .map_err(|e| TickerError::StorageError(e.to_string()))
    }

    async fn refresh(&self, exchange: &str, symbol: &str) -> Result<(), TickerError> {
        let mut conn = self.conn.clone();
        conn.expire(Self::key(exchange, symbol), 120)
            .await
            .map_err(|e| TickerError::StorageError(e.to_string()))
    }
}
