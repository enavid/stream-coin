use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PythonStrategyRecord {
    pub strategy_id: String,
    pub name: String,
    pub code: String,
    pub params_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum PythonStrategyRepositoryError {
    #[error("database error: {0}")]
    Database(String),
    #[error("strategy not found: {0}")]
    NotFound(String),
}

#[async_trait]
pub trait PythonStrategyRepository: Send + Sync {
    async fn save(
        &self,
        record: &PythonStrategyRecord,
    ) -> Result<(), PythonStrategyRepositoryError>;
    async fn get(
        &self,
        strategy_id: &str,
    ) -> Result<PythonStrategyRecord, PythonStrategyRepositoryError>;
    async fn list_active(&self)
        -> Result<Vec<PythonStrategyRecord>, PythonStrategyRepositoryError>;
    async fn remove(&self, strategy_id: &str) -> Result<(), PythonStrategyRepositoryError>;
}

pub struct FakePythonStrategyRepository {
    inner: Mutex<Vec<PythonStrategyRecord>>,
}

impl Default for FakePythonStrategyRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePythonStrategyRepository {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(vec![]),
        }
    }

    pub fn with_records(records: Vec<PythonStrategyRecord>) -> Self {
        Self {
            inner: Mutex::new(records),
        }
    }

    pub async fn all_records(&self) -> Vec<PythonStrategyRecord> {
        self.inner.lock().await.clone()
    }
}

#[async_trait]
impl PythonStrategyRepository for FakePythonStrategyRepository {
    async fn save(
        &self,
        record: &PythonStrategyRecord,
    ) -> Result<(), PythonStrategyRepositoryError> {
        let mut inner = self.inner.lock().await;
        inner.retain(|r| r.strategy_id != record.strategy_id);
        inner.push(record.clone());
        Ok(())
    }

    async fn get(
        &self,
        strategy_id: &str,
    ) -> Result<PythonStrategyRecord, PythonStrategyRepositoryError> {
        self.inner
            .lock()
            .await
            .iter()
            .find(|r| r.strategy_id == strategy_id)
            .cloned()
            .ok_or_else(|| PythonStrategyRepositoryError::NotFound(strategy_id.to_string()))
    }

    async fn list_active(
        &self,
    ) -> Result<Vec<PythonStrategyRecord>, PythonStrategyRepositoryError> {
        Ok(self.inner.lock().await.clone())
    }

    async fn remove(&self, strategy_id: &str) -> Result<(), PythonStrategyRepositoryError> {
        self.inner
            .lock()
            .await
            .retain(|r| r.strategy_id != strategy_id);
        Ok(())
    }
}
