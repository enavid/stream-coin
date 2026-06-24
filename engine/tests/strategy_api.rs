use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_http::ws;
use actix_web::App;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::infrastructure::db::order_repository::FakeOrderRepository;
use stream_coin::infrastructure::db::python_strategy_repository::FakePythonStrategyRepository;
use stream_coin::infrastructure::db::strategy_repository::{
    FakeStrategyRepository, StrategyRecord,
};
use stream_coin::order::entity::SafetyConfig;
use stream_coin::order::fake::FakeOrderAdapter;
use stream_coin::order::manager::{spawn_order_manager_listener, OrderManager};
use stream_coin::order::port::OrderAdapter;
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
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
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
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
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
async fn running_strategy_with_risk_reward_emits_signal_with_stop_loss_and_take_profit() {
    use chrono::Utc;
    use stream_coin::presentation::ws_message::PricePayload;

    let state = build_state();
    let broadcaster = state.broadcaster.clone();

    let mut srv =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut conn = srv.ws_at("/v1/ws").await.unwrap();

    let resp = srv
        .post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "test-spread-rr",
            "strategy_type": "spread_threshold",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {
                "threshold": 1000,
                "risk_reward": { "stop_pct": 0.02, "target_rr": 2.0 }
            }
        }))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let price_json = serde_json::to_string(&WsMessage::PriceUpdate(PricePayload {
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        bid: 175_000,
        ask: 177_000,
        timestamp: Utc::now(),
    }))
    .unwrap();
    broadcaster.send(price_json).unwrap();

    let deadline = Duration::from_millis(500);
    loop {
        match tokio::time::timeout(deadline, conn.next()).await {
            Ok(Some(Ok(ws::Frame::Text(bytes)))) => {
                let val: Value = serde_json::from_slice(&bytes).unwrap();
                if val["type"] == "signal" {
                    assert!(
                        val["stop_loss"].is_number(),
                        "signal must carry a computed stop_loss, got {:?}",
                        val["stop_loss"]
                    );
                    assert!(
                        val["take_profit"].is_number(),
                        "signal must carry a computed take_profit, got {:?}",
                        val["take_profit"]
                    );
                    // entry = ask = 177_000; risk = 177_000 * 0.02 = 3_540
                    assert_eq!(val["stop_loss"], 173_460);
                    assert_eq!(val["take_profit"], 184_080);
                    return;
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => panic!("WS client did not receive a signal frame within the deadline"),
        }
    }
}

#[actix_web::test]
async fn ws_client_receives_closed_trade_event_for_live_strategy() {
    use chrono::Utc;
    use stream_coin::candle::entity::CandlePayload;

    let state = build_state();
    let broadcaster = state.broadcaster.clone();

    let mut srv =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut conn = srv.ws_at("/v1/ws").await.unwrap();

    // price_delta can emit both Buy and Sell, unlike spread_threshold (always
    // Buy) — needed so the live preview actually has an opposite-side signal
    // to close the position with.
    let resp = srv
        .post("/v1/strategies/start")
        .send_json(&json!({
            "strategy_id": "test-live-preview",
            "strategy_type": "price_delta",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "params": {"window": 2, "threshold": 0.05}
        }))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let candle_json = |close: u64, time_offset_secs: i64| -> String {
        serde_json::to_string(&WsMessage::Candle(CandlePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: Utc::now() + chrono::Duration::seconds(time_offset_secs),
            open: close,
            high: close,
            low: close,
            close,
            volume: 10,
        }))
        .unwrap()
    };

    // Window of 2: candle[0] seeds history; candle[1] (+10% move) emits Buy
    // and opens the live position; candle[2] (-10% move from candle[1])
    // emits Sell, which LiveTradeTracker pairs into a ClosedTrade.
    broadcaster.send(candle_json(100_000, 0)).unwrap();
    broadcaster.send(candle_json(110_000, 60)).unwrap();
    broadcaster.send(candle_json(99_000, 120)).unwrap();

    let deadline = Duration::from_millis(500);
    loop {
        match tokio::time::timeout(deadline, conn.next()).await {
            Ok(Some(Ok(ws::Frame::Text(bytes)))) => {
                let val: Value = serde_json::from_slice(&bytes).unwrap();
                if val["type"] == "closed_trade" {
                    assert_eq!(val["strategy_id"], "test-live-preview");
                    assert_eq!(val["entry_price"], 110_000);
                    assert_eq!(val["exit_price"], 99_000);
                    assert!(val["pnl"].is_number());
                    return;
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => panic!("WS client did not receive a closed_trade frame within the deadline"),
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
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
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

fn build_state_with_deploy_support() -> (actix_web::web::Data<AppState>, Arc<FakeOrderRepository>) {
    let broadcaster = AppState::new_broadcaster();
    let repo = Arc::new(FakeOrderRepository::new());

    let mut order_adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
    order_adapters.insert(
        "tabdeal".to_string(),
        Arc::new(FakeOrderAdapter::new("tabdeal")),
    );
    let order_adapters = Arc::new(RwLock::new(order_adapters));

    let manager = Arc::new(OrderManager::with_poll_interval(
        order_adapters.clone(),
        repo.clone(),
        broadcaster.clone(),
        SafetyConfig {
            dry_run: true,
            min_confidence: 0.5,
            ..SafetyConfig::default()
        },
        Duration::from_millis(20),
    ));
    let _listener = spawn_order_manager_listener(Arc::clone(&manager), broadcaster.clone());

    let state = actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster,
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters,
        order_manager: Some(manager),
        python_strategy_repository: Some(Arc::new(FakePythonStrategyRepository::new())),
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    });
    (state, repo)
}

#[actix_web::test]
async fn deployed_strategy_produces_order() {
    use actix_http::ws;
    use futures_util::StreamExt;

    // Python code: read one candle from stdin, emit a buy signal, then exit
    let code = r#"
import sys, json, uuid
from datetime import datetime, timezone

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    print(json.dumps({
        "signal_id": str(uuid.uuid4()),
        "strategy_id": "will-be-overridden",
        "exchange": candle["exchange"],
        "pair": candle["pair"],
        "action": "buy",
        "confidence": 0.9,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }), flush=True)
    break
"#;

    let (state, order_repo) = build_state_with_deploy_support();
    let broadcaster = state.broadcaster.clone();

    let mut srv =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    // Connect WS client to receive order updates
    let mut ws_conn = srv.ws_at("/v1/ws").await.unwrap();

    // Deploy the Python strategy
    let resp = srv
        .post("/v1/strategies/deploy")
        .send_json(&json!({
            "name": "Test Buy Strategy",
            "code": code,
            "params": {}
        }))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "deploy must return 200");

    // Wait for subprocess to start
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Inject a candle event via the broadcaster
    let candle = WsMessage::Candle(stream_coin::candle::entity::CandlePayload {
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        interval: "1m".to_string(),
        time: chrono::Utc::now(),
        open: 58_000,
        high: 58_500,
        low: 57_800,
        close: 58_200,
        volume: 100,
    });
    broadcaster
        .send(serde_json::to_string(&candle).unwrap())
        .unwrap();

    // Wait for an OrderUpdate on the WS feed (dry-run mode)
    let deadline = Duration::from_secs(15);
    let mut saw_order_update = false;
    loop {
        match tokio::time::timeout(deadline, ws_conn.next()).await {
            Ok(Some(Ok(ws::Frame::Text(bytes)))) => {
                let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if val["type"] == "order_update" {
                    saw_order_update = true;
                    assert_eq!(val["status"], "dry_run");
                    assert_eq!(val["side"], "buy");
                    break;
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }

    assert!(
        saw_order_update || !order_repo.all_records().await.is_empty(),
        "deployed python strategy must produce an order (via WS or DB)"
    );
}
