use std::sync::Arc;

use actix_web::{web, Responder};
use tokio::sync::mpsc;

use crate::presentation::dto::ticker::{
    ActiveTicker, SymbolRequest, TickerList, TickerStarted, TickerStopped,
};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

#[utoipa::path(
    post,
    path = "/v1/exchanges/futures/start_kline_symbol_ticker",
    tag = "Exchanges",
    request_body = SymbolRequest,
    responses(
        (status = 200, description = "Ticker started successfully", body = TickerStarted),
        (status = 400, description = "Exchange not supported or ticker already running", body = ApiError)
    )
)]
pub async fn start_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: web::Json<SymbolRequest>,
) -> impl Responder {
    let adapter = match state.exchange_adapters.get(&request.exchange) {
        Some(a) => Arc::clone(a),
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                "ticker request rejected: exchange not supported"
            );
            return ApiError::new("Exchange not supported", vec![]).to_response();
        }
    };

    let key = format!("{}:{}", request.exchange, request.symbol);
    {
        let clients = state.clients.lock().await;
        if clients.contains_key(&key) {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "ticker already running"
            );
            return ApiError::new("Ticker already running", vec![]).to_response();
        }
    }

    let (tx, mut rx) = mpsc::channel(100);

    let abort = match adapter.subscribe(&request.symbol, tx).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                error = %e,
                "failed to start ticker"
            );
            return ApiError::new("Failed to start ticker", vec![]).to_response();
        }
    };

    let publisher = state.publisher.clone();
    let topic = std::env::var("KAFKA_TOPIC_PRICES").unwrap_or_else(|_| "prices".to_string());

    tokio::spawn(async move {
        while let Some(price) = rx.recv().await {
            tracing::info!(
                exchange = %price.exchange,
                pair = %price.pair,
                bid = price.bid,
                ask = price.ask,
                "price update"
            );

            if let Some(ref pub_) = publisher {
                use crate::kafka::producer::KafkaProducer;
                let key = KafkaProducer::price_to_key(&price);
                match KafkaProducer::price_to_payload(&price) {
                    Ok(payload) => {
                        if let Err(e) = pub_.publish(&topic, &key, &payload).await {
                            tracing::error!(error = %e, "failed to publish price to kafka");
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "failed to serialize price"),
                }
            }
        }
    });

    {
        let mut clients = state.clients.lock().await;
        clients.insert(key, abort);
    }

    tracing::info!(
        exchange = %request.exchange,
        symbol = %request.symbol,
        "ticker started"
    );

    success_response(
        "Ticker started",
        TickerStarted {
            exchange: request.exchange.clone(),
            symbol: request.symbol.clone(),
        },
    )
}

#[utoipa::path(
    post,
    path = "/v1/exchanges/futures/stop_kline_symbol_ticker",
    tag = "Exchanges",
    request_body = SymbolRequest,
    responses(
        (status = 200, description = "Ticker stopped successfully", body = TickerStopped),
        (status = 400, description = "Ticker not found", body = ApiError)
    )
)]
pub async fn stop_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: web::Json<SymbolRequest>,
) -> impl Responder {
    let key = format!("{}:{}", request.exchange, request.symbol);
    let mut clients = state.clients.lock().await;

    match clients.remove(&key) {
        Some(handle) => {
            handle.abort();
            tracing::info!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "ticker stopped"
            );
            success_response(
                "Ticker stopped",
                TickerStopped {
                    exchange: request.exchange.clone(),
                    symbol: request.symbol.clone(),
                },
            )
        }
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "stop request rejected: ticker not running"
            );
            ApiError::new("Ticker not found", vec![]).to_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/exchanges/futures/tickers",
    tag = "Exchanges",
    responses(
        (status = 200, description = "List of active tickers", body = TickerList)
    )
)]
pub async fn list_tickers(state: web::Data<AppState>) -> impl Responder {
    let clients = state.clients.lock().await;

    let tickers = clients
        .keys()
        .filter_map(|key| {
            let mut parts = key.splitn(2, ':');
            let exchange = parts.next()?.to_string();
            let symbol = parts.next()?.to_string();
            Some(ActiveTicker { exchange, symbol })
        })
        .collect();

    success_response("Active tickers", TickerList { tickers })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_web::http::StatusCode;
    use actix_web::{test, web, App};
    use tokio::sync::Mutex;
    use tokio::task::AbortHandle;

    use super::*;
    use crate::presentation::shared::app_state::AppState;

    fn empty_state() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            ticker_repository: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
        })
    }

    fn state_with_ticker(key: &str) -> web::Data<AppState> {
        let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let mut map = HashMap::new();
        map.insert(key.to_string(), handle);
        web::Data::new(AppState {
            redis: None,
            ticker_repository: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(map)),
            publisher: None,
        })
    }

    // ── start ────────────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn ticker_returns_400_when_exchange_not_supported() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn ticker_returns_exchange_not_supported_message() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["message"], "Exchange not supported");
    }

    // ── stop ─────────────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn stop_returns_400_when_ticker_not_running() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn stop_returns_ticker_not_found_message() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["message"], "Ticker not found");
    }

    #[actix_web::test]
    async fn stop_returns_200_when_ticker_is_running() {
        let state = state_with_ticker("tabdeal:USDTIRT");
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn stop_removes_ticker_from_client_map() {
        let clients: Arc<Mutex<HashMap<String, AbortHandle>>> = Arc::new(Mutex::new({
            let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
            let mut m = HashMap::new();
            m.insert("tabdeal:USDTIRT".to_string(), handle);
            m
        }));
        let state = web::Data::new(AppState {
            redis: None,
            ticker_repository: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::clone(&clients),
            publisher: None,
        });
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        test::call_service(&app, req).await;

        assert!(clients.lock().await.is_empty());
    }

    #[actix_web::test]
    async fn stop_returns_exchange_and_symbol_in_response() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_ticker("tabdeal:USDTIRT"))
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["exchange"], "tabdeal");
        assert_eq!(body["data"]["symbol"], "USDTIRT");
    }

    // ── list ─────────────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn list_returns_200_with_empty_list_when_no_tickers() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/tickers", web::get().to(list_tickers)),
        )
        .await;
        let req = test::TestRequest::get().uri("/tickers").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn list_returns_empty_array_when_no_tickers() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/tickers", web::get().to(list_tickers)),
        )
        .await;
        let req = test::TestRequest::get().uri("/tickers").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["tickers"], serde_json::json!([]));
    }

    #[actix_web::test]
    async fn list_returns_active_ticker_exchange_and_symbol() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_ticker("tabdeal:USDTIRT"))
                .route("/tickers", web::get().to(list_tickers)),
        )
        .await;
        let req = test::TestRequest::get().uri("/tickers").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let tickers = body["data"]["tickers"].as_array().unwrap();
        assert_eq!(tickers.len(), 1);
        assert_eq!(tickers[0]["exchange"], "tabdeal");
        assert_eq!(tickers[0]["symbol"], "USDTIRT");
    }

    #[actix_web::test]
    async fn list_returns_multiple_tickers() {
        let handle1 = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let handle2 = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let mut map = HashMap::new();
        map.insert("tabdeal:USDTIRT".to_string(), handle1);
        map.insert("tabdeal:BTCIRT".to_string(), handle2);
        let state = web::Data::new(AppState {
            redis: None,
            ticker_repository: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(map)),
            publisher: None,
        });
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/tickers", web::get().to(list_tickers)),
        )
        .await;
        let req = test::TestRequest::get().uri("/tickers").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let tickers = body["data"]["tickers"].as_array().unwrap();
        assert_eq!(tickers.len(), 2);
    }

    #[actix_web::test]
    async fn list_response_success_is_true() {
        let app = test::init_service(
            App::new()
                .app_data(empty_state())
                .route("/tickers", web::get().to(list_tickers)),
        )
        .await;
        let req = test::TestRequest::get().uri("/tickers").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], true);
    }
}
