//! Postgres `OrderRepository` integration tests (M11).
//!
//! These require a live Postgres+TimescaleDB on localhost:5432 (see
//! `compose/postgres.yml`) and are `#[ignore]`d so the default `cargo test` /
//! `just check` run stays hermetic. Run with:
//!   `cargo test --tests -- --ignored`

use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::PgPool;

use stream_coin::infrastructure::db::order_repository::{OrderRecord, OrderRepository};
use stream_coin::infrastructure::db::postgres::PostgresOrderRepository;

const DATABASE_URL_FALLBACK: &str = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";

async fn connect() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DATABASE_URL_FALLBACK.to_string());
    PgPool::connect(&url)
        .await
        .expect("failed to connect to postgres — see compose/postgres.yml")
}

/// A unique client_order_id so reruns never collide on the UNIQUE constraint.
fn unique_coid(tag: &str) -> String {
    format!(
        "test-{tag}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

fn record(coid: &str, user_id: Option<i32>, side: &str, status: &str, qty: i64) -> OrderRecord {
    let now = Utc::now();
    OrderRecord {
        id: None,
        user_id,
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        side: side.to_string(),
        order_type: "market".to_string(),
        quantity: Decimal::new(qty, 0),
        filled_quantity: Decimal::ZERO,
        price: None,
        status: status.to_string(),
        exchange_order_id: None,
        client_order_id: coid.to_string(),
        strategy_id: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_order_repository_round_trips_order() {
    let repo = PostgresOrderRepository::new(connect().await);
    let coid = unique_coid("roundtrip");
    let mut rec = record(&coid, Some(101), "buy", "open", 100);
    rec.price = Some(Decimal::new(58_000, 0));
    rec.strategy_id = Some("spread-1".to_string());

    let id = repo.insert(&rec).await.expect("insert");
    assert!(id > 0);

    let fetched = repo.get_by_client_order_id(&coid).await.expect("get");
    assert_eq!(fetched.client_order_id, coid);
    assert_eq!(fetched.user_id, Some(101));
    assert_eq!(fetched.quantity, Decimal::new(100, 0));
    assert_eq!(fetched.price, Some(Decimal::new(58_000, 0)));
    assert_eq!(fetched.status, "open");
    assert_eq!(fetched.strategy_id.as_deref(), Some("spread-1"));
    assert_eq!(fetched.filled_quantity, Decimal::ZERO);
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_order_repository_get_unknown_is_not_found() {
    let repo = PostgresOrderRepository::new(connect().await);
    let result = repo.get_by_client_order_id(&unique_coid("missing")).await;
    assert!(matches!(
        result,
        Err(stream_coin::infrastructure::db::order_repository::OrderRepositoryError::NotFound(_))
    ));
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_order_repository_update_status_preserves_filled_quantity_on_cancel() {
    // M7 over real SQL: a partial fill recorded by the poller must survive a
    // cancel (which passes None for filled_quantity), and net_position must keep
    // that residual inventory rather than dropping it to zero.
    let repo = PostgresOrderRepository::new(connect().await);
    let coid = unique_coid("partialcancel");
    let user = Some(202);
    repo.insert(&record(&coid, user, "buy", "open", 100))
        .await
        .expect("insert");

    repo.update_status(
        &coid,
        "partially_filled",
        Some("exch-1"),
        None,
        Some(Decimal::new(40, 0)),
    )
    .await
    .expect("record partial fill");
    repo.update_status(&coid, "cancelled", None, None, None)
        .await
        .expect("cancel");

    let fetched = repo.get_by_client_order_id(&coid).await.expect("get");
    assert_eq!(fetched.status, "cancelled");
    assert_eq!(
        fetched.filled_quantity,
        Decimal::new(40, 0),
        "cancel must not erase the partial fill"
    );
    let net = repo
        .net_position(user, "tabdeal", "USDT/IRT")
        .await
        .expect("net_position");
    assert_eq!(
        net,
        Decimal::new(40, 0),
        "cancelled-but-partially-filled inventory must remain in the position"
    );
}

#[tokio::test]
#[ignore = "requires a live Postgres on localhost:5432 (see compose/postgres.yml); run with `cargo test --tests -- --ignored`"]
async fn pg_order_repository_net_position_is_scoped_per_user() {
    // M8 over real SQL: one user's exposure must not count against another's.
    let repo = PostgresOrderRepository::new(connect().await);
    let u1 = Some(Utc::now().timestamp_subsec_nanos() as i32 | 1);
    let u2 = u1.map(|v| v.wrapping_add(1));
    let pair = format!("U{}/IRT", Utc::now().timestamp_nanos_opt().unwrap_or(0));

    let mut a = record(&unique_coid("u1"), u1, "buy", "filled", 100);
    a.pair = pair.clone();
    let mut b = record(&unique_coid("u2"), u2, "buy", "filled", 30);
    b.pair = pair.clone();
    repo.insert(&a).await.expect("insert u1");
    repo.insert(&b).await.expect("insert u2");

    assert_eq!(
        repo.net_position(u1, "tabdeal", &pair).await.unwrap(),
        Decimal::new(100, 0)
    );
    assert_eq!(
        repo.net_position(u2, "tabdeal", &pair).await.unwrap(),
        Decimal::new(30, 0)
    );
    assert_eq!(
        repo.net_position(None, "tabdeal", &pair).await.unwrap(),
        Decimal::ZERO,
        "the system bucket sees neither user's orders"
    );
}
