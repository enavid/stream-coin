use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test;
use actix_web::App;
use tokio::sync::Mutex;

use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::AppState;

fn build_state() -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
        redis: None,
        ticker_repository: None,
        exchange_adapters: Arc::new(HashMap::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
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
async fn health_endpoint_returns_status_up() {
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

    assert_eq!(body["data"]["status"], "up");
}

#[actix_web::test]
async fn health_endpoint_returns_redis_disconnected() {
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

    assert_eq!(body["data"]["dependencies"]["redis"], "disconnected");
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
