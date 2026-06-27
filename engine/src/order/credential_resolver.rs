use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::infrastructure::crypto::credential_cipher::{CredentialCipher, CryptoError};
use crate::infrastructure::db::credential_repository::{
    CredentialRepository, CredentialRepositoryError,
};
use crate::order::port::{OrderAdapter, OrderAdapterError};

/// Factory: given decrypted credentials JSON for an exchange, construct an OrderAdapter.
/// Returns `Err(String)` when required fields (e.g. `api_key`) are missing.
pub type OrderAdapterFactory =
    Arc<dyn Fn(&serde_json::Value) -> Result<Arc<dyn OrderAdapter>, String> + Send + Sync>;

#[derive(Debug, Error)]
pub enum CredentialResolverError {
    #[error("credential repository error: {0}")]
    Repository(#[from] CredentialRepositoryError),
    #[error("decryption failed: {0}")]
    Crypto(#[from] CryptoError),
    #[error("credential JSON is invalid: {0}")]
    InvalidCredentials(String),
    #[error("no order adapter factory registered for exchange '{0}'")]
    NoFactory(String),
}

impl From<CredentialResolverError> for OrderAdapterError {
    fn from(e: CredentialResolverError) -> Self {
        OrderAdapterError::Rejected(e.to_string())
    }
}

/// Port: resolves per-user exchange credentials into a ready-to-use `OrderAdapter`.
#[async_trait]
pub trait CredentialResolver: Send + Sync {
    /// Returns an `OrderAdapter` loaded with `user_id`'s credentials for `exchange`.
    /// Returns `Ok(None)` when the user has no credentials stored for that exchange.
    async fn adapter_for_user(
        &self,
        user_id: i32,
        exchange: &str,
    ) -> Result<Option<Arc<dyn OrderAdapter>>, CredentialResolverError>;
}

// ---------------------------------------------------------------------------
// Live implementation — uses the credential repository + AES-256-GCM cipher

/// Fetches encrypted credentials from the DB, decrypts them, then constructs an
/// exchange-specific `OrderAdapter` using the factory registered for that exchange.
pub struct LiveCredentialResolver {
    credential_repository: Arc<dyn CredentialRepository>,
    cipher: Arc<CredentialCipher>,
    /// One factory per canonical exchange name (e.g. `"tabdeal"`).
    factories: HashMap<String, OrderAdapterFactory>,
}

impl LiveCredentialResolver {
    pub fn new(
        credential_repository: Arc<dyn CredentialRepository>,
        cipher: Arc<CredentialCipher>,
        factories: HashMap<String, OrderAdapterFactory>,
    ) -> Self {
        Self {
            credential_repository,
            cipher,
            factories,
        }
    }
}

#[async_trait]
impl CredentialResolver for LiveCredentialResolver {
    async fn adapter_for_user(
        &self,
        user_id: i32,
        exchange: &str,
    ) -> Result<Option<Arc<dyn OrderAdapter>>, CredentialResolverError> {
        let Some(envelope) = self.credential_repository.get(user_id, exchange).await? else {
            return Ok(None);
        };

        let plaintext = self.cipher.decrypt(&envelope)?;

        let creds: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| CredentialResolverError::InvalidCredentials(e.to_string()))?;

        let factory = self
            .factories
            .get(exchange)
            .ok_or_else(|| CredentialResolverError::NoFactory(exchange.to_string()))?;

        let adapter = factory(&creds).map_err(CredentialResolverError::InvalidCredentials)?;

        tracing::debug!(
            user_id,
            exchange,
            "credential resolver: adapter constructed from stored credentials"
        );

        Ok(Some(adapter))
    }
}

// ---------------------------------------------------------------------------
// Fake — for unit and integration tests

pub struct FakeCredentialResolver {
    adapter: Option<Arc<dyn OrderAdapter>>,
}

impl FakeCredentialResolver {
    /// Always returns `Some(adapter)` regardless of user_id / exchange.
    pub fn returning(adapter: Arc<dyn OrderAdapter>) -> Self {
        Self {
            adapter: Some(adapter),
        }
    }

    /// Always returns `None` (user has no credentials stored).
    pub fn none() -> Self {
        Self { adapter: None }
    }
}

#[async_trait]
impl CredentialResolver for FakeCredentialResolver {
    async fn adapter_for_user(
        &self,
        _user_id: i32,
        _exchange: &str,
    ) -> Result<Option<Arc<dyn OrderAdapter>>, CredentialResolverError> {
        Ok(self.adapter.as_ref().map(Arc::clone))
    }
}

// ---------------------------------------------------------------------------
// Unit tests

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::infrastructure::crypto::credential_cipher::CredentialCipher;
    use crate::infrastructure::db::credential_repository::FakeCredentialRepository;
    use crate::order::fake::FakeOrderAdapter;
    use crate::order::port::OrderAdapter;

    fn cipher() -> Arc<CredentialCipher> {
        Arc::new(CredentialCipher::new([42u8; 32]))
    }

    fn repo_with(exchanges: &[&str]) -> Arc<FakeCredentialRepository> {
        Arc::new(FakeCredentialRepository::new(
            exchanges.iter().map(|s| s.to_string()).collect(),
        ))
    }

    fn tabdeal_factory() -> (String, OrderAdapterFactory) {
        (
            "tabdeal".to_string(),
            Arc::new(|_creds: &serde_json::Value| {
                Ok(Arc::new(FakeOrderAdapter::new("tabdeal")) as Arc<dyn OrderAdapter>)
            }),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_resolver_returns_none_when_no_credentials_stored() {
        let repo = repo_with(&["tabdeal"]);
        let resolver = LiveCredentialResolver::new(
            repo as Arc<dyn CredentialRepository>,
            cipher(),
            HashMap::new(),
        );

        let result = resolver.adapter_for_user(42, "tabdeal").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_resolver_returns_adapter_when_credentials_exist() {
        let repo = repo_with(&["tabdeal"]);
        let c = cipher();
        let creds = serde_json::json!({"api_key": "user-key"});
        let envelope = c.encrypt(serde_json::to_vec(&creds).unwrap().as_slice());
        repo.upsert(42, "tabdeal", envelope).await.unwrap();

        let (name, factory) = tabdeal_factory();
        let resolver = LiveCredentialResolver::new(
            Arc::clone(&repo) as Arc<dyn CredentialRepository>,
            c,
            HashMap::from([(name, factory)]),
        );

        let adapter = resolver.adapter_for_user(42, "tabdeal").await.unwrap();
        assert!(adapter.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_resolver_returns_no_factory_error_for_unknown_exchange() {
        let repo = repo_with(&["tabdeal"]);
        let c = cipher();
        let creds = serde_json::json!({"api_key": "key"});
        let envelope = c.encrypt(serde_json::to_vec(&creds).unwrap().as_slice());
        repo.upsert(42, "tabdeal", envelope).await.unwrap();

        let resolver =
            LiveCredentialResolver::new(repo as Arc<dyn CredentialRepository>, c, HashMap::new());
        let result = resolver.adapter_for_user(42, "tabdeal").await;
        assert!(
            matches!(result, Err(CredentialResolverError::NoFactory(_))),
            "expected NoFactory error"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_resolver_does_not_cross_users() {
        let repo = repo_with(&["tabdeal"]);
        let c = cipher();
        let creds = serde_json::json!({"api_key": "alice-key"});
        let envelope = c.encrypt(serde_json::to_vec(&creds).unwrap().as_slice());
        repo.upsert(1, "tabdeal", envelope).await.unwrap();

        let (name, factory) = tabdeal_factory();
        let resolver = LiveCredentialResolver::new(
            Arc::clone(&repo) as Arc<dyn CredentialRepository>,
            c,
            HashMap::from([(name, factory)]),
        );

        let result = resolver.adapter_for_user(2, "tabdeal").await.unwrap();
        assert!(result.is_none(), "user 2 must not get user 1's adapter");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_resolver_returning_always_gives_adapter() {
        let adapter = Arc::new(FakeOrderAdapter::new("tabdeal")) as Arc<dyn OrderAdapter>;
        let resolver = FakeCredentialResolver::returning(adapter);

        let result = resolver.adapter_for_user(1, "tabdeal").await.unwrap();
        assert!(result.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_resolver_none_always_returns_none() {
        let resolver = FakeCredentialResolver::none();
        let result = resolver.adapter_for_user(1, "tabdeal").await.unwrap();
        assert!(result.is_none());
    }
}
