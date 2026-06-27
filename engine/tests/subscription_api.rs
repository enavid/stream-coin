use std::collections::HashMap;
use std::sync::Arc;

use actix_web::App;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::infrastructure::crypto::password::hash_password;
use stream_coin::infrastructure::db::subscription_repository::FakeSubscriptionRepository;
use stream_coin::infrastructure::db::user_repository::{
    FakeUserRepository, RoleRecord, UserRepository,
};
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

fn build_state(
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
        order_manager: None,
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

fn trader_repo() -> Arc<FakeUserRepository> {
    Arc::new(FakeUserRepository::with_roles(vec![RoleRecord {
        name: "trader".to_string(),
        permissions: vec![
            "subscriptions.write".to_string(),
            "subscriptions.read".to_string(),
        ],
    }]))
}

async fn seed_user_and_login(
    user_repo: &FakeUserRepository,
    username: &str,
    password: &str,
    role: &str,
    srv: &actix_test::TestServer,
) -> String {
    user_repo
        .create_user(username, &hash_password(password))
        .await
        .unwrap();
    let user = user_repo.find_by_username(username).await.unwrap().unwrap();
    user_repo
        .assign_roles(user.id, &[role.to_string()])
        .await
        .unwrap();

    let mut resp = srv
        .post("/v1/auth/token")
        .send_json(&json!({"username": username, "password": password}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "login for {username} must succeed");
    let body: Value = resp.json().await.unwrap();
    body["data"]["token"].as_str().unwrap().to_string()
}

// ── Full CRUD lifecycle ──────────────────────────────────────────────────────

#[actix_web::test]
async fn subscription_full_crud_lifecycle_through_real_router() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = seed_user_and_login(&user_repo, "alice", "pw", "trader", &srv).await;

    // Subscribe
    let mut resp = srv
        .post("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"strategy_id": "spread-1"}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "create subscription must succeed");
    let body: Value = resp.json().await.unwrap();
    let sub_id = body["data"]["id"].as_i64().unwrap();
    assert_eq!(body["data"]["strategy_id"], "spread-1");
    assert!(body["data"]["active"].as_bool().unwrap());

    // List — should contain exactly the new subscription
    let mut resp = srv
        .get("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["data"]["subscriptions"].as_array().unwrap().len(), 1);

    // Update — deactivate and set overrides
    let mut resp = srv
        .patch(format!("/v1/subscriptions/{sub_id}"))
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"active": false, "confidence_threshold": 0.8}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(!body["data"]["active"].as_bool().unwrap());
    assert_eq!(body["data"]["confidence_threshold"], 0.8);

    // Delete
    let resp = srv
        .delete(format!("/v1/subscriptions/{sub_id}"))
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "delete must succeed");

    // List — must be empty again
    let mut resp = srv
        .get("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["data"]["subscriptions"].as_array().unwrap().len(),
        0,
        "subscription list must be empty after delete"
    );
}

// ── Duplicate subscription returns 409 ──────────────────────────────────────

#[actix_web::test]
async fn subscribe_twice_to_same_strategy_returns_409() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = seed_user_and_login(&user_repo, "bob", "pw", "trader", &srv).await;

    let payload = json!({"strategy_id": "spread-1"});
    srv.post("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&payload)
        .await
        .unwrap();

    let resp = srv
        .post("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&payload)
        .await
        .unwrap();
    assert_eq!(resp.status(), 409, "duplicate subscription must return 409");
}

// ── Unauthenticated requests are rejected ───────────────────────────────────

#[actix_web::test]
async fn unauthenticated_subscription_request_returns_401() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/subscriptions")
        .send_json(&json!({"strategy_id": "spread-1"}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── Ownership isolation: user cannot touch another user's subscription ───────

#[actix_web::test]
async fn user_cannot_update_subscription_belonging_to_another_user() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token_alice = seed_user_and_login(&user_repo, "alice2", "pw", "trader", &srv).await;
    let token_bob = seed_user_and_login(&user_repo, "bob2", "pw", "trader", &srv).await;

    // Alice creates a subscription
    let mut resp = srv
        .post("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token_alice}")))
        .send_json(&json!({"strategy_id": "spread-x"}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let alice_sub_id = body["data"]["id"].as_i64().unwrap();

    // Bob tries to update Alice's subscription
    let resp = srv
        .patch(format!("/v1/subscriptions/{alice_sub_id}"))
        .insert_header(("Authorization", format!("Bearer {token_bob}")))
        .send_json(&json!({"active": false}))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "updating another user's subscription must return 403"
    );
}

#[actix_web::test]
async fn user_cannot_delete_subscription_belonging_to_another_user() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token_alice = seed_user_and_login(&user_repo, "alice3", "pw", "trader", &srv).await;
    let token_bob = seed_user_and_login(&user_repo, "bob3", "pw", "trader", &srv).await;

    // Alice creates a subscription
    let mut resp = srv
        .post("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token_alice}")))
        .send_json(&json!({"strategy_id": "spread-y"}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let alice_sub_id = body["data"]["id"].as_i64().unwrap();

    // Bob tries to delete Alice's subscription
    let resp = srv
        .delete(format!("/v1/subscriptions/{alice_sub_id}"))
        .insert_header(("Authorization", format!("Bearer {token_bob}")))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "deleting another user's subscription must return 403"
    );
}

// ── Cross-user isolation: list only returns own subscriptions ────────────────

#[actix_web::test]
async fn list_subscriptions_never_leaks_other_users_data() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token_alice = seed_user_and_login(&user_repo, "alice4", "pw", "trader", &srv).await;
    let token_bob = seed_user_and_login(&user_repo, "bob4", "pw", "trader", &srv).await;

    // Alice subscribes to two strategies
    for strat in ["spread-1", "rsi-2"] {
        srv.post("/v1/subscriptions")
            .insert_header(("Authorization", format!("Bearer {token_alice}")))
            .send_json(&json!({"strategy_id": strat}))
            .await
            .unwrap();
    }
    // Bob subscribes to one strategy
    srv.post("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token_bob}")))
        .send_json(&json!({"strategy_id": "spread-1"}))
        .await
        .unwrap();

    // Alice's list must contain exactly 2 (her own)
    let mut resp = srv
        .get("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token_alice}")))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["data"]["subscriptions"].as_array().unwrap().len(),
        2,
        "alice must only see her own subscriptions"
    );

    // Bob's list must contain exactly 1 (his own)
    let mut resp = srv
        .get("/v1/subscriptions")
        .insert_header(("Authorization", format!("Bearer {token_bob}")))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["data"]["subscriptions"].as_array().unwrap().len(),
        1,
        "bob must only see his own subscriptions"
    );
}

// ── 400 on missing subscription ──────────────────────────────────────────────

#[actix_web::test]
async fn update_nonexistent_subscription_returns_400() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = seed_user_and_login(&user_repo, "carol", "pw", "trader", &srv).await;

    let resp = srv
        .patch("/v1/subscriptions/9999")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"active": true}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn delete_nonexistent_subscription_returns_400() {
    let sub_repo = Arc::new(FakeSubscriptionRepository::new());
    let user_repo = trader_repo();
    let state = build_state(Arc::clone(&sub_repo), Arc::clone(&user_repo));

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = seed_user_and_login(&user_repo, "dave", "pw", "trader", &srv).await;

    let resp = srv
        .delete("/v1/subscriptions/9999")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}
