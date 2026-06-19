use std::fmt;

use async_trait::async_trait;

#[derive(Debug)]
pub enum TickerError {
    Unavailable(String),
    StorageError(String),
}

impl fmt::Display for TickerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TickerError::Unavailable(msg) => write!(f, "ticker storage unavailable: {}", msg),
            TickerError::StorageError(msg) => write!(f, "ticker storage error: {}", msg),
        }
    }
}

/// Persists the set of actively-running tickers across server restarts.
///
/// Not yet wired into `exchange_handler`; the in-memory `clients` map is
/// still the authoritative source of truth. This trait is ready to be
/// connected when persistence becomes a requirement.
#[async_trait]
pub trait TickerRepository: Send + Sync {
    /// Returns `true` if a ticker for `(exchange, symbol)` is registered.
    async fn exists(&self, exchange: &str, symbol: &str) -> Result<bool, TickerError>;

    /// Records that a ticker for `(exchange, symbol)` has started.
    async fn register(&self, exchange: &str, symbol: &str) -> Result<(), TickerError>;

    /// Refreshes the TTL for an already-registered ticker.
    async fn refresh(&self, exchange: &str, symbol: &str) -> Result<(), TickerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTickerRepository {
        exists_result: bool,
    }

    #[async_trait]
    impl TickerRepository for MockTickerRepository {
        async fn exists(&self, _exchange: &str, _symbol: &str) -> Result<bool, TickerError> {
            Ok(self.exists_result)
        }

        async fn register(&self, _exchange: &str, _symbol: &str) -> Result<(), TickerError> {
            Ok(())
        }

        async fn refresh(&self, _exchange: &str, _symbol: &str) -> Result<(), TickerError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn ticker_repository_exists_returns_true_when_registered() {
        let repo = MockTickerRepository {
            exists_result: true,
        };
        assert!(repo.exists("tabdeal", "USDT_IRT").await.unwrap());
    }

    #[tokio::test]
    async fn ticker_repository_exists_returns_false_when_not_registered() {
        let repo = MockTickerRepository {
            exists_result: false,
        };
        assert!(!repo.exists("tabdeal", "USDT_IRT").await.unwrap());
    }

    #[tokio::test]
    async fn ticker_repository_register_returns_ok() {
        let repo = MockTickerRepository {
            exists_result: false,
        };
        assert!(repo.register("tabdeal", "USDT_IRT").await.is_ok());
    }

    #[tokio::test]
    async fn ticker_error_display_is_not_empty() {
        let err = TickerError::StorageError("connection lost".to_string());
        assert!(!err.to_string().is_empty());
    }
}
