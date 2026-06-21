use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test;
use actix_web::App;
use tokio::sync::{Mutex, RwLock};

use stream_coin::exchange::hitobit::HitobitWsAdapter;
use stream_coin::exchange::port::ExchangeAdapter;
use stream_coin::exchange::registry::{ExchangeRecord, ExchangeRegistry, TradingPairRecord};
use stream_coin::exchange::tabdeal::TabdealWsAdapter;
use stream_coin::infrastructure::db::ticker_repository::{FakeTickerRepository, TickerRepository};
use stream_coin::presentation::handlers::exchange_handler::restore_tickers;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
use stream_coin::presentation::middlewares::jwt::mint_token;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};
use stream_coin::price::entity::MarketType;

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
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
    })
}

fn build_state_with_hitobit() -> actix_web::web::Data<AppState> {
    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    adapters.insert("hitobit".to_string(), Arc::new(HitobitWsAdapter::default()));
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(adapters)),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
    })
}

fn build_state_with_both_adapters() -> actix_web::web::Data<AppState> {
    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    adapters.insert("hitobit".to_string(), Arc::new(HitobitWsAdapter::default()));
    adapters.insert("tabdeal".to_string(), Arc::new(TabdealWsAdapter::default()));
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(adapters)),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
    })
}

fn build_state_with_ticker(key: &str) -> actix_web::web::Data<AppState> {
    let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
    let mut map = HashMap::new();
    map.insert(key.to_string(), handle);
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(map)),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
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

// --- hitobit adapter tests (Loop 1a) ---

#[actix_web::test]
async fn start_ticker_hitobit_returns_200() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state_with_hitobit())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"hitobit","symbol":"USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn hitobit_and_tabdeal_same_pair_both_active() {
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(build_state_with_both_adapters())
            .app_data(json_error_handler_config()),
    )
    .await;

    let req1 = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"hitobit","symbol":"USDT/IRT"}"#)
        .to_request();
    let resp1 = test::call_service(&app, req1).await;
    assert_eq!(
        resp1.status(),
        200,
        "hitobit ticker must start successfully"
    );

    let req2 = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(
        resp2.status(),
        200,
        "tabdeal ticker must start alongside hitobit for the same pair"
    );
}

// --- field-level error name tests (1c) ---

#[actix_web::test]
async fn start_ticker_wrong_symbol_format_errors_names_symbol_field() {
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
    assert_eq!(body["errors"][0]["field"], "symbol");
}

#[actix_web::test]
async fn start_ticker_unsupported_exchange_errors_names_exchange_field() {
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
        .set_payload(r#"{"exchange":"unknown_exchange","symbol":"USDT/IRT"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(
        body["errors"][0]["field"], "exchange",
        "unsupported exchange error must name the exchange field"
    );
}

#[actix_web::test]
async fn start_ticker_wrong_type_for_symbol_errors_names_symbol_field() {
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
        .set_payload(r#"{"symbol":42,"exchange":"tabdeal"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["errors"][0]["field"], "symbol");
}

#[actix_web::test]
async fn start_ticker_missing_symbol_field_errors_names_symbol_field() {
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
        .set_payload(r#"{"exchange":"tabdeal"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["errors"][0]["field"], "symbol");
}

#[actix_web::test]
async fn start_ticker_missing_exchange_field_errors_names_exchange_field() {
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
        .set_payload(r#"{"symbol":"USDT/IRT"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["errors"][0]["field"], "exchange");
}

#[actix_web::test]
async fn start_ticker_empty_exchange_errors_names_exchange_field() {
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
        .set_payload(r#"{"exchange":"","symbol":"USDT/IRT"}"#)
        .to_request();

    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(
        body["errors"][0]["field"], "exchange",
        "empty exchange must produce a field error naming 'exchange'"
    );
}

// --- exchange registry tests (ROADMAP 1b) ---

#[actix_web::test]
async fn start_ticker_for_disabled_exchange_returns_400() {
    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "tabdeal".to_string(),
        display_name: "Tabdeal".to_string(),
        ws_url: "wss://tabdeal.example.com".to_string(),
        enabled: false,
    });

    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
                exchange_registry: Arc::new(Mutex::new(registry)),
                adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
                clients: Arc::new(Mutex::new(HashMap::new())),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: None,
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        400,
        "disabled exchange must return 400 (no adapter in map)"
    );
}

#[actix_web::test]
async fn disable_exchange_aborts_running_tickers() {
    let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
    let clients = Arc::new(Mutex::new({
        let mut m = HashMap::new();
        m.insert("tabdeal:USDT/IRT".to_string(), handle);
        m
    }));

    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "tabdeal".to_string(),
        display_name: "Tabdeal".to_string(),
        ws_url: "wss://tabdeal.example.com".to_string(),
        enabled: true,
    });

    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    adapters.insert("tabdeal".to_string(), Arc::new(TabdealWsAdapter::default()));

    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(adapters)),
                exchange_registry: Arc::new(Mutex::new(registry)),
                adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
                clients: Arc::clone(&clients),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: None,
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/admin/exchanges/disable")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "disable must return 200");
    assert!(
        clients.lock().await.is_empty(),
        "all tickers for disabled exchange must be aborted"
    );
}

#[actix_web::test]
async fn enable_exchange_then_start_ticker_returns_200() {
    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "hitobit".to_string(),
        display_name: "Hitobit".to_string(),
        ws_url: "wss://stream.hitobit.com:443".to_string(),
        enabled: false,
    });

    let factories: HashMap<String, AdapterFactory> = HashMap::from([(
        "hitobit".to_string(),
        Arc::new(|_ws_url: &str| Arc::new(HitobitWsAdapter::default()) as Arc<dyn ExchangeAdapter>)
            as AdapterFactory,
    )]);

    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
                exchange_registry: Arc::new(Mutex::new(registry)),
                adapter_factories: Arc::new(factories),
                clients: Arc::new(Mutex::new(HashMap::new())),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: None,
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let enable_req = test::TestRequest::post()
        .uri("/v1/admin/exchanges/enable")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"hitobit"}"#)
        .to_request();
    let enable_resp = test::call_service(&app, enable_req).await;
    assert_eq!(enable_resp.status(), 200, "enable must succeed");

    let start_req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"hitobit","symbol":"USDT/IRT"}"#)
        .to_request();
    let start_resp = test::call_service(&app, start_req).await;
    assert_eq!(
        start_resp.status(),
        200,
        "start ticker after enable must succeed"
    );
}

#[actix_web::test]
async fn list_pairs_returns_only_active_pairs() {
    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "tabdeal".to_string(),
        display_name: "Tabdeal".to_string(),
        ws_url: "wss://tabdeal.example.com".to_string(),
        enabled: true,
    });
    registry.add_pair(TradingPairRecord {
        exchange_name: "tabdeal".to_string(),
        base: "USDT".to_string(),
        quote: "IRT".to_string(),
        market_type: MarketType::Spot,
        active: true,
    });
    registry.add_pair(TradingPairRecord {
        exchange_name: "tabdeal".to_string(),
        base: "BTC".to_string(),
        quote: "IRT".to_string(),
        market_type: MarketType::Spot,
        active: false,
    });

    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
                exchange_registry: Arc::new(Mutex::new(registry)),
                adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
                clients: Arc::new(Mutex::new(HashMap::new())),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: None,
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v1/exchanges/tabdeal/pairs")
        .to_request();
    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(body["success"], true);
    let pairs = body["data"]["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "only active pairs must be returned");
    assert_eq!(pairs[0]["base"], "USDT");
    assert_eq!(pairs[0]["quote"], "IRT");
}

#[actix_web::test]
async fn list_pairs_filters_by_market_type() {
    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "tabdeal".to_string(),
        display_name: "Tabdeal".to_string(),
        ws_url: "wss://tabdeal.example.com".to_string(),
        enabled: true,
    });
    registry.add_pair(TradingPairRecord {
        exchange_name: "tabdeal".to_string(),
        base: "USDT".to_string(),
        quote: "IRT".to_string(),
        market_type: MarketType::Spot,
        active: true,
    });
    registry.add_pair(TradingPairRecord {
        exchange_name: "tabdeal".to_string(),
        base: "BTC".to_string(),
        quote: "IRT".to_string(),
        market_type: MarketType::Futures,
        active: true,
    });

    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
                exchange_registry: Arc::new(Mutex::new(registry)),
                adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
                clients: Arc::new(Mutex::new(HashMap::new())),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: None,
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v1/exchanges/tabdeal/pairs?market_type=spot")
        .to_request();
    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let pairs = body["data"]["pairs"].as_array().unwrap();
    assert_eq!(
        pairs.len(),
        1,
        "spot filter must return only spot pairs, got {pairs:?}"
    );
    assert_eq!(pairs[0]["market_type"], "spot");
}

#[actix_web::test]
async fn start_ticker_without_token_returns_401() {
    let secret = "test_jwt_secret";
    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
                exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
                adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
                clients: Arc::new(Mutex::new(HashMap::new())),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: Some(Arc::new(secret.to_string())),
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401, "missing token must return 401");
}

#[actix_web::test]
async fn start_ticker_with_valid_token_returns_200() {
    let secret = "test_jwt_secret";
    let token = mint_token("test_user", secret, 3600);

    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    adapters.insert("tabdeal".to_string(), Arc::new(TabdealWsAdapter::default()));

    let app = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(actix_web::web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(adapters)),
                exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
                adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
                clients: Arc::new(Mutex::new(HashMap::new())),
                publisher: None,
                broadcaster: AppState::new_broadcaster(),
                jwt_secret: Some(Arc::new(secret.to_string())),
                ticker_repository: None,
                running_strategies: Arc::new(Mutex::new(HashMap::new())),
                strategy_repository: None,
                signal_repository: None,
                order_adapters: Arc::new(HashMap::new()),
                order_manager: None,
                python_strategy_repository: None,
                candle_repository: None,
            }))
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .insert_header(("Authorization", format!("Bearer {token}")))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "valid token must allow the request");
}

// --- ticker persistence tests (ROADMAP 1d) ---

#[actix_web::test]
async fn two_engine_instances_share_ticker_state() {
    let shared_repo: Arc<dyn TickerRepository> = Arc::new(FakeTickerRepository::new());

    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    adapters.insert("tabdeal".to_string(), Arc::new(TabdealWsAdapter::default()));

    let state1 = actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(adapters.clone())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: Some(Arc::clone(&shared_repo)),
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
    });

    let app1 = test::init_service(
        App::new()
            .configure(init_routes)
            .app_data(state1)
            .app_data(json_error_handler_config()),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/exchanges/futures/start_kline_symbol_ticker")
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"exchange":"tabdeal","symbol":"USDT/IRT"}"#)
        .to_request();
    let resp = test::call_service(&app1, req).await;
    assert_eq!(resp.status(), 200, "instance 1 must start the ticker");

    let state2 = actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(adapters)),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: Some(Arc::clone(&shared_repo)),
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
    });

    restore_tickers(&state2).await;

    assert!(
        state2.clients.lock().await.contains_key("tabdeal:USDT/IRT"),
        "instance 2 must restore the ticker from shared state"
    );
}
