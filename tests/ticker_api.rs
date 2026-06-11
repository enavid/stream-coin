use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test;
use actix_web::App;
use tokio::sync::Mutex;

use stream_coin::presentation::dto::ticker::SymbolRequest;
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
async fn ticker_endpoint_returns_400_when_exchange_not_supported() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .set_json(SymbolRequest {
            exchange: "tabdeal".to_string(),
            symbol: "USDTIRT".to_string(),
        })
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn ticker_endpoint_returns_exchange_not_supported_message() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .set_json(SymbolRequest {
            exchange: "tabdeal".to_string(),
            symbol: "USDTIRT".to_string(),
        })
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["message"], "Exchange not supported");
}

#[actix_web::test]
async fn ticker_endpoint_returns_400_on_missing_body() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}
