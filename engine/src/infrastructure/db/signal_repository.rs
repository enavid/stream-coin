use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct SignalRecord {
    pub signal_id: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum SignalRepositoryError {
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait SignalRepository: Send + Sync {
    async fn save(&self, record: &SignalRecord) -> Result<(), SignalRepositoryError>;
}

pub struct FakeSignalRepository {
    inner: Mutex<Vec<SignalRecord>>,
}

impl Default for FakeSignalRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSignalRepository {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(vec![]),
        }
    }

    pub async fn records(&self) -> Vec<SignalRecord> {
        self.inner.lock().await.clone()
    }
}

#[async_trait]
impl SignalRepository for FakeSignalRepository {
    async fn save(&self, record: &SignalRecord) -> Result<(), SignalRepositoryError> {
        self.inner.lock().await.push(record.clone());
        Ok(())
    }
}
