use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct TickerSubscription {
    pub exchange: String,
    pub symbol: String,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait TickerRepository: Send + Sync {
    async fn insert(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError>;
    async fn remove(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError>;
    async fn list_active(&self) -> Result<Vec<TickerSubscription>, RepositoryError>;
}

pub struct FakeTickerRepository {
    inner: Mutex<Vec<TickerSubscription>>,
}

impl Default for FakeTickerRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeTickerRepository {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(vec![]),
        }
    }

    pub fn new_with(subs: Vec<TickerSubscription>) -> Self {
        Self {
            inner: Mutex::new(subs),
        }
    }
}

#[async_trait]
impl TickerRepository for FakeTickerRepository {
    async fn insert(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError> {
        let mut inner = self.inner.lock().await;
        let exists = inner
            .iter()
            .any(|s| s.exchange == exchange && s.symbol == symbol);
        if !exists {
            inner.push(TickerSubscription {
                exchange: exchange.to_string(),
                symbol: symbol.to_string(),
            });
        }
        Ok(())
    }

    async fn remove(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError> {
        let mut inner = self.inner.lock().await;
        inner.retain(|s| !(s.exchange == exchange && s.symbol == symbol));
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<TickerSubscription>, RepositoryError> {
        Ok(self.inner.lock().await.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn fake_repo_insert_adds_subscription() {
        let repo = FakeTickerRepository::new();
        repo.insert("tabdeal", "USDT/IRT").await.unwrap();
        let subs = repo.list_active().await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].exchange, "tabdeal");
        assert_eq!(subs[0].symbol, "USDT/IRT");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_repo_insert_is_idempotent() {
        let repo = FakeTickerRepository::new();
        repo.insert("tabdeal", "USDT/IRT").await.unwrap();
        repo.insert("tabdeal", "USDT/IRT").await.unwrap();
        assert_eq!(repo.list_active().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_repo_remove_deletes_subscription() {
        let repo = FakeTickerRepository::new_with(vec![TickerSubscription {
            exchange: "tabdeal".to_string(),
            symbol: "USDT/IRT".to_string(),
        }]);
        repo.remove("tabdeal", "USDT/IRT").await.unwrap();
        assert!(repo.list_active().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_repo_remove_nonexistent_is_noop() {
        let repo = FakeTickerRepository::new();
        let result = repo.remove("tabdeal", "USDT/IRT").await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_repo_new_with_seeds_initial_state() {
        let repo = FakeTickerRepository::new_with(vec![
            TickerSubscription {
                exchange: "tabdeal".to_string(),
                symbol: "USDT/IRT".to_string(),
            },
            TickerSubscription {
                exchange: "hitobit".to_string(),
                symbol: "BTC/IRT".to_string(),
            },
        ]);
        assert_eq!(repo.list_active().await.unwrap().len(), 2);
    }
}
