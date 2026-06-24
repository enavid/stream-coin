use std::sync::Arc;

use actix_web::{web, Responder};
use tokio::sync::mpsc;

use crate::candle::aggregator::CandleAggregator;
use crate::candle::entity::Interval;
use crate::infrastructure::db::candle_repository::CandleRepository;
use crate::kafka::port::MessagePublisher;
use crate::presentation::dto::ticker::{
    ActiveTicker, SymbolRequest, TickerList, TickerStarted, TickerStopped,
};
use crate::presentation::extractors::ValidatedJson;
use crate::presentation::responses::{success_response, ApiError, FieldError};
use crate::presentation::shared::app_state::AppState;
use crate::price::entity::Price;

fn spawn_price_forwarder(
    mut rx: tokio::sync::mpsc::Receiver<Price>,
    broadcaster: tokio::sync::broadcast::Sender<String>,
    publisher: Option<Arc<dyn MessagePublisher>>,
    mut aggregators: Vec<CandleAggregator>,
    candle_history: crate::presentation::shared::app_state::CandleHistory,
    candle_repository: Option<Arc<dyn CandleRepository>>,
) {
    use crate::candle::entity::CandlePayload;
    use crate::kafka::producer::KafkaProducer;
    use crate::presentation::ws_message::{PricePayload, WsMessage};

    let price_topic = std::env::var("KAFKA_TOPIC_PRICES").unwrap_or_else(|_| "prices".to_string());
    let candle_topic =
        std::env::var("KAFKA_TOPIC_CANDLES").unwrap_or_else(|_| "candles".to_string());

    tokio::spawn(async move {
        while let Some(price) = rx.recv().await {
            tracing::trace!(
                exchange = %price.exchange,
                pair = %price.pair,
                bid = price.bid,
                ask = price.ask,
                "price update"
            );
            let payload =
                match serde_json::to_string(&WsMessage::PriceUpdate(PricePayload::from(&price))) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize price");
                        continue;
                    }
                };
            let _ = broadcaster.send(payload.clone());
            if let Some(ref pub_) = publisher {
                let key = KafkaProducer::price_to_key(&price);
                if let Err(e) = pub_.publish(&price_topic, &key, &payload).await {
                    tracing::error!(error = %e, "failed to publish price to kafka");
                }
            }

            for agg in &mut aggregators {
                match agg.push(&price) {
                    Some(candle) => {
                        if let Some(ref repo) = candle_repository {
                            if let Err(e) = repo.upsert_candles(std::slice::from_ref(&candle)).await
                            {
                                tracing::error!(
                                    exchange = %candle.exchange,
                                    pair = %candle.pair,
                                    interval = candle.interval.as_str(),
                                    error = %e,
                                    "failed to persist closed candle to db"
                                );
                            }
                        }
                        let payload = CandlePayload::from(&candle);
                        {
                            use crate::presentation::shared::app_state::CANDLE_HISTORY_CAPACITY;
                            let history_key = format!(
                                "{}:{}:{}",
                                payload.exchange, payload.pair, payload.interval
                            );
                            let mut history = candle_history.lock().await;
                            let bucket = history.entry(history_key).or_default();
                            bucket.push_back(payload.clone());
                            while bucket.len() > CANDLE_HISTORY_CAPACITY {
                                bucket.pop_front();
                            }
                        }
                        let key = format!("{}:{}", candle.exchange, candle.pair);
                        match serde_json::to_string(&WsMessage::Candle(payload)) {
                            Ok(json) => {
                                let _ = broadcaster.send(json.clone());
                                if let Some(ref pub_) = publisher {
                                    if let Err(e) = pub_.publish(&candle_topic, &key, &json).await {
                                        tracing::error!(error = %e, "failed to publish candle to kafka");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "failed to serialize candle");
                            }
                        }
                    }
                    None => {
                        // No interval boundary crossed — still broadcast the
                        // forming bar's live OHLC so the UI's chart updates
                        // tick-by-tick instead of jumping only on candle close.
                        // Not persisted to candle_history/Kafka: those stay
                        // closed-candle-only (history is the durable record;
                        // Kafka consumers expect immutable bars).
                        if let Some(forming) = agg.current_candle() {
                            let payload = CandlePayload::from(&forming);
                            if let Ok(json) = serde_json::to_string(&WsMessage::Candle(payload)) {
                                let _ = broadcaster.send(json);
                            }
                        }
                    }
                }
            }
        }
    });
}

/// Reads active ticker subscriptions from the repository and restarts each one.
/// Called on engine startup to restore state that survived a restart.
pub async fn restore_tickers(state: &web::Data<AppState>) {
    let Some(repo) = &state.ticker_repository else {
        return;
    };
    let subs = match repo.list_active().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to load ticker subscriptions from DB");
            return;
        }
    };

    tracing::info!(count = subs.len(), "restoring ticker subscriptions from DB");

    for sub in subs {
        let pair = match sub.symbol.split_once('/') {
            Some((base, quote)) => crate::price::entity::TradingPair::new(base, quote),
            None => {
                tracing::warn!(symbol = %sub.symbol, "invalid symbol format in DB, skipping");
                continue;
            }
        };

        let adapter = {
            let adapters = state.exchange_adapters.read().await;
            adapters.get(&sub.exchange).map(Arc::clone)
        };
        let adapter = match adapter {
            Some(a) => a,
            None => {
                tracing::warn!(exchange = %sub.exchange, "no adapter found for restored ticker, skipping");
                continue;
            }
        };

        let key = format!("{}:{}", sub.exchange, sub.symbol);
        {
            let clients = state.clients.lock().await;
            if clients.contains_key(&key) {
                continue;
            }
        }

        let (tx, rx) = mpsc::channel(100);
        let abort = match adapter.subscribe(&pair, tx).await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(exchange = %sub.exchange, pair = %sub.symbol, error = %e, "failed to restore ticker");
                continue;
            }
        };

        let aggregators = Interval::all()
            .into_iter()
            .map(|i| CandleAggregator::new(sub.exchange.clone(), sub.symbol.clone(), i))
            .collect::<Vec<_>>();
        spawn_price_forwarder(
            rx,
            state.broadcaster.clone(),
            state.publisher.clone(),
            aggregators,
            state.candle_history.clone(),
            state.candle_repository.clone(),
        );

        state.clients.lock().await.insert(key, abort);
        tracing::info!(exchange = %sub.exchange, pair = %sub.symbol, "ticker restored");
    }
}

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
/// `POST /v1/exchanges/futures/start_kline_symbol_ticker` — starts a ticker
/// subscription for the requested exchange and symbol. Returns 409 if already
/// running, 400 if the exchange is not registered.
pub async fn start_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: ValidatedJson<SymbolRequest>,
) -> impl Responder {
    let adapter = {
        let adapters = state.exchange_adapters.read().await;
        adapters.get(request.exchange.as_str()).map(Arc::clone)
    };
    let adapter = match adapter {
        Some(a) => a,
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                "ticker request rejected: exchange not supported"
            );
            return ApiError::new(
                "Exchange not supported",
                vec![FieldError::new("exchange", "unsupported exchange")],
            )
            .to_response();
        }
    };

    let key = format!("{}:{}", request.exchange, request.symbol);

    // Hold the lock across subscribe() — tokio::sync::Mutex is safe to hold across
    // await points. This prevents two concurrent requests for the same ticker from
    // both passing the duplicate check before either has inserted its handle.
    let mut clients = state.clients.lock().await;

    // `contains_key` alone isn't enough: if the underlying task already
    // finished (panicked, or the exchange adapter gave up after losing its
    // upstream connection — there is no supervisor that restarts it or
    // cleans up its entry, see ROADMAP's error-handling section) its
    // `AbortHandle` lingers in `clients` forever. Without this check every
    // future start attempt was permanently rejected as "already running"
    // even though nothing was actually streaming any more.
    if let Some(existing) = clients.get(&key) {
        if existing.is_finished() {
            tracing::warn!(
                exchange = %request.exchange,
                pair = %request.symbol,
                "replacing dead ticker handle"
            );
            clients.remove(&key);
        } else {
            tracing::warn!(
                exchange = %request.exchange,
                pair = %request.symbol,
                "ticker already running"
            );
            return ApiError::new("Ticker already running", vec![]).to_response();
        }
    }

    let (tx, rx) = mpsc::channel(100);

    let abort = match adapter.subscribe(&request.symbol, tx).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(
                exchange = %request.exchange,
                pair = %request.symbol,
                error = %e,
                "failed to start ticker"
            );
            return ApiError::new("Failed to start ticker", vec![]).to_response();
        }
    };

    let aggregators = Interval::all()
        .into_iter()
        .map(|i| CandleAggregator::new(request.exchange.to_string(), request.symbol.to_string(), i))
        .collect::<Vec<_>>();
    spawn_price_forwarder(
        rx,
        state.broadcaster.clone(),
        state.publisher.clone(),
        aggregators,
        state.candle_history.clone(),
        state.candle_repository.clone(),
    );

    clients.insert(key.clone(), abort);

    if let Some(repo) = &state.ticker_repository {
        if let Err(e) = repo
            .insert(&request.exchange.to_string(), &request.symbol.to_string())
            .await
        {
            tracing::error!(error = %e, "failed to persist ticker to DB; ticker runs but will not survive restart");
        }
    }

    tracing::info!(
        exchange = %request.exchange,
        pair = %request.symbol,
        "ticker started"
    );

    success_response(
        "Ticker started",
        TickerStarted {
            exchange: request.exchange.to_string(),
            pair: request.symbol.to_string(),
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
/// `POST /v1/exchanges/futures/stop_kline_symbol_ticker` — aborts a running
/// ticker and removes it from the active client map. Returns 400 if no ticker
/// with the given exchange and symbol is currently running.
pub async fn stop_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: ValidatedJson<SymbolRequest>,
) -> impl Responder {
    let key = format!("{}:{}", request.exchange, request.symbol);
    let mut clients = state.clients.lock().await;

    match clients.remove(&key) {
        Some(handle) => {
            handle.abort();

            if let Some(repo) = &state.ticker_repository {
                if let Err(e) = repo
                    .remove(&request.exchange.to_string(), &request.symbol.to_string())
                    .await
                {
                    tracing::error!(error = %e, "failed to remove ticker from DB");
                }
            }

            tracing::info!(
                exchange = %request.exchange,
                pair = %request.symbol,
                "ticker stopped"
            );
            success_response(
                "Ticker stopped",
                TickerStopped {
                    exchange: request.exchange.to_string(),
                    pair: request.symbol.to_string(),
                },
            )
        }
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                pair = %request.symbol,
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
/// `GET /v1/exchanges/futures/tickers` — returns the list of all currently
/// active ticker subscriptions as `[{exchange, symbol}]` pairs.
pub async fn list_tickers(state: web::Data<AppState>) -> impl Responder {
    let clients = state.clients.lock().await;

    let tickers = clients
        .keys()
        .filter_map(|key| {
            let mut parts = key.splitn(2, ':');
            let exchange = parts.next()?.to_string();
            let pair = parts.next()?.to_string();
            Some(ActiveTicker { exchange, pair })
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
    use tokio::sync::{Mutex, RwLock};
    use tokio::task::AbortHandle;

    use super::*;
    use crate::exchange::entity::ExchangeId;
    use chrono::{TimeZone, Utc};

    use crate::exchange::port::{ExchangeAdapter, ExchangeAdapterError};
    use crate::exchange::registry::ExchangeRegistry;
    use crate::infrastructure::db::ticker_repository::{
        FakeTickerRepository, TickerRepository, TickerSubscription,
    };
    use crate::kafka::port::mock::MockPublisher;
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};
    use crate::price::entity::{Price, TradingPair};

    struct TwoPriceAdapter {
        price1: Price,
        price2: Price,
    }

    #[async_trait]
    impl ExchangeAdapter for TwoPriceAdapter {
        fn exchange_id(&self) -> ExchangeId {
            ExchangeId::new("tabdeal")
        }

        fn symbol_for_pair(&self, pair: &TradingPair) -> String {
            format!("{}{}", pair.base, pair.quote).to_lowercase()
        }

        async fn subscribe(
            &self,
            _pair: &TradingPair,
            tx: Sender<Price>,
        ) -> Result<AbortHandle, ExchangeAdapterError> {
            let p1 = self.price1.clone();
            let p2 = self.price2.clone();
            let handle = tokio::spawn(async move {
                let _ = tx.send(p1).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
                let _ = tx.send(p2).await;
            });
            Ok(handle.abort_handle())
        }
    }

    struct CountingAdapter {
        count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ExchangeAdapter for CountingAdapter {
        fn exchange_id(&self) -> ExchangeId {
            ExchangeId::new("tabdeal")
        }

        fn symbol_for_pair(&self, pair: &TradingPair) -> String {
            format!("{}{}", pair.base, pair.quote).to_lowercase()
        }

        async fn subscribe(
            &self,
            _pair: &TradingPair,
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

        fn symbol_for_pair(&self, pair: &TradingPair) -> String {
            format!("{}{}", pair.base, pair.quote).to_lowercase()
        }

        async fn subscribe(
            &self,
            _pair: &TradingPair,
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
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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

    fn state_with_ticker(key: &str) -> web::Data<AppState> {
        let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let mut map = HashMap::new();
        map.insert(key.to_string(), handle);
        web::Data::new(AppState {
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
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
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
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
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
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
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
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["message"], "Ticker not found");
    }

    #[actix_web::test]
    async fn stop_returns_200_when_ticker_is_running() {
        let state = state_with_ticker("tabdeal:USDT/IRT");
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
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
            m.insert("tabdeal:USDT/IRT".to_string(), handle);
            m
        }));
        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            })
            .to_request();
        test::call_service(&app, req).await;

        assert!(clients.lock().await.is_empty());
    }

    #[actix_web::test]
    async fn stop_returns_exchange_and_symbol_in_response() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_ticker("tabdeal:USDT/IRT"))
                .route("/stop", web::post().to(stop_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/stop")
            .set_json(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["exchange"], "tabdeal");
        assert_eq!(body["data"]["pair"], "USDT/IRT");
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
                .app_data(state_with_ticker("tabdeal:USDT/IRT"))
                .route("/tickers", web::get().to(list_tickers)),
        )
        .await;
        let req = test::TestRequest::get().uri("/tickers").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let tickers = body["data"]["tickers"].as_array().unwrap();
        assert_eq!(tickers.len(), 1);
        assert_eq!(tickers[0]["exchange"], "tabdeal");
        assert_eq!(tickers[0]["pair"], "USDT/IRT");
    }

    #[actix_web::test]
    async fn list_returns_multiple_tickers() {
        let handle1 = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let handle2 = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let mut map = HashMap::new();
        map.insert("tabdeal:USDT/IRT".to_string(), handle1);
        map.insert("tabdeal:BTC/IRT".to_string(), handle2);
        let state = web::Data::new(AppState {
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
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
            exchange_adapters: Arc::new(RwLock::new(adapters)),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: Some(failing_publisher),
            broadcaster,
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        let dummy_req = test::TestRequest::default().to_http_request();
        start_kline_symbol_ticker(
            state,
            ValidatedJson(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
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

    #[actix_web::test]
    async fn closed_candle_published_to_kafka() {
        // Two prices in different 1m windows: 10:00:30 and 10:01:00.
        // The second tick closes the 10:00 candle → publisher must receive it on "candles".
        let price1 = Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRT"),
            bid: 58000,
            ask: 58100,
            timestamp: Utc.with_ymd_and_hms(2026, 6, 20, 10, 0, 30).unwrap(),
        };
        let price2 = Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRT"),
            bid: 59000,
            ask: 59100,
            timestamp: Utc.with_ymd_and_hms(2026, 6, 20, 10, 1, 0).unwrap(),
        };

        let publisher = Arc::new(MockPublisher::new());
        let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(TwoPriceAdapter { price1, price2 }),
        );

        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(adapters)),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: Some(Arc::clone(&publisher) as Arc<dyn crate::kafka::port::MessagePublisher>),
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        let dummy_req = test::TestRequest::default().to_http_request();
        start_kline_symbol_ticker(
            state,
            ValidatedJson(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            }),
        )
        .await
        .respond_to(&dummy_req);

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let messages = publisher.published();
                if messages.iter().any(|(topic, _, payload)| {
                    topic == "candles" && payload.contains("\"type\":\"candle\"")
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("a closed candle must be published to the candles Kafka topic within 5s");
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
            exchange_adapters: Arc::new(RwLock::new(adapters)),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        let dummy_req = test::TestRequest::default().to_http_request();

        let (r1, r2) = tokio::join!(
            start_kline_symbol_ticker(
                state.clone(),
                ValidatedJson(SymbolRequest {
                    exchange: ExchangeId::new("tabdeal"),
                    symbol: TradingPair::new("USDT", "IRT"),
                }),
            ),
            start_kline_symbol_ticker(
                state.clone(),
                ValidatedJson(SymbolRequest {
                    exchange: ExchangeId::new("tabdeal"),
                    symbol: TradingPair::new("USDT", "IRT"),
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
            exchange_adapters: Arc::new(RwLock::new(adapters)),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        let dummy_req = test::TestRequest::default().to_http_request();

        let stop_status = stop_kline_symbol_ticker(
            state.clone(),
            ValidatedJson(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            }),
        )
        .await
        .respond_to(&dummy_req)
        .status();
        assert_eq!(stop_status, StatusCode::OK, "stop must succeed");

        let start_status = start_kline_symbol_ticker(
            state.clone(),
            ValidatedJson(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
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

    /// A `tokio::spawn`ed task whose future has already resolved, so its
    /// `AbortHandle::is_finished()` reports `true` — simulates a ticker
    /// task that died (e.g. the upstream exchange WS dropped) without
    /// anyone calling `stop` to clean up its entry in `clients`.
    async fn finished_abort_handle() -> AbortHandle {
        let join = tokio::spawn(async {});
        let abort = join.abort_handle();
        join.await.expect("the no-op task must not panic");
        abort
    }

    #[actix_web::test]
    async fn start_replaces_a_dead_handle_instead_of_rejecting_as_already_running() {
        // Root cause of a real production bug: a ticker task died (its
        // future resolved) but `clients` still held its `AbortHandle`.
        // `start_kline_symbol_ticker` only ever checked `contains_key`,
        // so every subsequent start attempt was rejected with "Ticker
        // already running" forever — even though nothing was actually
        // running, and the WS feed delivered no further price updates.
        let stale = finished_abort_handle().await;
        assert!(
            stale.is_finished(),
            "the simulated dead task must have already resolved"
        );

        let count = Arc::new(AtomicUsize::new(0));
        let clients: Arc<Mutex<HashMap<String, AbortHandle>>> = Arc::new(Mutex::new({
            let mut m = HashMap::new();
            m.insert("tabdeal:USDT/IRT".to_string(), stale);
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
            exchange_adapters: Arc::new(RwLock::new(adapters)),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::clone(&clients),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a dead handle must not block restarting the ticker"
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "subscribe must be called for the fresh start"
        );
        assert!(
            !clients
                .lock()
                .await
                .get("tabdeal:USDT/IRT")
                .unwrap()
                .is_finished(),
            "the stale handle must be replaced with a live one"
        );
    }

    #[actix_web::test]
    async fn start_still_rejects_when_the_existing_handle_is_actually_alive() {
        let state = state_with_ticker("tabdeal:USDT/IRT");
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "a genuinely running ticker must still be rejected as a duplicate"
        );
    }

    // --- ticker repository persistence tests (ROADMAP 1d) ---

    #[tokio::test(flavor = "current_thread")]
    async fn stop_ticker_removes_db_row() {
        let repo = Arc::new(FakeTickerRepository::new_with(vec![TickerSubscription {
            exchange: "tabdeal".to_string(),
            symbol: "USDT/IRT".to_string(),
        }]));
        let handle = tokio::spawn(std::future::pending::<()>()).abort_handle();
        let mut map = HashMap::new();
        map.insert("tabdeal:USDT/IRT".to_string(), handle);

        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(map)),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: Some(Arc::clone(&repo) as Arc<dyn TickerRepository>),
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        let dummy_req = test::TestRequest::default().to_http_request();
        stop_kline_symbol_ticker(
            state,
            ValidatedJson(SymbolRequest {
                exchange: ExchangeId::new("tabdeal"),
                symbol: TradingPair::new("USDT", "IRT"),
            }),
        )
        .await
        .respond_to(&dummy_req);

        let remaining = repo.list_active().await.unwrap();
        assert!(
            remaining.is_empty(),
            "DB row must be removed when ticker is stopped"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn active_tickers_restored_from_db_on_startup() {
        let repo = Arc::new(FakeTickerRepository::new_with(vec![TickerSubscription {
            exchange: "tabdeal".to_string(),
            symbol: "USDT/IRT".to_string(),
        }]));
        let count = Arc::new(AtomicUsize::new(0));
        let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
        adapters.insert(
            "tabdeal".to_string(),
            Arc::new(CountingAdapter {
                count: Arc::clone(&count),
            }),
        );

        let state = web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(adapters)),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: Some(Arc::clone(&repo) as Arc<dyn TickerRepository>),
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
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
        });

        restore_tickers(&state).await;

        assert!(
            state.clients.lock().await.contains_key("tabdeal:USDT/IRT"),
            "ticker from DB must be restored into the clients map on startup"
        );
    }
}
