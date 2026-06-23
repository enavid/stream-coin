use chrono::{TimeZone, Utc};
use sqlx::PgPool;

use stream_coin::candle::entity::{Candle, Interval};
use stream_coin::infrastructure::db::candle_repository::CandleRepository;
use stream_coin::infrastructure::db::postgres::PostgresCandleRepository;

const DATABASE_URL_FALLBACK: &str = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";

async fn connect() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DATABASE_URL_FALLBACK.to_string());
    PgPool::connect(&url)
        .await
        .expect("failed to connect to postgres — see compose/postgres.yml")
}

/// Unique per test run so concurrent/repeated runs never collide on the
/// `(exchange, pair, interval, time)` unique constraint added in migration 0012.
fn unique_pair(test_name: &str) -> String {
    format!(
        "TEST-{test_name}-{}/IRT",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

fn candle(exchange: &str, pair: &str, interval: Interval, time_secs: i64, close: u64) -> Candle {
    Candle {
        exchange: exchange.to_string(),
        pair: pair.to_string(),
        interval,
        time: Utc.timestamp_opt(time_secs, 0).unwrap(),
        open: close,
        high: close + 10,
        low: close.saturating_sub(10),
        close,
        volume: 5,
    }
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_then_list_round_trips_a_candle() {
    let pool = connect().await;
    let repo = PostgresCandleRepository::new(pool);
    let pair = unique_pair("roundtrip");

    repo.upsert_candles(&[candle(
        "coinex",
        &pair,
        Interval::OneHour,
        1_700_000_000,
        100,
    )])
    .await
    .expect("upsert failed");

    let from = Utc.timestamp_opt(1_699_999_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_001_000, 0).unwrap();
    let result = repo
        .list_candles("coinex", &pair, "1h", from, to)
        .await
        .expect("list failed");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].close, 100);
    assert_eq!(result[0].exchange, "coinex");
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_candles_is_idempotent_against_real_db() {
    let pool = connect().await;
    let repo = PostgresCandleRepository::new(pool);
    let pair = unique_pair("idempotent");
    let c = candle("coinex", &pair, Interval::OneMinute, 1_700_100_000, 200);

    repo.upsert_candles(std::slice::from_ref(&c))
        .await
        .expect("first upsert failed");
    repo.upsert_candles(&[c])
        .await
        .expect("second upsert failed");

    let from = Utc.timestamp_opt(1_700_099_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_101_000, 0).unwrap();
    let result = repo
        .list_candles("coinex", &pair, "1m", from, to)
        .await
        .expect("list failed");

    assert_eq!(result.len(), 1, "re-upserting must not duplicate the row");
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_candles_updates_close_on_conflict_against_real_db() {
    let pool = connect().await;
    let repo = PostgresCandleRepository::new(pool);
    let pair = unique_pair("update-on-conflict");

    repo.upsert_candles(&[candle(
        "coinex",
        &pair,
        Interval::OneMinute,
        1_700_200_000,
        100,
    )])
    .await
    .expect("first upsert failed");
    repo.upsert_candles(&[candle(
        "coinex",
        &pair,
        Interval::OneMinute,
        1_700_200_000,
        999,
    )])
    .await
    .expect("second upsert failed");

    let from = Utc.timestamp_opt(1_700_199_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_201_000, 0).unwrap();
    let result = repo
        .list_candles("coinex", &pair, "1m", from, to)
        .await
        .expect("list failed");

    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].close, 999,
        "ON CONFLICT must update the existing row"
    );
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn upsert_candles_batches_multiple_rows_in_one_call() {
    let pool = connect().await;
    let repo = PostgresCandleRepository::new(pool);
    let pair = unique_pair("batch");

    repo.upsert_candles(&[
        candle("coinex", &pair, Interval::OneMinute, 1_700_300_000, 1),
        candle("coinex", &pair, Interval::OneMinute, 1_700_300_060, 2),
        candle("coinex", &pair, Interval::OneMinute, 1_700_300_120, 3),
    ])
    .await
    .expect("batch upsert failed");

    let from = Utc.timestamp_opt(1_700_299_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_301_000, 0).unwrap();
    let result = repo
        .list_candles("coinex", &pair, "1m", from, to)
        .await
        .expect("list failed");

    assert_eq!(result.len(), 3);
}

#[tokio::test]
#[ignore = "requires a live Postgres+TimescaleDB on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn list_candles_excludes_rows_outside_requested_pair() {
    let pool = connect().await;
    let repo = PostgresCandleRepository::new(pool);
    let pair_a = unique_pair("isolation-a");
    let pair_b = unique_pair("isolation-b");

    repo.upsert_candles(&[candle(
        "coinex",
        &pair_a,
        Interval::OneHour,
        1_700_400_000,
        1,
    )])
    .await
    .unwrap();
    repo.upsert_candles(&[candle(
        "coinex",
        &pair_b,
        Interval::OneHour,
        1_700_400_000,
        2,
    )])
    .await
    .unwrap();

    let from = Utc.timestamp_opt(1_700_399_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_401_000, 0).unwrap();
    let result = repo
        .list_candles("coinex", &pair_a, "1h", from, to)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].pair, pair_a);
}
