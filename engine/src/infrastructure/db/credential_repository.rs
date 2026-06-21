use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::infrastructure::crypto::credential_cipher::EncryptedEnvelope;

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialSummary {
    pub exchange_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum CredentialRepositoryError {
    #[error("database error: {0}")]
    Database(String),
    #[error("exchange not found: {0}")]
    ExchangeNotFound(String),
}

#[async_trait]
pub trait CredentialRepository: Send + Sync {
    async fn upsert(
        &self,
        user_id: i32,
        exchange_name: &str,
        envelope: EncryptedEnvelope,
    ) -> Result<(), CredentialRepositoryError>;
    async fn get(
        &self,
        user_id: i32,
        exchange_name: &str,
    ) -> Result<Option<EncryptedEnvelope>, CredentialRepositoryError>;
    async fn list_for_user(
        &self,
        user_id: i32,
    ) -> Result<Vec<CredentialSummary>, CredentialRepositoryError>;
    async fn delete(
        &self,
        user_id: i32,
        exchange_name: &str,
    ) -> Result<(), CredentialRepositoryError>;
}

struct StoredCredential {
    user_id: i32,
    exchange_name: String,
    envelope: EncryptedEnvelope,
    created_at: DateTime<Utc>,
}

pub struct FakeCredentialRepository {
    inner: tokio::sync::Mutex<Vec<StoredCredential>>,
    /// Exchanges known to the registry — `upsert` rejects any name not in this set,
    /// mirroring the `exchanges` foreign key in Postgres.
    known_exchanges: Vec<String>,
}

impl FakeCredentialRepository {
    pub fn new(known_exchanges: Vec<String>) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(vec![]),
            known_exchanges,
        }
    }
}

#[async_trait]
impl CredentialRepository for FakeCredentialRepository {
    async fn upsert(
        &self,
        user_id: i32,
        exchange_name: &str,
        envelope: EncryptedEnvelope,
    ) -> Result<(), CredentialRepositoryError> {
        if !self.known_exchanges.iter().any(|e| e == exchange_name) {
            return Err(CredentialRepositoryError::ExchangeNotFound(
                exchange_name.to_string(),
            ));
        }
        let mut inner = self.inner.lock().await;
        inner.retain(|c| !(c.user_id == user_id && c.exchange_name == exchange_name));
        inner.push(StoredCredential {
            user_id,
            exchange_name: exchange_name.to_string(),
            envelope,
            created_at: Utc::now(),
        });
        Ok(())
    }

    async fn get(
        &self,
        user_id: i32,
        exchange_name: &str,
    ) -> Result<Option<EncryptedEnvelope>, CredentialRepositoryError> {
        Ok(self
            .inner
            .lock()
            .await
            .iter()
            .find(|c| c.user_id == user_id && c.exchange_name == exchange_name)
            .map(|c| c.envelope.clone()))
    }

    async fn list_for_user(
        &self,
        user_id: i32,
    ) -> Result<Vec<CredentialSummary>, CredentialRepositoryError> {
        Ok(self
            .inner
            .lock()
            .await
            .iter()
            .filter(|c| c.user_id == user_id)
            .map(|c| CredentialSummary {
                exchange_name: c.exchange_name.clone(),
                created_at: c.created_at,
            })
            .collect())
    }

    async fn delete(
        &self,
        user_id: i32,
        exchange_name: &str,
    ) -> Result<(), CredentialRepositoryError> {
        self.inner
            .lock()
            .await
            .retain(|c| !(c.user_id == user_id && c.exchange_name == exchange_name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> EncryptedEnvelope {
        EncryptedEnvelope {
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_then_get_returns_stored_envelope() {
        let repo = FakeCredentialRepository::new(vec!["tabdeal".to_string()]);
        repo.upsert(1, "tabdeal", envelope()).await.unwrap();
        let got = repo.get(1, "tabdeal").await.unwrap().unwrap();
        assert_eq!(got, envelope());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_rejects_unknown_exchange() {
        let repo = FakeCredentialRepository::new(vec!["tabdeal".to_string()]);
        let result = repo.upsert(1, "nobitex", envelope()).await;
        assert!(matches!(
            result,
            Err(CredentialRepositoryError::ExchangeNotFound(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_is_idempotent_per_user_and_exchange() {
        let repo = FakeCredentialRepository::new(vec!["tabdeal".to_string()]);
        repo.upsert(1, "tabdeal", envelope()).await.unwrap();
        let second = EncryptedEnvelope {
            nonce: "n2".to_string(),
            ciphertext: "c2".to_string(),
        };
        repo.upsert(1, "tabdeal", second.clone()).await.unwrap();
        assert_eq!(repo.list_for_user(1).await.unwrap().len(), 1);
        assert_eq!(repo.get(1, "tabdeal").await.unwrap().unwrap(), second);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_for_user_only_returns_own_credentials() {
        let repo =
            FakeCredentialRepository::new(vec!["tabdeal".to_string(), "hitobit".to_string()]);
        repo.upsert(1, "tabdeal", envelope()).await.unwrap();
        repo.upsert(2, "hitobit", envelope()).await.unwrap();
        let list = repo.list_for_user(1).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].exchange_name, "tabdeal");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_returns_none_for_other_users_credential() {
        let repo = FakeCredentialRepository::new(vec!["tabdeal".to_string()]);
        repo.upsert(1, "tabdeal", envelope()).await.unwrap();
        assert!(repo.get(2, "tabdeal").await.unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delete_removes_credential() {
        let repo = FakeCredentialRepository::new(vec!["tabdeal".to_string()]);
        repo.upsert(1, "tabdeal", envelope()).await.unwrap();
        repo.delete(1, "tabdeal").await.unwrap();
        assert!(repo.get(1, "tabdeal").await.unwrap().is_none());
    }
}
