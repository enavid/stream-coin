use chrono::Utc;
use sqlx::PgPool;

use stream_coin::exchange::registry::TradingPairRecord;
use stream_coin::infrastructure::db::exchange_repository::ExchangeRepository;
use stream_coin::infrastructure::db::postgres::PostgresExchangeRepository;
use stream_coin::price::entity::MarketType;

const DATABASE_URL_FALLBACK: &str = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";

async fn connect() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DATABASE_URL_FALLBACK.to_string());
    PgPool::connect(&url)
        .await
        .expect("failed to connect to postgres — see compose/postgres.yml")
}

/// Unique per test run so concurrent/repeated runs never collide on the
/// `(exchange_id, base, quote, market_type)` unique constraint.
fn unique_base(test_name: &str) -> String {
    format!(
        "T{test_name}{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

fn pair(base: &str, active: bool) -> TradingPairRecord {
    TradingPairRecord {
        exchange_name: "coinex".to_string(),
        base: base.to_string(),
        quote: "USDT".to_string(),
        market_type: MarketType::Spot,
        active,
    }
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_pair_inserts_then_appears_in_load_all() {
    let pool = connect().await;
    let repo = PostgresExchangeRepository::new(pool);
    let base = unique_base("insert");

    repo.upsert_pair(&pair(&base, true))
        .await
        .expect("upsert failed");

    let (_, pairs) = repo.load_all().await.expect("load failed");
    assert!(
        pairs
            .iter()
            .any(|p| p.exchange_name == "coinex" && p.base == base),
        "newly upserted pair must appear in load_all"
    );
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_pair_is_idempotent_against_real_db() {
    let pool = connect().await;
    let repo = PostgresExchangeRepository::new(pool);
    let base = unique_base("idempotent");

    repo.upsert_pair(&pair(&base, true))
        .await
        .expect("first upsert failed");
    repo.upsert_pair(&pair(&base, true))
        .await
        .expect("second upsert failed");

    let (_, pairs) = repo.load_all().await.expect("load failed");
    let count = pairs
        .iter()
        .filter(|p| p.exchange_name == "coinex" && p.base == base)
        .count();
    assert_eq!(count, 1, "re-upserting must not duplicate the row");
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_pair_updates_active_flag_on_conflict() {
    let pool = connect().await;
    let repo = PostgresExchangeRepository::new(pool);
    let base = unique_base("update");

    repo.upsert_pair(&pair(&base, false))
        .await
        .expect("first upsert failed");
    repo.upsert_pair(&pair(&base, true))
        .await
        .expect("second upsert failed");

    let (_, pairs) = repo.load_all().await.expect("load failed");
    let found = pairs
        .iter()
        .find(|p| p.exchange_name == "coinex" && p.base == base)
        .expect("pair must exist");
    assert!(found.active, "ON CONFLICT must update active flag");
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_pair_is_noop_for_unknown_exchange_against_real_db() {
    let pool = connect().await;
    let repo = PostgresExchangeRepository::new(pool);
    let base = unique_base("unknown-exchange");

    let record = TradingPairRecord {
        exchange_name: "definitely-not-a-real-exchange".to_string(),
        base: base.clone(),
        quote: "USDT".to_string(),
        market_type: MarketType::Spot,
        active: true,
    };
    repo.upsert_pair(&record)
        .await
        .expect("upsert must not error");

    let (_, pairs) = repo.load_all().await.expect("load failed");
    assert!(
        !pairs.iter().any(|p| p.base == base),
        "unknown exchange must not create a pair row"
    );
}
