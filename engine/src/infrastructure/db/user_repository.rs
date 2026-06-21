use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::infrastructure::crypto::password::hash_password;

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: i32,
    pub username: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RoleRecord {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Error)]
pub enum UserRepositoryError {
    #[error("database error: {0}")]
    Database(String),
    #[error("username already exists: {0}")]
    DuplicateUsername(String),
    #[error("role not found: {0}")]
    RoleNotFound(String),
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<UserRecord, UserRepositoryError>;
    async fn find_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserRecord>, UserRepositoryError>;
    async fn list_users(&self) -> Result<Vec<UserRecord>, UserRepositoryError>;
    async fn assign_roles(
        &self,
        user_id: i32,
        role_names: &[String],
    ) -> Result<(), UserRepositoryError>;
    async fn roles_for_user(&self, user_id: i32) -> Result<Vec<String>, UserRepositoryError>;
    /// Flattened, deduplicated permission set across every role assigned to the user.
    async fn permissions_for_user(&self, user_id: i32) -> Result<Vec<String>, UserRepositoryError>;
    async fn list_roles(&self) -> Result<Vec<RoleRecord>, UserRepositoryError>;
    async fn create_role(
        &self,
        name: &str,
        permissions: &[String],
    ) -> Result<(), UserRepositoryError>;
    async fn list_permissions(&self) -> Result<Vec<String>, UserRepositoryError>;
    async fn user_count(&self) -> Result<i64, UserRepositoryError>;
}

pub struct FakeUserRepository {
    users: tokio::sync::Mutex<Vec<UserRecord>>,
    roles: tokio::sync::Mutex<Vec<RoleRecord>>,
    user_roles: tokio::sync::Mutex<Vec<(i32, String)>>,
    permission_catalog: tokio::sync::Mutex<Vec<String>>,
    next_id: std::sync::atomic::AtomicI32,
}

impl Default for FakeUserRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeUserRepository {
    pub fn new() -> Self {
        Self {
            users: tokio::sync::Mutex::new(vec![]),
            roles: tokio::sync::Mutex::new(vec![]),
            user_roles: tokio::sync::Mutex::new(vec![]),
            permission_catalog: tokio::sync::Mutex::new(vec![]),
            next_id: std::sync::atomic::AtomicI32::new(1),
        }
    }

    /// Seeds the permission catalog and role table — mirrors migration 0008's seed data.
    pub fn with_roles(roles: Vec<RoleRecord>) -> Self {
        let catalog: Vec<String> =
            roles
                .iter()
                .flat_map(|r| r.permissions.clone())
                .fold(Vec::new(), |mut acc, p| {
                    if !acc.contains(&p) {
                        acc.push(p);
                    }
                    acc
                });
        Self {
            users: tokio::sync::Mutex::new(vec![]),
            roles: tokio::sync::Mutex::new(roles),
            user_roles: tokio::sync::Mutex::new(vec![]),
            permission_catalog: tokio::sync::Mutex::new(catalog),
            next_id: std::sync::atomic::AtomicI32::new(1),
        }
    }
}

#[async_trait]
impl UserRepository for FakeUserRepository {
    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<UserRecord, UserRepositoryError> {
        let mut users = self.users.lock().await;
        if users.iter().any(|u| u.username == username) {
            return Err(UserRepositoryError::DuplicateUsername(username.to_string()));
        }
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let record = UserRecord {
            id,
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            created_at: Utc::now(),
        };
        users.push(record.clone());
        Ok(record)
    }

    async fn find_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserRecord>, UserRepositoryError> {
        Ok(self
            .users
            .lock()
            .await
            .iter()
            .find(|u| u.username == username)
            .cloned())
    }

    async fn list_users(&self) -> Result<Vec<UserRecord>, UserRepositoryError> {
        Ok(self.users.lock().await.clone())
    }

    async fn assign_roles(
        &self,
        user_id: i32,
        role_names: &[String],
    ) -> Result<(), UserRepositoryError> {
        let roles = self.roles.lock().await;
        for name in role_names {
            if !roles.iter().any(|r| &r.name == name) {
                return Err(UserRepositoryError::RoleNotFound(name.clone()));
            }
        }
        let mut user_roles = self.user_roles.lock().await;
        user_roles.retain(|(uid, _)| *uid != user_id);
        for name in role_names {
            user_roles.push((user_id, name.clone()));
        }
        Ok(())
    }

    async fn roles_for_user(&self, user_id: i32) -> Result<Vec<String>, UserRepositoryError> {
        Ok(self
            .user_roles
            .lock()
            .await
            .iter()
            .filter(|(uid, _)| *uid == user_id)
            .map(|(_, name)| name.clone())
            .collect())
    }

    async fn permissions_for_user(&self, user_id: i32) -> Result<Vec<String>, UserRepositoryError> {
        let role_names: Vec<String> = self
            .user_roles
            .lock()
            .await
            .iter()
            .filter(|(uid, _)| *uid == user_id)
            .map(|(_, name)| name.clone())
            .collect();
        let roles = self.roles.lock().await;
        let mut permissions = Vec::new();
        for role_name in &role_names {
            if let Some(role) = roles.iter().find(|r| &r.name == role_name) {
                for perm in &role.permissions {
                    if !permissions.contains(perm) {
                        permissions.push(perm.clone());
                    }
                }
            }
        }
        Ok(permissions)
    }

    async fn list_roles(&self) -> Result<Vec<RoleRecord>, UserRepositoryError> {
        Ok(self.roles.lock().await.clone())
    }

    async fn create_role(
        &self,
        name: &str,
        permissions: &[String],
    ) -> Result<(), UserRepositoryError> {
        let mut roles = self.roles.lock().await;
        roles.push(RoleRecord {
            name: name.to_string(),
            permissions: permissions.to_vec(),
        });
        let mut catalog = self.permission_catalog.lock().await;
        for p in permissions {
            if !catalog.contains(p) {
                catalog.push(p.clone());
            }
        }
        Ok(())
    }

    async fn list_permissions(&self) -> Result<Vec<String>, UserRepositoryError> {
        Ok(self.permission_catalog.lock().await.clone())
    }

    async fn user_count(&self) -> Result<i64, UserRepositoryError> {
        Ok(self.users.lock().await.len() as i64)
    }
}

/// Bootstraps the very first admin account from env vars when `users` is empty.
/// No-ops if any user already exists — env credentials are a one-time seed, not
/// an ongoing login path (that's `find_by_username` + `verify_password` now).
pub async fn seed_admin_if_empty(
    repo: &dyn UserRepository,
    username: &str,
    password: &str,
) -> Result<(), UserRepositoryError> {
    if repo.user_count().await? > 0 {
        return Ok(());
    }
    let user = repo.create_user(username, &hash_password(password)).await?;
    repo.assign_roles(user.id, &["admin".to_string()]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_repo() -> FakeUserRepository {
        FakeUserRepository::with_roles(vec![
            RoleRecord {
                name: "admin".to_string(),
                permissions: vec!["users.manage".to_string(), "roles.manage".to_string()],
            },
            RoleRecord {
                name: "trader".to_string(),
                permissions: vec!["exchange_credentials.write".to_string()],
            },
        ])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_user_then_find_by_username_returns_it() {
        let repo = FakeUserRepository::new();
        let created = repo.create_user("alice", "hash123").await.unwrap();
        let found = repo.find_by_username("alice").await.unwrap().unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.password_hash, "hash123");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_user_rejects_duplicate_username() {
        let repo = FakeUserRepository::new();
        repo.create_user("alice", "hash1").await.unwrap();
        let result = repo.create_user("alice", "hash2").await;
        assert!(matches!(
            result,
            Err(UserRepositoryError::DuplicateUsername(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn find_by_username_returns_none_for_unknown_user() {
        let repo = FakeUserRepository::new();
        assert!(repo.find_by_username("ghost").await.unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn assign_roles_rejects_unknown_role() {
        let repo = seeded_repo();
        let user = repo.create_user("bob", "hash").await.unwrap();
        let result = repo
            .assign_roles(user.id, &["nonexistent".to_string()])
            .await;
        assert!(matches!(result, Err(UserRepositoryError::RoleNotFound(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permissions_for_user_flattens_and_dedups_across_roles() {
        let repo = seeded_repo();
        repo.create_role(
            "ops",
            &["users.manage".to_string(), "orders.manage".to_string()],
        )
        .await
        .unwrap();
        let user = repo.create_user("carol", "hash").await.unwrap();
        repo.assign_roles(user.id, &["admin".to_string(), "ops".to_string()])
            .await
            .unwrap();

        let mut perms = repo.permissions_for_user(user.id).await.unwrap();
        perms.sort();
        assert_eq!(perms, vec!["orders.manage", "roles.manage", "users.manage"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permissions_for_user_with_no_roles_is_empty() {
        let repo = seeded_repo();
        let user = repo.create_user("dave", "hash").await.unwrap();
        assert!(repo.permissions_for_user(user.id).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn user_count_reflects_created_users() {
        let repo = FakeUserRepository::new();
        assert_eq!(repo.user_count().await.unwrap(), 0);
        repo.create_user("alice", "hash").await.unwrap();
        assert_eq!(repo.user_count().await.unwrap(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn assign_roles_replaces_previous_assignment() {
        let repo = seeded_repo();
        let user = repo.create_user("erin", "hash").await.unwrap();
        repo.assign_roles(user.id, &["admin".to_string()])
            .await
            .unwrap();
        repo.assign_roles(user.id, &["trader".to_string()])
            .await
            .unwrap();
        assert_eq!(repo.roles_for_user(user.id).await.unwrap(), vec!["trader"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn seed_admin_if_empty_creates_user_with_admin_role() {
        let repo = seeded_repo();
        seed_admin_if_empty(&repo, "admin", "secret123")
            .await
            .unwrap();

        let user = repo.find_by_username("admin").await.unwrap().unwrap();
        assert_eq!(repo.roles_for_user(user.id).await.unwrap(), vec!["admin"]);
        assert_ne!(
            user.password_hash, "secret123",
            "password must be hashed, not stored plain"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn seed_admin_if_empty_is_noop_when_users_exist() {
        let repo = seeded_repo();
        repo.create_user("someone", "hash").await.unwrap();

        seed_admin_if_empty(&repo, "admin", "secret123")
            .await
            .unwrap();

        assert!(repo.find_by_username("admin").await.unwrap().is_none());
        assert_eq!(repo.user_count().await.unwrap(), 1);
    }
}
