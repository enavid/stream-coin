//! Postgres `CircuitBreakerStore` integration tests (M9).
//!
//! Require a live Postgres on localhost:5432 (see `compose/postgres.yml`) and
//! are `#[ignore]`d so the default run stays hermetic. Run with:
//!   `cargo test --tests -- --ignored`

use sqlx::PgPool;

use stream_coin::infrastructure::db::postgres::PostgresCircuitBreakerStore;
use stream_coin::order::circuit_breaker_store::CircuitBreakerStore;

const DATABASE_URL_FALLBACK: &str = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";

async fn connect() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DATABASE_URL_FALLBACK.to_string());
    PgPool::connect(&url)
        .await
        .expect("failed to connect to postgres — see compose/postgres.yml")
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_circuit_breaker_store_persists_and_loads_trip() {
    let store = PostgresCircuitBreakerStore::new(connect().await);

    // Leave the shared singleton row in a known state regardless of test order.
    store.set_tripped(true).await.expect("set tripped");
    assert!(
        store.load_tripped().await.expect("load"),
        "a persisted trip must read back as tripped"
    );

    store.set_tripped(false).await.expect("clear");
    assert!(
        !store.load_tripped().await.expect("load"),
        "a cleared trip must read back as not tripped"
    );
}
