use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_http::ws;
use actix_web::App;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::infrastructure::db::strategy_repository::{
    FakeStrategyRepository, StrategyRecord,
};
use stream_coin::presentation::handlers::strategy_handler::restore_strategies;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};
use stream_coin::wire_message::WsMessage;

fn build_state() -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
    })
}

fn build_state_with_strategy_repo(
    repo: Arc<FakeStrategyRepository>,
) -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: Some(repo),
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
    })
}

#[actix_web::test]
async fn start_strategy_returns_200() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "test-spread",
            "strategy_type": "spread_threshold",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {"threshold": 1000}
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn running_strategy_emits_signal_on_price_tick() {
    use chrono::Utc;
    use stream_coin::presentation::ws_message::PricePayload;

    let state = build_state();
    let broadcaster = state.broadcaster.clone();

    let mut srv =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    // Connect WS client before starting the strategy
    let mut conn = srv.ws_at("/v1/ws").await.unwrap();

    // Start spread_threshold strategy (threshold = 1000)
    let resp = srv
        .post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "test-spread",
            "strategy_type": "spread_threshold",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {"threshold": 1000}
        }))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Inject a price with spread = 2000 (> threshold 1000)
    let price_json = serde_json::to_string(&WsMessage::PriceUpdate(PricePayload {
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        bid: 175_000,
        ask: 177_000,
        timestamp: Utc::now(),
    }))
    .unwrap();
    broadcaster.send(price_json).unwrap();

    // Drain messages until we see a signal (or timeout)
    let deadline = Duration::from_millis(500);
    loop {
        match tokio::time::timeout(deadline, conn.next()).await {
            Ok(Some(Ok(ws::Frame::Text(bytes)))) => {
                let val: Value = serde_json::from_slice(&bytes).unwrap();
                if val["type"] == "signal" {
                    assert_eq!(val["action"], "buy");
                    assert_eq!(val["exchange"], "tabdeal");
                    return;
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => panic!("WS client did not receive a signal frame within the deadline"),
        }
    }
}

#[actix_web::test]
async fn start_strategy_with_unknown_type_returns_error() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "test-unknown",
            "strategy_type": "neural_network",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {}
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_strategy_duplicate_id_returns_error() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let payload = json!({
        "strategy_id": "dup-id",
        "strategy_type": "spread_threshold",
        "exchange": "tabdeal",
        "pair": "USDT/IRT",
        "params": {"threshold": 1000}
    });

    let first = app
        .post("/v1/strategies/start")
        .send_json(&payload)
        .await
        .unwrap();
    assert_eq!(first.status(), 200, "first start must succeed");

    let second = app
        .post("/v1/strategies/start")
        .send_json(&payload)
        .await
        .unwrap();
    assert_eq!(
        second.status(),
        400,
        "second start with same id must be rejected"
    );
}

#[actix_web::test]
async fn stop_strategy_returns_200() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    app.post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "to-stop",
            "strategy_type": "spread_threshold",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {"threshold": 1000}
        }))
        .await
        .unwrap();

    let resp = app
        .post("/v1/strategies/stop")
        .send_json(&json!({"strategy_id": "to-stop"}))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn stop_strategy_not_found_returns_error() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/strategies/stop")
        .send_json(&json!({"strategy_id": "ghost-strategy"}))
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn list_strategies_returns_running_strategies() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    app.post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "listed-strategy",
            "strategy_type": "spread_threshold",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {"threshold": 1000}
        }))
        .await
        .unwrap();

    let mut resp = app.get("/v1/strategies").send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let body: Value = resp.json().await.unwrap();
    let strategies = body["data"]["strategies"].as_array().unwrap();
    assert!(
        strategies
            .iter()
            .any(|s| s["strategy_id"] == "listed-strategy"),
        "started strategy must appear in the list"
    );
}

#[actix_web::test]
async fn register_strategy_builtin_returns_200() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/strategies/register")
        .send_json(&json!({
            "strategy_id": "my-arb",
            "name": "My Arb Strategy",
            "strategy_type": "builtin"
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn register_strategy_invalid_type_returns_error() {
    let state = build_state();
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/strategies/register")
        .send_json(&json!({
            "strategy_id": "bad",
            "name": "Bad",
            "strategy_type": "unknown_type"
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_strategy_without_token_returns_401() {
    use stream_coin::presentation::shared::app_state::AppState;

    let state = actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: Some(Arc::new("test-secret".to_string())),
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
    });

    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "no-auth",
            "strategy_type": "spread_threshold",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {"threshold": 1000}
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[actix_web::test]
async fn strategy_restored_on_engine_restart() {
    use chrono::Utc;

    let repo = Arc::new(FakeStrategyRepository::with_records(vec![StrategyRecord {
        strategy_id: "seeded-spread".to_string(),
        strategy_type: "spread_threshold".to_string(),
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        params_json: json!({"threshold": 500}),
        started_at: Utc::now(),
    }]));

    let state = build_state_with_strategy_repo(Arc::clone(&repo));

    // Simulate restart: restore strategies from the repository
    restore_strategies(&state).await;

    // Verify the strategy is running
    let running = state.running_strategies.lock().await;
    assert!(
        running.contains_key("seeded-spread"),
        "restored strategy must appear in running_strategies"
    );
}
