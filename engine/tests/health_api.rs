use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test;
use actix_web::App;
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::registry::ExchangeRegistry;
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
        jwt_secret: None,
        ticker_repository: None,
    })
}

#[actix_web::test]
async fn health_endpoint_returns_200() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn health_endpoint_returns_project_name() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let body: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .to_request(),
    )
    .await;

    assert_eq!(body["data"]["name"], "stream-coin");
}

#[actix_web::test]
async fn health_endpoint_returns_status_down_when_redis_disconnected() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let body: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .to_request(),
    )
    .await;

    assert_eq!(body["data"]["status"], "down");
}

#[actix_web::test]
async fn health_endpoint_checks_redis_is_down_when_disconnected() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let body: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .to_request(),
    )
    .await;

    assert_eq!(body["data"]["checks"]["redis"], "down");
}

#[actix_web::test]
async fn health_endpoint_response_has_success_true() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let body: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/v1/check/health")
            .to_request(),
    )
    .await;

    assert_eq!(body["success"], true);
}
