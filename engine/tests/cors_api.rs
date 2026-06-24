use std::collections::HashMap;
use std::sync::Arc;

use actix_web::http::header;
use actix_web::test;
use actix_web::App;
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::presentation::middlewares::cors::cors_middleware;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

fn build_state() -> actix_web::web::Data<AppState> {
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
async fn cors_preflight_request_receives_allow_origin_header_when_no_allowlist_configured() {
    let app = test::init_service(
        App::new()
            .wrap(cors_middleware(None))
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::with_uri("/v1/auth/token")
            .method(actix_web::http::Method::OPTIONS)
            .insert_header(("Origin", "http://localhost:38391"))
            .insert_header(("Access-Control-Request-Method", "POST"))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "preflight response must carry Access-Control-Allow-Origin"
    );
}

#[actix_web::test]
async fn cors_preflight_bypasses_jwt_auth() {
    // A preflight has no Authorization header — if CORS didn't short-circuit
    // it before reaching the JWT middleware, this would 401, not 200.
    let app = test::init_service(
        App::new()
            .wrap(cors_middleware(None))
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::with_uri("/v1/orders")
            .method(actix_web::http::Method::OPTIONS)
            .insert_header(("Origin", "http://localhost:38391"))
            .insert_header(("Access-Control-Request-Method", "GET"))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn cors_actual_response_carries_allow_origin_header() {
    let app = test::init_service(
        App::new()
            .wrap(cors_middleware(None))
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .insert_header(("Origin", "http://localhost:38391"))
            .to_request(),
    )
    .await;

    assert!(resp
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[actix_web::test]
async fn cors_with_explicit_allowlist_allows_listed_origin() {
    let app = test::init_service(
        App::new()
            .wrap(cors_middleware(Some(
                "http://localhost:38391,http://localhost:5173",
            )))
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .insert_header(("Origin", "http://localhost:38391"))
            .to_request(),
    )
    .await;

    assert!(resp
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[actix_web::test]
async fn cors_rejects_arbitrary_internet_origin_when_no_allowlist_configured() {
    let app = test::init_service(
        App::new()
            .wrap(cors_middleware(None))
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .insert_header(("Origin", "https://evil.com"))
            .to_request(),
    )
    .await;

    assert!(
        !resp
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "an arbitrary internet origin must not be granted CORS access just because no allowlist is configured"
    );
}
