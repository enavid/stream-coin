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
use stream_coin::order::entity::SafetyConfig;
use stream_coin::order::fake::FakeOrderAdapter;
use stream_coin::order::manager::OrderManager;
use stream_coin::order::port::{OrderAdapter, OrderStatusResult};
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};
use stream_coin::wire_message::WsMessage;

fn build_state_with_order_manager(
    safety_config: SafetyConfig,
    adapter: FakeOrderAdapter,
) -> (actix_web::web::Data<AppState>, Arc<FakeOrderRepository>) {
    let broadcaster = AppState::new_broadcaster();
    let repo = Arc::new(FakeOrderRepository::new());

    let mut order_adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
    order_adapters.insert("tabdeal".to_string(), Arc::new(adapter));
    let order_adapters = Arc::new(RwLock::new(order_adapters));

    let manager = Arc::new(OrderManager::with_poll_interval(
        order_adapters.clone(),
        repo.clone(),
        broadcaster.clone(),
        safety_config,
        Duration::from_millis(20),
    ));

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
        python_strategy_repository: None,
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        subscription_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    });

    (state, repo)
}

fn live_config() -> SafetyConfig {
    SafetyConfig {
        dry_run: false,
        min_confidence: 0.7,
        max_position_size: rust_decimal::Decimal::new(10_000, 0),
        default_order_quantity: rust_decimal::Decimal::new(100, 0),
        circuit_breaker_max_orders: 20,
        circuit_breaker_window_secs: 60,
    }
}

// ---------------------------------------------------------------------------
// REST — place order

#[actix_web::test]
async fn place_order_returns_200_and_client_order_id() {
    let adapter = FakeOrderAdapter::new("tabdeal");
    adapter.will_succeed_with("exch-001").await;
    let (state, _repo) = build_state_with_order_manager(live_config(), adapter);

    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let mut resp = resp;
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert!(
        body["data"]["client_order_id"].is_string(),
        "response must include client_order_id"
    );
}

#[actix_web::test]
async fn place_order_invalid_side_returns_400() {
    let (state, _repo) =
        build_state_with_order_manager(live_config(), FakeOrderAdapter::new("tabdeal"));
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "long",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn signal_produces_order_in_db() {
    use stream_coin::wire_message::SignalPayload;

    let adapter = FakeOrderAdapter::new("tabdeal");
    adapter.will_succeed_with("exch-sig-001").await;

    let broadcaster = AppState::new_broadcaster();
    let repo = Arc::new(FakeOrderRepository::new());
    let mut order_adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
    order_adapters.insert("tabdeal".to_string(), Arc::new(adapter));
    let order_adapters = Arc::new(RwLock::new(order_adapters));

    let cfg = SafetyConfig {
        dry_run: false,
        min_confidence: 0.5,
        ..live_config()
    };
    let manager = Arc::new(OrderManager::with_poll_interval(
        order_adapters.clone(),
        repo.clone(),
        broadcaster.clone(),
        cfg,
        Duration::from_millis(20),
    ));

    // Process a signal directly (simulates strategy runner emitting a signal)
    let signal = SignalPayload {
        signal_id: uuid::Uuid::new_v4().to_string(),
        strategy_id: "test-strategy".to_string(),
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        action: "buy".to_string(),
        confidence: 0.9,
        timestamp: chrono::Utc::now(),
        stop_loss: None,
        take_profit: None,
    };

    manager.process_signal(&signal).await.unwrap();

    let records = repo.all_records().await;
    assert_eq!(records.len(), 1, "one order must be in the DB after signal");
    assert_eq!(records[0].status, "open");
    assert_eq!(
        records[0].exchange_order_id.as_deref(),
        Some("exch-sig-001")
    );
    assert_eq!(records[0].strategy_id.as_deref(), Some("test-strategy"));
}

#[actix_web::test]
async fn position_limit_blocks_oversized_order() {
    let adapter = FakeOrderAdapter::new("tabdeal");
    let cfg = SafetyConfig {
        dry_run: false,
        max_position_size: rust_decimal::Decimal::new(50, 0),
        default_order_quantity: rust_decimal::Decimal::new(100, 0),
        min_confidence: 0.0,
        ..live_config()
    };
    let (state, repo) = build_state_with_order_manager(cfg, adapter);
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "order exceeding position limit must return 400"
    );
    let records = repo.all_records().await;
    assert!(
        records.is_empty(),
        "no order must be persisted when blocked by position limit"
    );
}

#[actix_web::test]
async fn list_orders_returns_placed_orders() {
    let adapter = FakeOrderAdapter::new("tabdeal");
    adapter.will_succeed_with("exch-list-001").await;
    let (state, _repo) = build_state_with_order_manager(live_config(), adapter);
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    app.post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "50"
        }))
        .await
        .unwrap();

    let mut resp = app.get("/v1/orders").send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let orders = body["data"]["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0]["exchange"], "tabdeal");
    assert_eq!(orders[0]["status"], "open");
}

#[actix_web::test]
async fn circuit_breaker_reset_endpoint_returns_200() {
    let (state, _repo) =
        build_state_with_order_manager(live_config(), FakeOrderAdapter::new("tabdeal"));
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut resp = app
        .post("/v1/admin/circuit-breaker/reset")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["data"]["reset"], true);
}

// ---------------------------------------------------------------------------
// WebSocket — order_update broadcast

#[actix_web::test]
async fn ws_client_receives_order_update_on_placement() {
    let adapter = FakeOrderAdapter::new("tabdeal");
    adapter.will_succeed_with("exch-ws-001").await;
    let (state, _repo) = build_state_with_order_manager(live_config(), adapter);

    let mut app =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    // Connect WS client
    let mut ws = app.ws_at("/v1/ws").await.unwrap();

    // Place order via REST
    app.post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    // Read WS frames until we find an order_update
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for order_update WS message"
        );
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(ws::Frame::Text(bytes)))) => {
                let text = std::str::from_utf8(&bytes).unwrap();
                let msg: WsMessage = match serde_json::from_str(text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if let WsMessage::OrderUpdate(payload) = msg {
                    assert_eq!(payload.status, "open");
                    assert_eq!(payload.order_id, "exch-ws-001");
                    assert!(payload.fill_price.is_none());
                    break;
                }
            }
            _ => continue,
        }
    }
}

#[actix_web::test]
async fn ws_client_receives_order_update_on_fill() {
    let adapter = FakeOrderAdapter::new("tabdeal");
    adapter.will_succeed_with("exch-fill-001").await;
    // After placement, status polls return Filled immediately
    adapter
        .will_return_status(OrderStatusResult::filled(rust_decimal::Decimal::new(
            58_000, 0,
        )))
        .await;

    let (state, _repo) = build_state_with_order_manager(live_config(), adapter);

    let mut app =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut ws = app.ws_at("/v1/ws").await.unwrap();

    app.post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    // Expect two order_update frames: open (placement) then filled (poll)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut saw_open = false;
    let mut saw_filled = false;

    loop {
        if saw_open && saw_filled {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out: saw_open={saw_open}, saw_filled={saw_filled}"
        );
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(ws::Frame::Text(bytes)))) => {
                let text = std::str::from_utf8(&bytes).unwrap();
                if let Ok(WsMessage::OrderUpdate(payload)) = serde_json::from_str(text) {
                    match payload.status.as_str() {
                        "open" => saw_open = true,
                        "filled" => saw_filled = true,
                        _ => {}
                    }
                }
            }
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Input validation — exchange and pair

#[actix_web::test]
async fn place_order_unknown_exchange_returns_400() {
    let (state, _repo) =
        build_state_with_order_manager(live_config(), FakeOrderAdapter::new("tabdeal"));
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "hitobit",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "unknown exchange must be rejected with 400"
    );
}

#[actix_web::test]
async fn place_order_malformed_pair_returns_400() {
    let (state, _repo) =
        build_state_with_order_manager(live_config(), FakeOrderAdapter::new("tabdeal"));
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal",
            "pair": "USDTIRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "pair without '/' must be rejected with 400"
    );
}

// ---------------------------------------------------------------------------
// Authorization (C1/C4) — end-to-end through the real JWT middleware.

fn build_authed_state(
    safety_config: SafetyConfig,
    adapter: FakeOrderAdapter,
    jwt_secret: &str,
) -> actix_web::web::Data<AppState> {
    let broadcaster = AppState::new_broadcaster();
    let repo = Arc::new(FakeOrderRepository::new());

    let mut order_adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
    order_adapters.insert("tabdeal".to_string(), Arc::new(adapter));
    let order_adapters = Arc::new(RwLock::new(order_adapters));

    let manager = Arc::new(OrderManager::with_poll_interval(
        order_adapters.clone(),
        repo.clone(),
        broadcaster.clone(),
        safety_config,
        Duration::from_millis(20),
    ));

    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster,
        jwt_secret: Some(Arc::new(jwt_secret.to_string())),
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters,
        order_manager: Some(manager),
        python_strategy_repository: None,
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        subscription_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    })
}

const TEST_SECRET: &str = "order-api-test-secret";

fn token(perms: &[&str]) -> String {
    use stream_coin::presentation::middlewares::jwt::mint_token_with_permissions;
    let perms: Vec<String> = perms.iter().map(|s| s.to_string()).collect();
    mint_token_with_permissions("7", TEST_SECRET, 3600, &perms)
}

#[actix_web::test]
async fn place_order_without_token_is_unauthorized_401() {
    let state = build_authed_state(live_config(), FakeOrderAdapter::new("tabdeal"), TEST_SECRET);
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .send_json(&json!({
            "exchange": "tabdeal", "pair": "USDT/IRT",
            "side": "buy", "type": "market", "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        401,
        "no token must be rejected before any order logic"
    );
}

#[actix_web::test]
async fn place_order_without_orders_manage_is_forbidden_403() {
    let state = build_authed_state(live_config(), FakeOrderAdapter::new("tabdeal"), TEST_SECRET);
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .insert_header((
            "Authorization",
            format!("Bearer {}", token(&["subscriptions.read"])),
        ))
        .send_json(&json!({
            "exchange": "tabdeal", "pair": "USDT/IRT",
            "side": "buy", "type": "market", "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        403,
        "a token lacking orders.manage must be forbidden"
    );
}

#[actix_web::test]
async fn place_order_with_orders_manage_is_authorized() {
    let adapter = FakeOrderAdapter::new("tabdeal");
    adapter.will_succeed_with("exch-auth-1").await;
    let state = build_authed_state(live_config(), adapter, TEST_SECRET);
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = app
        .post("/v1/orders/place")
        .insert_header((
            "Authorization",
            format!("Bearer {}", token(&["orders.manage"])),
        ))
        .send_json(&json!({
            "exchange": "tabdeal", "pair": "USDT/IRT",
            "side": "buy", "type": "market", "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "orders.manage must authorize placement");
}

#[actix_web::test]
async fn reset_circuit_breaker_requires_orders_admin_not_just_manage() {
    let state = build_authed_state(live_config(), FakeOrderAdapter::new("tabdeal"), TEST_SECRET);
    let app = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    // orders.manage is NOT enough for the safety control.
    let forbidden = app
        .post("/v1/admin/circuit-breaker/reset")
        .insert_header((
            "Authorization",
            format!("Bearer {}", token(&["orders.manage"])),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        forbidden.status(),
        403,
        "circuit-breaker reset must require orders.admin"
    );

    // orders.admin succeeds.
    let allowed = app
        .post("/v1/admin/circuit-breaker/reset")
        .insert_header((
            "Authorization",
            format!("Bearer {}", token(&["orders.admin"])),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        allowed.status(),
        200,
        "orders.admin must authorize the reset"
    );
}
