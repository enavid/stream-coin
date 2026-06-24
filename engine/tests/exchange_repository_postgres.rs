use chrono::Utc;
use sqlx::PgPool;

use stream_coin::exchange::registry::TradingPairRecord;
use stream_coin::infrastructure::db::exchange_repository::{
    ExchangeRepository, ExchangeRepositoryError,
};
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
/// `(exchange_id, base_asset_id, quote_asset_id, market_type)` unique
/// constraint. Also inserts a matching row into `assets` (migration `0013`)
/// since `trading_pairs.base`/`quote` are now FKs into it (migration `0014`)
/// — `upsert_pair` rejects any symbol that isn't a seeded asset.
async fn unique_base(pool: &PgPool, test_name: &str) -> String {
    let symbol = format!(
        "T{test_name}{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    sqlx::query("INSERT INTO assets (symbol, display_name, decimals) VALUES ($1, $1, 8)")
        .bind(&symbol)
        .execute(pool)
        .await
        .expect("failed to seed test asset");
    symbol
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
    let base = unique_base(&pool, "insert").await;
    let repo = PostgresExchangeRepository::new(pool);

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
    let base = unique_base(&pool, "idempotent").await;
    let repo = PostgresExchangeRepository::new(pool);

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
    let base = unique_base(&pool, "update").await;
    let repo = PostgresExchangeRepository::new(pool);

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
    let base = unique_base(&pool, "unknown-exchange").await;
    let repo = PostgresExchangeRepository::new(pool);

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

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_pair_returns_unknown_asset_error_for_unseeded_symbol() {
    let pool = connect().await;
    let repo = PostgresExchangeRepository::new(pool);

    let result = repo
        .upsert_pair(&pair("DEFINITELY-NOT-A-REAL-ASSET", true))
        .await;

    match result {
        Err(ExchangeRepositoryError::UnknownAsset(symbol)) => {
            assert_eq!(symbol, "DEFINITELY-NOT-A-REAL-ASSET");
        }
        other => panic!("expected UnknownAsset, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn load_all_resolves_base_and_quote_from_asset_symbols() {
    let pool = connect().await;
    let base = unique_base(&pool, "resolve").await;
    let repo = PostgresExchangeRepository::new(pool);

    repo.upsert_pair(&pair(&base, true))
        .await
        .expect("upsert failed");

    let (_, pairs) = repo.load_all().await.expect("load failed");
    let found = pairs
        .iter()
        .find(|p| p.exchange_name == "coinex" && p.base == base)
        .expect("pair must exist");
    assert_eq!(
        found.quote, "USDT",
        "quote must resolve to the asset symbol"
    );
}
