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

    // Hold the lock across subscribe() — tokio::sync::Mutex is safe to hold across
    // await points. This prevents two concurrent requests for the same ticker from
    // both passing the duplicate check before either has inserted its handle.
    let mut clients = state.clients.lock().await;

    if clients.contains_key(&key) {
        tracing::warn!(
            exchange = %request.exchange,
            symbol = %request.symbol,
            "ticker already running"
        );
        return ApiError::new("Ticker already running", vec![]).to_response();
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
    let broadcaster = state.broadcaster.clone();
    let topic = std::env::var("KAFKA_TOPIC_PRICES").unwrap_or_else(|_| "prices".to_string());

    tokio::spawn(async move {
        use crate::kafka::producer::KafkaProducer;

        while let Some(price) = rx.recv().await {
            tracing::info!(
                exchange = %price.exchange,
                pair = %price.pair,
                bid = price.bid,
                ask = price.ask,
                "price update"
            );

            let payload = match KafkaProducer::price_to_payload(&price) {
                Ok(payload) => payload,
                Err(e) => {
                    tracing::error!(error = %e, "failed to serialize price");
                    continue;
                }
            };

            let _ = broadcaster.send(payload.clone());

            if let Some(ref pub_) = publisher {
                let key = KafkaProducer::price_to_key(&price);
                if let Err(e) = pub_.publish(&topic, &key, &payload).await {
                    tracing::error!(error = %e, "failed to publish price to kafka");
                }
            }
        }
    });

    clients.insert(key, abort);

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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use actix_web::http::StatusCode;
    use actix_web::Responder;
    use actix_web::{test, web, App};
    use async_trait::async_trait;
    use tokio::sync::mpsc::Sender;
    use tokio::sync::Mutex;
    use tokio::task::AbortHandle;

    use super::*;
    use crate::exchange::entity::ExchangeId;
    use crate::exchange::port::{ExchangeAdapter, ExchangeAdapterError};
    use crate::kafka::port::mock::MockPublisher;
    use crate::presentation::shared::app_state::AppState;
    use crate::price::entity::{Price, TradingPair};

    struct CountingAdapter {
        count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ExchangeAdapter for CountingAdapter {
        fn exchange_id(&self) -> ExchangeId {
            ExchangeId::new("tabdeal")
        }

        async fn subscribe(
            &self,
            _symbol: &str,
            _tx: Sender<Price>,
        ) -> Result<AbortHandle, ExchangeAdapterError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(tokio::spawn(std::future::pending::<()>()).abort_handle())
        }
    }

    struct InstantPriceAdapter;

    #[async_trait]
    impl ExchangeAdapter for InstantPriceAdapter {
        fn exchange_id(&self) -> ExchangeId {
            ExchangeId::new("tabdeal")
        }

        async fn subscribe(
            &self,
            _symbol: &str,
            tx: Sender<Price>,
        ) -> Result<AbortHandle, ExchangeAdapterError> {
            let _ = tx
                .send(Price {
                    exchange: ExchangeId::new("tabdeal"),
                    pair: TradingPair::new("USDT", "IRT"),
                    bid: 92_815,
                    ask: 92_936,
                    timestamp: chrono::Utc::now(),
                })
                .await;
            Ok(tokio::spawn(std::future::pending::<()>()).abort_handle())
        }
    }

    fn empty_state() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
        })
    }

    fn state_with_ticker(key: &str) -> web::Data<AppState> {
        let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let mut map = HashMap::new();
        map.insert(key.to_string(), handle);
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(map)),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
        })
    }

    // start

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

    // stop

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
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
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

    // list

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
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(map)),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
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

    // --- kafka failure tests ---

    #[actix_web::test]
    async fn kafka_publish_failure_does_not_stop_broadcast() {
        let failing_publisher = Arc::new(MockPublisher::failing());
        let broadcaster = AppState::new_broadcaster();
        let mut rx = broadcaster.subscribe();

        let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
        adapters.insert("tabdeal".to_string(), Arc::new(InstantPriceAdapter));

        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(adapters),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: Some(failing_publisher),
            broadcaster,
        });

        let dummy_req = test::TestRequest::default().to_http_request();
        start_kline_symbol_ticker(
            state,
            web::Json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDT/IRT".to_string(),
            }),
        )
        .await
        .respond_to(&dummy_req);

        let received = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(
            received.is_ok(),
            "broadcaster must deliver the price even when Kafka publish fails"
        );
        let payload = received.unwrap().unwrap();
        assert!(
            payload.contains("tabdeal"),
            "broadcast payload must contain the exchange name"
        );
    }

    // --- concurrent-start race tests ---

    #[actix_web::test]
    async fn concurrent_start_same_ticker_calls_subscribe_exactly_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let clients: Arc<Mutex<HashMap<String, AbortHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(CountingAdapter {
                count: Arc::clone(&count),
            }),
        );
        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(adapters),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
        });

        let dummy_req = test::TestRequest::default().to_http_request();

        let (r1, r2) = tokio::join!(
            start_kline_symbol_ticker(
                state.clone(),
                web::Json(SymbolRequest {
                    exchange: "tabdeal".to_string(),
                    symbol: "USDT/IRT".to_string(),
                }),
            ),
            start_kline_symbol_ticker(
                state.clone(),
                web::Json(SymbolRequest {
                    exchange: "tabdeal".to_string(),
                    symbol: "USDT/IRT".to_string(),
                }),
            ),
        );

        let s1 = r1.respond_to(&dummy_req).status();
        let s2 = r2.respond_to(&dummy_req).status();

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "subscribe must be called exactly once even under concurrent starts"
        );
        assert_eq!(
            clients.lock().await.len(),
            1,
            "exactly one entry must exist in the clients map"
        );
        let statuses = [s1, s2];
        assert!(
            statuses.contains(&StatusCode::OK),
            "one response must succeed (200)"
        );
        assert!(
            statuses.contains(&StatusCode::BAD_REQUEST),
            "one response must be rejected as duplicate (400)"
        );
    }

    #[actix_web::test]
    async fn stop_then_start_same_ticker_succeeds() {
        let count = Arc::new(AtomicUsize::new(0));
        let clients: Arc<Mutex<HashMap<String, AbortHandle>>> = Arc::new(Mutex::new({
            let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
            let mut m = HashMap::new();
            m.insert("tabdeal:USDT/IRT".to_string(), handle);
            m
        }));
        let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(CountingAdapter {
                count: Arc::clone(&count),
            }),
        );
        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(adapters),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
        });

        let dummy_req = test::TestRequest::default().to_http_request();

        let stop_status = stop_kline_symbol_ticker(
            state.clone(),
            web::Json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDT/IRT".to_string(),
            }),
        )
        .await
        .respond_to(&dummy_req)
        .status();
        assert_eq!(stop_status, StatusCode::OK, "stop must succeed");

        let start_status = start_kline_symbol_ticker(
            state.clone(),
            web::Json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDT/IRT".to_string(),
            }),
        )
        .await
        .respond_to(&dummy_req)
        .status();
        assert_eq!(
            start_status,
            StatusCode::OK,
            "start after stop must succeed"
        );

        assert_eq!(
            clients.lock().await.len(),
            1,
            "exactly one ticker in clients map after stop-and-restart"
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "subscribe must be called exactly once for the new start"
        );
    }
}
