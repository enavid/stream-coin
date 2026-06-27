//! Postgres `PythonStrategyRepository` integration tests (M12).
//!
//! These require a live Postgres on localhost:5432 (see `compose/postgres.yml`)
//! and are `#[ignore]`d so the default `cargo test` / `just check` run stays
//! hermetic. Run with:
//!   `cargo test --tests -- --ignored`

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;

use stream_coin::infrastructure::db::postgres::PostgresPythonStrategyRepository;
use stream_coin::infrastructure::db::python_strategy_repository::{
    PythonStrategyRecord, PythonStrategyRepository, PythonStrategyRepositoryError,
};

const DATABASE_URL_FALLBACK: &str = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";

async fn connect() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DATABASE_URL_FALLBACK.to_string());
    PgPool::connect(&url)
        .await
        .expect("failed to connect to postgres — see compose/postgres.yml")
}

fn unique_id(tag: &str) -> String {
    format!(
        "test-{tag}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

fn record(strategy_id: &str) -> PythonStrategyRecord {
    PythonStrategyRecord {
        strategy_id: strategy_id.to_string(),
        name: "Test Strategy".to_string(),
        code: "print('hello')".to_string(),
        params_json: json!({"threshold": 1000}),
        created_at: Utc::now(),
    }
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_python_strategy_repository_round_trips() {
    let repo = PostgresPythonStrategyRepository::new(connect().await);
    let id = unique_id("roundtrip");
    let rec = record(&id);

    repo.save(&rec).await.expect("save");

    let fetched = repo.get(&id).await.expect("get");
    assert_eq!(fetched.strategy_id, id);
    assert_eq!(fetched.name, "Test Strategy");
    assert_eq!(fetched.code, "print('hello')");
    assert_eq!(fetched.params_json, json!({"threshold": 1000}));

    repo.remove(&id).await.expect("remove");
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_python_strategy_repository_get_unknown_is_not_found() {
    let repo = PostgresPythonStrategyRepository::new(connect().await);
    let result = repo.get(&unique_id("missing")).await;
    assert!(matches!(
        result,
        Err(PythonStrategyRepositoryError::NotFound(_))
    ));
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_python_strategy_repository_save_is_idempotent_upsert() {
    // Re-saving the same strategy_id must replace, not violate UNIQUE.
    let repo = PostgresPythonStrategyRepository::new(connect().await);
    let id = unique_id("upsert");

    repo.save(&record(&id)).await.expect("first save");

    let mut updated = record(&id);
    updated.name = "Renamed".to_string();
    updated.code = "print('v2')".to_string();
    repo.save(&updated)
        .await
        .expect("re-save same id must upsert");

    let fetched = repo.get(&id).await.expect("get");
    assert_eq!(fetched.name, "Renamed");
    assert_eq!(fetched.code, "print('v2')");

    repo.remove(&id).await.expect("remove");
}
