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
        exchange_adapters: Arc::new(HashMap::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
    })
}

fn build_state_with_ticker(key: &str) -> actix_web::web::Data<AppState> {
    let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
    let mut map = HashMap::new();
    map.insert(key.to_string(), handle);
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(HashMap::new()),
        clients: Arc::new(Mutex::new(map)),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
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
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
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
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
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

// stop

#[actix_web::test]
async fn stop_ticker_returns_400_when_ticker_not_running() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/stop_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn stop_ticker_returns_ticker_not_found_message() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/stop_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["message"], "Ticker not found");
}

#[actix_web::test]
async fn stop_ticker_returns_200_when_ticker_is_running() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state_with_ticker("tabdeal:USDT/IRT"))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/stop_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn stop_ticker_response_contains_exchange_and_pair() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state_with_ticker("tabdeal:USDT/IRT"))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/stop_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["exchange"], "tabdeal");
    assert_eq!(body["data"]["pair"], "USDT/IRT");
}

// --- symbol format enforcement tests ---

#[actix_web::test]
async fn start_ticker_rejects_exchange_specific_format() {
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
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDTIRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        400,
        "exchange-specific format must be rejected"
    );
}

#[actix_web::test]
async fn start_ticker_rejects_exchange_specific_format_with_message() {
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
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDTIRT"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let errors_raw = body["errors"].to_string();
    assert!(
        errors_raw.contains("BASE/QUOTE"),
        "error must mention BASE/QUOTE format, got: {errors_raw}"
    );
}

#[actix_web::test]
async fn start_ticker_rejects_empty_base() {
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
        .set_payload(r#"{"exchange":"tabdeal","symbol":"/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_ticker_rejects_empty_quote() {
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
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

// --- malformed / boundary request body tests ---

#[actix_web::test]
async fn start_ticker_invalid_json_body_returns_400() {
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
        .set_payload("{not valid json}")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_ticker_wrong_field_types_returns_400() {
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
        .set_payload(r#"{"exchange": 42, "symbol": true}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_ticker_missing_exchange_field_returns_400() {
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
        .set_payload(r#"{"symbol": "USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_ticker_missing_symbol_field_returns_400() {
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
        .set_payload(r#"{"exchange": "tabdeal"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_ticker_empty_exchange_returns_400_as_unsupported() {
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
        .set_payload(r#"{"exchange": "", "symbol": "USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn start_ticker_oversized_payload_returns_400() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    // actix-web JsonConfig default limit is 256 KB; this exceeds it
    let large_symbol = "A".repeat(300_000);
    let body = format!(r#"{{"exchange":"tabdeal","symbol":"{large_symbol}"}}"#);

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(body)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

// list

#[actix_web::test]
async fn list_tickers_returns_200() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v1/exchanges/futures/tickers")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn list_tickers_returns_empty_array_when_no_tickers() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v1/exchanges/futures/tickers")
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["tickers"], serde_json::json!([]));
}

#[actix_web::test]
async fn list_tickers_returns_canonical_pair_not_exchange_format() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state_with_ticker("tabdeal:USDT/IRT"))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v1/exchanges/futures/tickers")
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let tickers = body["data"]["tickers"].as_array().unwrap();
    assert_eq!(tickers.len(), 1);
    assert_eq!(tickers[0]["exchange"], "tabdeal");
    assert_eq!(
        tickers[0]["pair"], "USDT/IRT",
        "list must return canonical pair, not exchange-specific symbol"
    );
}
