use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::App;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::infrastructure::crypto::password::hash_password;
use stream_coin::infrastructure::db::order_repository::FakeOrderRepository;
use stream_coin::infrastructure::db::subscription_repository::{
    FakeSubscriptionRepository, SubscriptionRepository,
};
use stream_coin::infrastructure::db::user_repository::{
    FakeUserRepository, RoleRecord, UserRepository,
};
use stream_coin::order::credential_resolver::FakeCredentialResolver;
use stream_coin::order::entity::SafetyConfig;
use stream_coin::order::fake::FakeOrderAdapter;
use stream_coin::order::manager::OrderManager;
use stream_coin::order::port::OrderAdapter;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

// ---------------------------------------------------------------------------
// Test state builder

fn admin_repo() -> Arc<FakeUserRepository> {
    Arc::new(FakeUserRepository::with_roles(vec![RoleRecord {
        name: "admin".to_string(),
        permissions: vec!["orders.admin".to_string()],
    }]))
}

fn build_state(
    order_manager: Arc<OrderManager>,
    sub_repo: Arc<FakeSubscriptionRepository>,
    user_repo: Arc<FakeUserRepository>,
) -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
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
        order_adapters: Arc::new(RwLock::new(HashMap::new())),
        order_manager: Some(order_manager),
        python_strategy_repository: None,
        candle_repository: None,
        historical_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        subscription_repository: Some(sub_repo),
        user_repository: Some(user_repo),
        credential_repository: None,
        credential_cipher: None,
    })
}

async fn login_as_admin(user_repo: &FakeUserRepository, srv: &actix_test::TestServer) -> String {
    user_repo
        .create_user("admin", &hash_password("secret"))
        .await
        .unwrap();
    let admin = user_repo.find_by_username("admin").await.unwrap().unwrap();
    user_repo
        .assign_roles(admin.id, &["admin".to_string()])
        .await
        .unwrap();

    let mut resp = srv
        .post("/v1/auth/token")
        .send_json(&json!({"username": "admin", "password": "secret"}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "admin login must succeed");
    let body: Value = resp.json().await.unwrap();
    body["data"]["token"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Test: admin_manual_order_uses_user_api_keys

#[actix_web::test]
async fn admin_manual_order_uses_user_api_keys() {
    let user_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
    let resolver = Arc::new(FakeCredentialResolver::returning(
        Arc::clone(&user_adapter) as Arc<dyn OrderAdapter>
    ));

    let (broadcaster, _) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(
        OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(FakeOrderRepository::new()),
            broadcaster,
            SafetyConfig {
                dry_run: false,
                default_order_quantity: Decimal::new(100, 0),
                max_position_size: Decimal::new(50_000, 0),
                min_confidence: 0.7,
                circuit_breaker_max_orders: 100,
                circuit_breaker_window_secs: 60,
            },
            Duration::from_millis(10),
        )
        .with_credential_resolver(resolver),
    );

    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = admin_repo();
    let state = build_state(manager, sub_repo, Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = login_as_admin(&user_repo, &srv).await;

    let mut resp = srv
        .post("/v1/admin/orders/place")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({
            "user_id": 42,
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "side": "buy",
            "type": "market",
            "quantity": "100"
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "admin order placement must succeed");
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["data"]["client_order_id"].as_str().is_some(),
        "response must include client_order_id"
    );

    assert_eq!(
        user_adapter.placed_count().await,
        1,
        "order must have been placed via the user's per-credential adapter"
    );
}

// ---------------------------------------------------------------------------
// Test: admin_can_halt_all_strategies_for_user

#[actix_web::test]
async fn admin_can_halt_all_strategies_for_user() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    // User 5 has two active subscriptions
    sub_repo.create(5, "spread-1", None, None).await.unwrap();
    sub_repo.create(5, "rsi-2", None, None).await.unwrap();
    // User 7 has one subscription — must not be touched
    sub_repo.create(7, "spread-1", None, None).await.unwrap();

    let (broadcaster, _) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(OrderManager::with_poll_interval(
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(FakeOrderRepository::new()),
        broadcaster,
        SafetyConfig::default(),
        Duration::from_millis(10),
    ));

    let user_repo = admin_repo();
    let state = build_state(manager, Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = login_as_admin(&user_repo, &srv).await;

    let mut resp = srv
        .post("/v1/admin/strategies/halt")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"user_id": 5}))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["data"]["halted"], 2,
        "both of user 5's subscriptions must be halted"
    );

    // Verify state: user 5 subs are inactive, user 7 sub is still active
    let user5 = sub_repo.list_for_user(5).await.unwrap();
    assert!(
        user5.iter().all(|s| !s.active),
        "all user 5 subscriptions must be deactivated"
    );
    let user7 = sub_repo.list_for_user(7).await.unwrap();
    assert!(
        user7.iter().all(|s| s.active),
        "user 7 subscription must remain active"
    );
}

// ---------------------------------------------------------------------------
// Test: unauthenticated requests are rejected

#[actix_web::test]
async fn admin_place_order_without_token_returns_401() {
    let (broadcaster, _) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(OrderManager::with_poll_interval(
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(FakeOrderRepository::new()),
        broadcaster,
        SafetyConfig::default(),
        Duration::from_millis(10),
    ));
    let state = build_state(
        manager,
        Arc::new(FakeSubscriptionRepository::new()),
        admin_repo(),
    );

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/admin/orders/place")
        .send_json(&json!({
            "user_id": 1, "exchange": "tabdeal", "pair": "USDT/IRT",
            "side": "buy", "type": "market", "quantity": "100"
        }))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[actix_web::test]
async fn admin_halt_without_token_returns_401() {
    let (broadcaster, _) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(OrderManager::with_poll_interval(
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(FakeOrderRepository::new()),
        broadcaster,
        SafetyConfig::default(),
        Duration::from_millis(10),
    ));
    let state = build_state(
        manager,
        Arc::new(FakeSubscriptionRepository::new()),
        admin_repo(),
    );

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/admin/strategies/halt")
        .send_json(&json!({"user_id": 5}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ---------------------------------------------------------------------------
// Test: non-admin user cannot access admin endpoints

#[actix_web::test]
async fn trader_cannot_place_admin_order() {
    let user_repo = Arc::new(FakeUserRepository::with_roles(vec![RoleRecord {
        name: "trader".to_string(),
        permissions: vec!["subscriptions.write".to_string()],
    }]));

    let (broadcaster, _) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(OrderManager::with_poll_interval(
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(FakeOrderRepository::new()),
        broadcaster,
        SafetyConfig::default(),
        Duration::from_millis(10),
    ));
    let state = build_state(
        manager,
        Arc::new(FakeSubscriptionRepository::new()),
        Arc::clone(&user_repo),
    );
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    user_repo
        .create_user("trader1", &hash_password("pw"))
        .await
        .unwrap();
    let u = user_repo
        .find_by_username("trader1")
        .await
        .unwrap()
        .unwrap();
    user_repo
        .assign_roles(u.id, &["trader".to_string()])
        .await
        .unwrap();

    let mut resp = srv
        .post("/v1/auth/token")
        .send_json(&json!({"username": "trader1", "password": "pw"}))
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let token = body["data"]["token"].as_str().unwrap();

    let resp = srv
        .post("/v1/admin/orders/place")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({
            "user_id": 1, "exchange": "tabdeal", "pair": "USDT/IRT",
            "side": "buy", "type": "market", "quantity": "100"
        }))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "trader must be forbidden from admin endpoints"
    );
}
