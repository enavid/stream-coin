use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionRecord {
    pub id: i64,
    pub user_id: i32,
    pub strategy_id: String,
    pub active: bool,
    pub max_position_size: Option<Decimal>,
    pub confidence_threshold: Option<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum SubscriptionRepositoryError {
    #[error("database error: {0}")]
    Database(String),
    #[error("subscription {0} not found")]
    NotFound(i64),
    #[error("user {user_id} is already subscribed to strategy '{strategy_id}'")]
    AlreadySubscribed { user_id: i32, strategy_id: String },
}

#[async_trait]
pub trait SubscriptionRepository: Send + Sync {
    /// Creates a new subscription. Returns `AlreadySubscribed` if the user is
    /// already subscribed to `strategy_id` (unique constraint on the pair).
    async fn create(
        &self,
        user_id: i32,
        strategy_id: &str,
        max_position_size: Option<Decimal>,
        confidence_threshold: Option<f64>,
    ) -> Result<SubscriptionRecord, SubscriptionRepositoryError>;

    async fn get(&self, id: i64)
        -> Result<Option<SubscriptionRecord>, SubscriptionRepositoryError>;

    /// All subscriptions for `user_id`, regardless of `active` flag.
    async fn list_for_user(
        &self,
        user_id: i32,
    ) -> Result<Vec<SubscriptionRecord>, SubscriptionRepositoryError>;

    /// Only the rows where `active = true`. Called on every inbound signal to
    /// determine which users should receive orders — must be fast.
    async fn list_active_for_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, SubscriptionRepositoryError>;

    /// Updates `active`, `max_position_size`, and `confidence_threshold` for
    /// the subscription identified by `id`. Returns `NotFound` if it does not exist.
    async fn update(
        &self,
        id: i64,
        active: bool,
        max_position_size: Option<Decimal>,
        confidence_threshold: Option<f64>,
    ) -> Result<SubscriptionRecord, SubscriptionRepositoryError>;

    /// Removes the subscription row. Idempotent — deleting a nonexistent id is `Ok`.
    async fn delete(&self, id: i64) -> Result<(), SubscriptionRepositoryError>;
}

// ---------------------------------------------------------------------------
// In-memory fake — used in unit and integration tests

struct StoredSubscription {
    record: SubscriptionRecord,
}

pub struct FakeSubscriptionRepository {
    inner: Mutex<Vec<StoredSubscription>>,
    next_id: Mutex<i64>,
}

impl Default for FakeSubscriptionRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSubscriptionRepository {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(vec![]),
            next_id: Mutex::new(1),
        }
    }
}

#[async_trait]
impl SubscriptionRepository for FakeSubscriptionRepository {
    async fn create(
        &self,
        user_id: i32,
        strategy_id: &str,
        max_position_size: Option<Decimal>,
        confidence_threshold: Option<f64>,
    ) -> Result<SubscriptionRecord, SubscriptionRepositoryError> {
        let mut inner = self.inner.lock().await;
        if inner
            .iter()
            .any(|s| s.record.user_id == user_id && s.record.strategy_id == strategy_id)
        {
            return Err(SubscriptionRepositoryError::AlreadySubscribed {
                user_id,
                strategy_id: strategy_id.to_string(),
            });
        }
        let mut id_guard = self.next_id.lock().await;
        let id = *id_guard;
        *id_guard += 1;
        let record = SubscriptionRecord {
            id,
            user_id,
            strategy_id: strategy_id.to_string(),
            active: true,
            max_position_size,
            confidence_threshold,
            created_at: Utc::now(),
        };
        inner.push(StoredSubscription {
            record: record.clone(),
        });
        Ok(record)
    }

    async fn get(
        &self,
        id: i64,
    ) -> Result<Option<SubscriptionRecord>, SubscriptionRepositoryError> {
        Ok(self
            .inner
            .lock()
            .await
            .iter()
            .find(|s| s.record.id == id)
            .map(|s| s.record.clone()))
    }

    async fn list_for_user(
        &self,
        user_id: i32,
    ) -> Result<Vec<SubscriptionRecord>, SubscriptionRepositoryError> {
        Ok(self
            .inner
            .lock()
            .await
            .iter()
            .filter(|s| s.record.user_id == user_id)
            .map(|s| s.record.clone())
            .collect())
    }

    async fn list_active_for_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, SubscriptionRepositoryError> {
        Ok(self
            .inner
            .lock()
            .await
            .iter()
            .filter(|s| s.record.strategy_id == strategy_id && s.record.active)
            .map(|s| s.record.clone())
            .collect())
    }

    async fn update(
        &self,
        id: i64,
        active: bool,
        max_position_size: Option<Decimal>,
        confidence_threshold: Option<f64>,
    ) -> Result<SubscriptionRecord, SubscriptionRepositoryError> {
        let mut inner = self.inner.lock().await;
        let entry = inner
            .iter_mut()
            .find(|s| s.record.id == id)
            .ok_or(SubscriptionRepositoryError::NotFound(id))?;
        entry.record.active = active;
        entry.record.max_position_size = max_position_size;
        entry.record.confidence_threshold = confidence_threshold;
        Ok(entry.record.clone())
    }

    async fn delete(&self, id: i64) -> Result<(), SubscriptionRepositoryError> {
        self.inner.lock().await.retain(|s| s.record.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn create_subscription_returns_record_with_assigned_id() {
        let repo = FakeSubscriptionRepository::new();
        let rec = repo
            .create(1, "spread-1", None, None)
            .await
            .expect("create must succeed");
        assert_eq!(rec.id, 1);
        assert_eq!(rec.user_id, 1);
        assert_eq!(rec.strategy_id, "spread-1");
        assert!(rec.active);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_assigns_sequential_ids_across_users() {
        let repo = FakeSubscriptionRepository::new();
        let r1 = repo.create(1, "s1", None, None).await.unwrap();
        let r2 = repo.create(2, "s1", None, None).await.unwrap();
        assert_eq!(r1.id, 1);
        assert_eq!(r2.id, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_duplicate_subscription_returns_already_subscribed_error() {
        let repo = FakeSubscriptionRepository::new();
        repo.create(1, "spread-1", None, None).await.unwrap();
        let result = repo.create(1, "spread-1", None, None).await;
        assert!(
            matches!(
                result,
                Err(SubscriptionRepositoryError::AlreadySubscribed { .. })
            ),
            "duplicate subscription must be rejected"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn same_strategy_different_users_are_independent() {
        let repo = FakeSubscriptionRepository::new();
        repo.create(1, "spread-1", None, None).await.unwrap();
        let result = repo.create(2, "spread-1", None, None).await;
        assert!(
            result.is_ok(),
            "different users can subscribe to the same strategy"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_returns_none_for_unknown_id() {
        let repo = FakeSubscriptionRepository::new();
        assert!(repo.get(999).await.unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_returns_subscription_by_id() {
        let repo = FakeSubscriptionRepository::new();
        let created = repo.create(5, "rsi-1", None, Some(0.8)).await.unwrap();
        let found = repo.get(created.id).await.unwrap().unwrap();
        assert_eq!(found.strategy_id, "rsi-1");
        assert_eq!(found.confidence_threshold, Some(0.8));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_for_user_returns_only_that_users_subscriptions() {
        let repo = FakeSubscriptionRepository::new();
        repo.create(1, "s1", None, None).await.unwrap();
        repo.create(1, "s2", None, None).await.unwrap();
        repo.create(2, "s1", None, None).await.unwrap();
        let user1_subs = repo.list_for_user(1).await.unwrap();
        assert_eq!(user1_subs.len(), 2);
        assert!(user1_subs.iter().all(|s| s.user_id == 1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_for_user_returns_empty_for_user_with_no_subscriptions() {
        let repo = FakeSubscriptionRepository::new();
        assert!(repo.list_for_user(99).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_active_for_strategy_returns_only_active_subscriptions() {
        let repo = FakeSubscriptionRepository::new();
        let sub = repo.create(1, "spread-1", None, None).await.unwrap();
        repo.create(2, "spread-1", None, None).await.unwrap();
        // Deactivate user 1's subscription
        repo.update(sub.id, false, None, None).await.unwrap();

        let active = repo.list_active_for_strategy("spread-1").await.unwrap();
        assert_eq!(active.len(), 1, "deactivated subscription must not appear");
        assert_eq!(active[0].user_id, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_active_for_strategy_excludes_other_strategies() {
        let repo = FakeSubscriptionRepository::new();
        repo.create(1, "spread-1", None, None).await.unwrap();
        repo.create(1, "rsi-1", None, None).await.unwrap();
        let active = repo.list_active_for_strategy("spread-1").await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].strategy_id, "spread-1");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn update_changes_active_flag_and_overrides() {
        let repo = FakeSubscriptionRepository::new();
        let sub = repo.create(1, "s1", None, None).await.unwrap();
        let updated = repo
            .update(sub.id, false, Some(Decimal::new(500, 0)), Some(0.9))
            .await
            .unwrap();
        assert!(!updated.active);
        assert_eq!(updated.max_position_size, Some(Decimal::new(500, 0)));
        assert_eq!(updated.confidence_threshold, Some(0.9));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn update_nonexistent_subscription_returns_not_found() {
        let repo = FakeSubscriptionRepository::new();
        let result = repo.update(999, true, None, None).await;
        assert!(matches!(
            result,
            Err(SubscriptionRepositoryError::NotFound(999))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delete_removes_subscription() {
        let repo = FakeSubscriptionRepository::new();
        let sub = repo.create(1, "s1", None, None).await.unwrap();
        repo.delete(sub.id).await.unwrap();
        assert!(repo.get(sub.id).await.unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delete_nonexistent_id_is_idempotent() {
        let repo = FakeSubscriptionRepository::new();
        assert!(
            repo.delete(999).await.is_ok(),
            "deleting a missing id must not error"
        );
    }
}
