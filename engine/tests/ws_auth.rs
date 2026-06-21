use std::collections::HashMap;
use std::sync::Arc;

use actix_web::App;
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::presentation::middlewares::jwt::mint_token;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

const SECRET: &str = "test-secret";

fn build_state() -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: Some(Arc::new(SECRET.to_string())),
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
        exchange_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    })
}

// Browsers' native WebSocket API cannot set an Authorization header on the
// upgrade request, so /v1/ws must accept the JWT as a `?token=` query
// param instead — this is the one exemption from header-only auth.

#[actix_web::test]
async fn ws_connection_with_valid_token_in_query_succeeds() {
    let state = build_state();
    let mut app =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let token = mint_token("1", SECRET, 3600);
    let result = app.ws_at(&format!("/v1/ws?token={token}")).await;

    assert!(
        result.is_ok(),
        "WS upgrade with a valid query token must succeed, got: {:?}",
        result.err()
    );
}

#[actix_web::test]
async fn ws_connection_without_token_is_rejected() {
    let state = build_state();
    let mut app =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let result = app.ws_at("/v1/ws").await;

    assert!(
        result.is_err(),
        "WS upgrade with no token at all must be rejected when auth is enabled"
    );
}

#[actix_web::test]
async fn ws_connection_with_invalid_token_in_query_is_rejected() {
    let state = build_state();
    let mut app =
        actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let result = app.ws_at("/v1/ws?token=not-a-real-jwt").await;

    assert!(
        result.is_err(),
        "WS upgrade with an invalid token must be rejected"
    );
}
