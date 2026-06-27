//! Postgres integration tests for the user, credential, and subscription repos
//! (M19). The existing PG tests covered candles/orders/exchanges; the
//! user/credential/subscription SQL had none. All are `#[ignore]`d so the
//! default `cargo test` / `just check` run stays hermetic. Run with:
//!   `cargo test --tests -- --ignored`

use chrono::Utc;
use sqlx::PgPool;

use stream_coin::infrastructure::crypto::credential_cipher::EncryptedEnvelope;
use stream_coin::infrastructure::db::credential_repository::CredentialRepository;
use stream_coin::infrastructure::db::postgres::{
    PostgresCredentialRepository, PostgresSubscriptionRepository, PostgresUserRepository,
};
use stream_coin::infrastructure::db::subscription_repository::{
    SubscriptionRepository, SubscriptionRepositoryError,
};
use stream_coin::infrastructure::db::user_repository::{UserRepository, UserRepositoryError};

const DATABASE_URL_FALLBACK: &str = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";

async fn connect() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DATABASE_URL_FALLBACK.to_string());
    PgPool::connect(&url)
        .await
        .expect("failed to connect to postgres — see compose/postgres.yml")
}

/// Unique suffix so repeated runs never collide on UNIQUE(username) etc.
fn unique(tag: &str) -> String {
    format!(
        "test-{tag}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

async fn create_user(pool: &PgPool, tag: &str) -> i32 {
    let repo = PostgresUserRepository::new(pool.clone());
    repo.create_user(&unique(tag), "hash")
        .await
        .expect("create_user")
        .id
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_user_repository_round_trips() {
    let pool = connect().await;
    let repo = PostgresUserRepository::new(pool.clone());
    let username = unique("user-rt");

    let created = repo.create_user(&username, "hash-1").await.expect("create");
    assert_eq!(created.username, username);

    let found = repo
        .find_by_username(&username)
        .await
        .expect("find")
        .expect("user must exist");
    assert_eq!(found.id, created.id);
    assert_eq!(found.password_hash, "hash-1");
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_user_repository_duplicate_username_is_rejected() {
    let pool = connect().await;
    let repo = PostgresUserRepository::new(pool.clone());
    let username = unique("user-dup");

    repo.create_user(&username, "h")
        .await
        .expect("first create");
    let result = repo.create_user(&username, "h").await;
    assert!(
        matches!(result, Err(UserRepositoryError::DuplicateUsername(_))),
        "a duplicate username must map to DuplicateUsername, got {result:?}"
    );
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_user_repository_roles_and_permissions_flatten() {
    let pool = connect().await;
    let repo = PostgresUserRepository::new(pool.clone());
    let role = unique("role");

    repo.create_role(
        &role,
        &["orders.read".to_string(), "orders.write".to_string()],
    )
    .await
    .expect("create_role");

    let user_id = create_user(&pool, "perms").await;
    repo.assign_roles(user_id, std::slice::from_ref(&role))
        .await
        .expect("assign_roles");

    let mut perms = repo
        .permissions_for_user(user_id)
        .await
        .expect("permissions_for_user");
    perms.sort();
    assert_eq!(perms, vec!["orders.read", "orders.write"]);
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_credential_repository_round_trips_jsonb() {
    let pool = connect().await;
    let user_id = create_user(&pool, "cred").await;
    let repo = PostgresCredentialRepository::new(pool.clone());

    let envelope = EncryptedEnvelope {
        nonce: "nonce-abc".to_string(),
        ciphertext: "cipher-xyz".to_string(),
    };
    repo.upsert(user_id, "tabdeal", envelope.clone())
        .await
        .expect("upsert");

    // The envelope is stored as JSONB and must round-trip byte-for-byte.
    let fetched = repo
        .get(user_id, "tabdeal")
        .await
        .expect("get")
        .expect("credential must exist");
    assert_eq!(fetched.nonce, "nonce-abc");
    assert_eq!(fetched.ciphertext, "cipher-xyz");

    let listed = repo.list_for_user(user_id).await.expect("list");
    assert!(listed.iter().any(|c| c.exchange_name == "tabdeal"));

    repo.delete(user_id, "tabdeal").await.expect("delete");
    assert!(
        repo.get(user_id, "tabdeal").await.expect("get").is_none(),
        "credential must be gone after delete"
    );
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_credential_repository_upsert_replaces_existing() {
    let pool = connect().await;
    let user_id = create_user(&pool, "cred-upsert").await;
    let repo = PostgresCredentialRepository::new(pool.clone());

    repo.upsert(
        user_id,
        "tabdeal",
        EncryptedEnvelope {
            nonce: "n1".to_string(),
            ciphertext: "c1".to_string(),
        },
    )
    .await
    .expect("first upsert");
    repo.upsert(
        user_id,
        "tabdeal",
        EncryptedEnvelope {
            nonce: "n2".to_string(),
            ciphertext: "c2".to_string(),
        },
    )
    .await
    .expect("second upsert replaces");

    let fetched = repo.get(user_id, "tabdeal").await.unwrap().unwrap();
    assert_eq!(fetched.nonce, "n2", "upsert must overwrite, not duplicate");
    assert_eq!(repo.list_for_user(user_id).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_subscription_repository_round_trips() {
    let pool = connect().await;
    let user_id = create_user(&pool, "sub").await;
    let repo = PostgresSubscriptionRepository::new(pool.clone());
    let strategy = unique("strat");

    let created = repo
        .create(user_id, &strategy, None, None)
        .await
        .expect("create");
    assert_eq!(created.user_id, user_id);
    assert!(created.active);

    let listed = repo.list_for_user(user_id).await.expect("list");
    assert_eq!(listed.len(), 1);

    let updated = repo
        .update(created.id, false, None, None)
        .await
        .expect("update");
    assert!(!updated.active, "active flag must be updated");

    repo.delete(created.id).await.expect("delete");
    assert!(repo.list_for_user(user_id).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn duplicate_subscription_maps_to_conflict_via_sqlstate() {
    // M16 over real SQL: a duplicate (user, strategy) must be recognized via
    // SQLSTATE 23505 and mapped to AlreadySubscribed, not a generic Database error.
    let pool = connect().await;
    let user_id = create_user(&pool, "sub-dup").await;
    let repo = PostgresSubscriptionRepository::new(pool.clone());
    let strategy = unique("strat-dup");

    repo.create(user_id, &strategy, None, None)
        .await
        .expect("first subscribe");
    let result = repo.create(user_id, &strategy, None, None).await;
    assert!(
        matches!(
            result,
            Err(SubscriptionRepositoryError::AlreadySubscribed { .. })
        ),
        "a duplicate subscription must map to AlreadySubscribed, got {result:?}"
    );
}
