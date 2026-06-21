use std::collections::HashMap;
use std::sync::Arc;

use actix_web::App;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::{ExchangeRecord, ExchangeRegistry};
use stream_coin::infrastructure::crypto::credential_cipher::CredentialCipher;
use stream_coin::infrastructure::crypto::password::hash_password;
use stream_coin::infrastructure::db::credential_repository::FakeCredentialRepository;
use stream_coin::infrastructure::db::user_repository::{FakeUserRepository, RoleRecord};
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

/// Builds full app state with every repo wired, routed through the real `init_routes`
/// composition — this is what `set_exchange_credentials`/`/admin` duplicate-scope bug
/// (two `web::scope("/admin")`, two `web::scope("/exchanges")`) slipped past: handler unit
/// tests bypass the router tree entirely. These tests go through the real router.
fn build_state() -> actix_web::web::Data<AppState> {
    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "tabdeal".to_string(),
        display_name: "Tabdeal".to_string(),
        ws_url: "wss://tabdeal.example.com".to_string(),
        enabled: true,
    });

    let user_repo = Arc::new(FakeUserRepository::with_roles(vec![
        RoleRecord {
            name: "admin".to_string(),
            permissions: vec![
                "users.manage".to_string(),
                "roles.manage".to_string(),
                "orders.manage".to_string(),
            ],
        },
        RoleRecord {
            name: "trader".to_string(),
            permissions: vec!["exchange_credentials.write".to_string()],
        },
    ]));

    let credential_repo = Arc::new(FakeCredentialRepository::new(vec!["tabdeal".to_string()]));

    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(registry)),
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
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        user_repository: Some(user_repo),
        credential_repository: Some(credential_repo),
        credential_cipher: Some(Arc::new(CredentialCipher::new([5u8; 32]))),
    })
}

async fn login(srv: &actix_test::TestServer, username: &str, password: &str) -> String {
    let mut resp = srv
        .post("/v1/auth/token")
        .send_json(&json!({"username": username, "password": password}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "login must succeed for {username}");
    let body: Value = resp.json().await.unwrap();
    body["data"]["token"].as_str().unwrap().to_string()
}

#[actix_web::test]
async fn admin_creates_user_through_real_router() {
    let state = build_state();
    state
        .user_repository
        .as_ref()
        .unwrap()
        .create_user("admin", &hash_password("adminpw"))
        .await
        .unwrap();
    let admin = state
        .user_repository
        .as_ref()
        .unwrap()
        .find_by_username("admin")
        .await
        .unwrap()
        .unwrap();
    state
        .user_repository
        .as_ref()
        .unwrap()
        .assign_roles(admin.id, &["admin".to_string()])
        .await
        .unwrap();

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = login(&srv, "admin", "adminpw").await;

    let resp = srv
        .post("/v1/admin/users")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"username": "alice", "password": "alicepw", "roles": ["trader"]}))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "creating a user through the real router must succeed (regression guard for duplicate-scope routing bug)"
    );
}

#[actix_web::test]
async fn admin_routes_404_was_a_duplicate_scope_bug_now_fixed() {
    let state = build_state();
    state
        .user_repository
        .as_ref()
        .unwrap()
        .create_user("admin", &hash_password("adminpw"))
        .await
        .unwrap();
    let admin = state
        .user_repository
        .as_ref()
        .unwrap()
        .find_by_username("admin")
        .await
        .unwrap()
        .unwrap();
    state
        .user_repository
        .as_ref()
        .unwrap()
        .assign_roles(admin.id, &["admin".to_string()])
        .await
        .unwrap();

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = login(&srv, "admin", "adminpw").await;

    for (method_post, path) in [
        (true, "/v1/admin/users"),
        (false, "/v1/admin/roles"),
        (false, "/v1/admin/permissions"),
    ] {
        let resp = if method_post {
            srv.post(path)
                .insert_header(("Authorization", format!("Bearer {token}")))
                .send_json(&json!({"username": "x", "password": "y", "roles": []}))
                .await
                .unwrap()
        } else {
            srv.get(path)
                .insert_header(("Authorization", format!("Bearer {token}")))
                .send()
                .await
                .unwrap()
        };
        assert_ne!(
            resp.status(),
            404,
            "{path} must be routable, not shadowed by another scope"
        );
    }

    // The pre-existing /admin/exchanges scope (registry_router) must still work too.
    let resp = srv
        .post("/v1/admin/exchanges/enable")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"exchange": "tabdeal"}))
        .await
        .unwrap();
    assert_ne!(resp.status(), 404);

    // And the circuit-breaker route that used to live in its own /admin scope.
    let resp = srv
        .post("/v1/admin/circuit-breaker/reset")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();
    assert_ne!(resp.status(), 404);
}

#[actix_web::test]
async fn self_service_credential_routes_work_through_real_router() {
    let state = build_state();
    state
        .user_repository
        .as_ref()
        .unwrap()
        .create_user("bob", &hash_password("bobpw"))
        .await
        .unwrap();
    let bob = state
        .user_repository
        .as_ref()
        .unwrap()
        .find_by_username("bob")
        .await
        .unwrap()
        .unwrap();
    state
        .user_repository
        .as_ref()
        .unwrap()
        .assign_roles(bob.id, &["trader".to_string()])
        .await
        .unwrap();

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));
    let token = login(&srv, "bob", "bobpw").await;

    let resp = srv
        .post("/v1/exchanges/tabdeal/credentials")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send_json(&json!({"api_key": "secret-abc"}))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "setting own credentials must succeed");

    let mut resp = srv
        .get("/v1/exchanges/credentials")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let creds = body["data"]["credentials"].as_array().unwrap();
    assert_eq!(creds.len(), 1);
    assert_eq!(creds[0]["exchange"], "tabdeal");
    assert!(!body.to_string().contains("secret-abc"));

    // The pre-existing /exchanges (list) and /exchanges/{name}/pairs routes must still work.
    let resp = srv.get("/v1/exchanges").send().await.unwrap();
    assert_ne!(resp.status(), 404);
}
