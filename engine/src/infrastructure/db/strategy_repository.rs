use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct StrategyRecord {
    pub strategy_id: String,
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
    pub params_json: serde_json::Value,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StrategyRegistration {
    pub strategy_id: String,
    pub name: String,
    pub strategy_type: String,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum StrategyRepositoryError {
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait StrategyRepository: Send + Sync {
    async fn save(&self, record: &StrategyRecord) -> Result<(), StrategyRepositoryError>;
    async fn remove(&self, strategy_id: &str) -> Result<(), StrategyRepositoryError>;
    async fn list_active(&self) -> Result<Vec<StrategyRecord>, StrategyRepositoryError>;
    async fn register(&self, reg: &StrategyRegistration) -> Result<(), StrategyRepositoryError>;
}

pub struct FakeStrategyRepository {
    inner: Mutex<Vec<StrategyRecord>>,
    registrations: Mutex<Vec<StrategyRegistration>>,
}

impl Default for FakeStrategyRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeStrategyRepository {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(vec![]),
            registrations: Mutex::new(vec![]),
        }
    }

    pub fn with_records(records: Vec<StrategyRecord>) -> Self {
        Self {
            inner: Mutex::new(records),
            registrations: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl StrategyRepository for FakeStrategyRepository {
    async fn save(&self, record: &StrategyRecord) -> Result<(), StrategyRepositoryError> {
        let mut inner = self.inner.lock().await;
        inner.retain(|r| r.strategy_id != record.strategy_id);
        inner.push(record.clone());
        Ok(())
    }

    async fn remove(&self, strategy_id: &str) -> Result<(), StrategyRepositoryError> {
        let mut inner = self.inner.lock().await;
        inner.retain(|r| r.strategy_id != strategy_id);
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<StrategyRecord>, StrategyRepositoryError> {
        Ok(self.inner.lock().await.clone())
    }

    async fn register(&self, reg: &StrategyRegistration) -> Result<(), StrategyRepositoryError> {
        let mut regs = self.registrations.lock().await;
        regs.retain(|r| r.strategy_id != reg.strategy_id);
        regs.push(reg.clone());
        Ok(())
    }
}
