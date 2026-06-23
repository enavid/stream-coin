use actix_web::{web, Responder};

use crate::exchange::registry::TradingPairRecord;
use crate::presentation::dto::exchange::{
    ExchangeListResponse, ExchangeNameRequest, ExchangeResponse, PairListQuery, PairListResponse,
    PairResponse, SeedTopPairsQuery, SeedTopPairsResponse,
};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;
use crate::price::entity::MarketType;

/// `GET /v1/exchanges` — returns all currently enabled exchanges.
pub async fn list_exchanges(state: web::Data<AppState>) -> impl Responder {
    let registry = state.exchange_registry.lock().await;
    let exchanges = registry
        .get_enabled_exchanges()
        .into_iter()
        .map(|e| ExchangeResponse {
            name: e.name.clone(),
            display_name: e.display_name.clone(),
            enabled: e.enabled,
        })
        .collect();

    success_response("Enabled exchanges", ExchangeListResponse { exchanges })
}

/// `GET /v1/exchanges/{name}/pairs` — returns active pairs for an exchange,
/// optionally filtered by `?market_type=spot|futures|swap`.
pub async fn list_exchange_pairs(
    state: web::Data<AppState>,
    path: web::Path<String>,
    query: web::Query<PairListQuery>,
) -> impl Responder {
    let exchange_name = path.into_inner();
    let registry = state.exchange_registry.lock().await;
    let pairs = registry
        .get_active_pairs(&exchange_name, query.market_type.as_ref())
        .into_iter()
        .map(|p| PairResponse {
            base: p.base.clone(),
            quote: p.quote.clone(),
            market_type: p.market_type.clone(),
            active: p.active,
        })
        .collect();

    success_response("Active pairs", PairListResponse { pairs })
}

/// `POST /v1/admin/exchanges/enable` — enables an exchange and inserts its adapter
/// into the live adapter map using the registered factory.
pub async fn enable_exchange(
    state: web::Data<AppState>,
    body: web::Json<ExchangeNameRequest>,
) -> impl Responder {
    let name = body.exchange.as_str();

    let ws_url = {
        let mut registry = state.exchange_registry.lock().await;
        if !registry.enable(name) {
            return ApiError::new("Exchange not found in registry", vec![]).to_response();
        }
        registry.find_ws_url(name).map(String::from)
    };

    let ws_url = match ws_url {
        Some(u) => u,
        None => {
            return ApiError::new("Exchange has no WS URL configured", vec![]).to_response();
        }
    };

    if let Some(factory) = state.adapter_factories.get(name) {
        let adapter = factory(&ws_url);
        state
            .exchange_adapters
            .write()
            .await
            .insert(name.to_string(), adapter);
        tracing::info!(exchange = %name, "exchange enabled and adapter inserted");
    }

    if let Some(repo) = &state.exchange_repository {
        if let Err(e) = repo.set_enabled(name, true).await {
            tracing::error!(error = %e, exchange = %name, "failed to persist exchange enable to DB");
        }
    }

    success_response("Exchange enabled", serde_json::json!({"exchange": name}))
}

/// `POST /v1/admin/exchanges/disable` — disables an exchange, removes its adapter,
/// and aborts all running ticker subscriptions for that exchange.
pub async fn disable_exchange(
    state: web::Data<AppState>,
    body: web::Json<ExchangeNameRequest>,
) -> impl Responder {
    let name = body.exchange.as_str();

    {
        let mut registry = state.exchange_registry.lock().await;
        if !registry.disable(name) {
            return ApiError::new("Exchange not found in registry", vec![]).to_response();
        }
    }

    state.exchange_adapters.write().await.remove(name);

    let prefix = format!("{name}:");
    let mut clients = state.clients.lock().await;
    let to_abort: Vec<String> = clients
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
    for key in to_abort {
        if let Some(handle) = clients.remove(&key) {
            handle.abort();
        }
    }

    if let Some(repo) = &state.exchange_repository {
        if let Err(e) = repo.set_enabled(name, false).await {
            tracing::error!(error = %e, exchange = %name, "failed to persist exchange disable to DB");
        }
    }

    tracing::info!(exchange = %name, "exchange disabled and tickers aborted");
    success_response("Exchange disabled", serde_json::json!({"exchange": name}))
}

/// `POST /v1/admin/exchanges/coinex/seed-top-pairs?count=20` — fetches
/// CoinEx's markets ranked by 24h quote volume (via `TopMarketSource`) and
/// seeds the top `count` as active spot trading pairs, in both the in-memory
/// registry and the persistent repository (if configured). A one-shot admin
/// action, not a recurring job — re-running it with the same count is
/// idempotent (`ExchangeRegistry::upsert_pair` / `ExchangeRepository::upsert_pair`).
pub async fn seed_coinex_top_pairs(
    state: web::Data<AppState>,
    query: web::Query<SeedTopPairsQuery>,
) -> impl Responder {
    const EXCHANGE: &str = "coinex";
    let count = query.resolved_count();

    let Some(source) = state.top_market_sources.get(EXCHANGE) else {
        return ApiError::service_unavailable("no top-market source configured for coinex")
            .to_response();
    };

    tracing::info!(
        exchange = EXCHANGE,
        count,
        "seeding top pairs by quote volume"
    );

    let markets = match source.fetch_top_markets(count).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(
                exchange = EXCHANGE,
                error = %e,
                transient = e.is_transient(),
                "top-pair seeding fetch failed"
            );
            return if e.is_transient() {
                ApiError::service_unavailable(&format!("upstream exchange unavailable: {e}"))
            } else {
                ApiError::new(&format!("seeding rejected by exchange: {e}"), vec![])
            }
            .to_response();
        }
    };

    let mut pairs_seeded = 0usize;
    for market in &markets {
        let pair = crate::exchange::coinex::market_to_pair(&market.market);
        let record = TradingPairRecord {
            exchange_name: EXCHANGE.to_string(),
            base: pair.base,
            quote: pair.quote,
            market_type: MarketType::Spot,
            active: true,
        };

        state
            .exchange_registry
            .lock()
            .await
            .upsert_pair(record.clone());

        if let Some(repo) = &state.exchange_repository {
            if let Err(e) = repo.upsert_pair(&record).await {
                tracing::error!(
                    exchange = EXCHANGE,
                    base = %record.base,
                    quote = %record.quote,
                    error = %e,
                    "failed to persist seeded pair to db"
                );
                continue;
            }
        }

        pairs_seeded += 1;
    }

    tracing::info!(
        exchange = EXCHANGE,
        pairs_seeded,
        "top-pair seeding complete"
    );

    success_response("Top pairs seeded", SeedTopPairsResponse { pairs_seeded })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_web::{test, web, App};
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::exchange::registry::{ExchangeRecord, ExchangeRegistry, TradingPairRecord};
    use crate::infrastructure::db::exchange_repository::{
        ExchangeRepository, FakeExchangeRepository,
    };
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};
    use crate::price::entity::MarketType;

    fn registry_with_exchanges() -> ExchangeRegistry {
        let mut r = ExchangeRegistry::new();
        r.add_exchange(ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://tabdeal.example.com".to_string(),
            enabled: true,
        });
        r.add_exchange(ExchangeRecord {
            name: "hitobit".to_string(),
            display_name: "Hitobit".to_string(),
            ws_url: "wss://hitobit.example.com".to_string(),
            enabled: false,
        });
        r
    }

    fn state_with_registry(registry: ExchangeRegistry) -> web::Data<AppState> {
        state_with_registry_and_repo(registry, None)
    }

    fn state_with_registry_and_repo(
        registry: ExchangeRegistry,
        exchange_repository: Option<Arc<dyn ExchangeRepository>>,
    ) -> web::Data<AppState> {
        web::Data::new(AppState {
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
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            historical_sources: Arc::new(HashMap::new()),
            top_market_sources: Arc::new(HashMap::new()),
            candle_history: AppState::new_candle_history(),
            exchange_repository,
            user_repository: None,
            credential_repository: None,
            credential_cipher: None,
        })
    }

    #[actix_web::test]
    async fn list_exchanges_returns_only_enabled() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_registry(registry_with_exchanges()))
                .route("/exchanges", web::get().to(list_exchanges)),
        )
        .await;

        let req = test::TestRequest::get().uri("/exchanges").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let exchanges = body["data"]["exchanges"].as_array().unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0]["name"], "tabdeal");
    }

    #[actix_web::test]
    async fn disable_then_enable_exchange_changes_list() {
        let registry = registry_with_exchanges();
        let state = state_with_registry(registry);

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/exchanges", web::get().to(list_exchanges))
                .route("/admin/exchanges/disable", web::post().to(disable_exchange))
                .route("/admin/exchanges/enable", web::post().to(enable_exchange)),
        )
        .await;

        let disable_req = test::TestRequest::post()
            .uri("/admin/exchanges/disable")
            .set_json(serde_json::json!({"exchange": "tabdeal"}))
            .to_request();
        let disable_resp = test::call_service(&app, disable_req).await;
        assert_eq!(disable_resp.status(), 200);

        let list_req = test::TestRequest::get().uri("/exchanges").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, list_req).await;
        assert_eq!(
            body["data"]["exchanges"].as_array().unwrap().len(),
            0,
            "no enabled exchanges after disable"
        );
    }

    #[actix_web::test]
    async fn list_pairs_returns_200() {
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

        let app = test::init_service(App::new().app_data(state_with_registry(registry)).route(
            "/exchanges/{name}/pairs",
            web::get().to(list_exchange_pairs),
        ))
        .await;

        let req = test::TestRequest::get()
            .uri("/exchanges/tabdeal/pairs")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn disable_exchange_persists_to_repository() {
        let repo = Arc::new(FakeExchangeRepository::new_with(
            vec![ExchangeRecord {
                name: "tabdeal".to_string(),
                display_name: "Tabdeal".to_string(),
                ws_url: "wss://tabdeal.example.com".to_string(),
                enabled: true,
            }],
            vec![],
        ));
        let state = state_with_registry_and_repo(
            registry_with_exchanges(),
            Some(repo.clone() as Arc<dyn ExchangeRepository>),
        );

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/admin/exchanges/disable", web::post().to(disable_exchange)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/exchanges/disable")
            .set_json(serde_json::json!({"exchange": "tabdeal"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let (exchanges, _) = repo.load_all().await.unwrap();
        assert!(
            !exchanges[0].enabled,
            "disable must persist enabled=false to the repository"
        );
    }

    #[actix_web::test]
    async fn enable_exchange_persists_to_repository() {
        let repo = Arc::new(FakeExchangeRepository::new_with(
            vec![ExchangeRecord {
                name: "hitobit".to_string(),
                display_name: "Hitobit".to_string(),
                ws_url: "wss://hitobit.example.com".to_string(),
                enabled: false,
            }],
            vec![],
        ));
        let state = state_with_registry_and_repo(
            registry_with_exchanges(),
            Some(repo.clone() as Arc<dyn ExchangeRepository>),
        );

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/admin/exchanges/enable", web::post().to(enable_exchange)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/exchanges/enable")
            .set_json(serde_json::json!({"exchange": "hitobit"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let (exchanges, _) = repo.load_all().await.unwrap();
        assert!(
            exchanges[0].enabled,
            "enable must persist enabled=true to the repository"
        );
    }

    // --- seed_coinex_top_pairs ---

    use crate::exchange::market_seed_port::{MarketSeederError, MarketVolume, TopMarketSource};

    struct FakeTopMarketSource {
        markets: Vec<MarketVolume>,
    }

    #[async_trait::async_trait]
    impl TopMarketSource for FakeTopMarketSource {
        fn exchange_id(&self) -> crate::exchange::entity::ExchangeId {
            crate::exchange::entity::ExchangeId::new("coinex")
        }

        async fn fetch_top_markets(
            &self,
            count: usize,
        ) -> Result<Vec<MarketVolume>, MarketSeederError> {
            Ok(self.markets.iter().take(count).cloned().collect())
        }
    }

    fn fake_markets(n: usize) -> Vec<MarketVolume> {
        (0..n)
            .map(|i| MarketVolume {
                market: format!("COIN{i}USDT"),
                quote_volume: (n - i) as u64 * 1000,
                status: "online".to_string(),
            })
            .collect()
    }

    fn state_with_top_market_source(
        registry: ExchangeRegistry,
        exchange_repository: Option<Arc<dyn ExchangeRepository>>,
        markets: Vec<MarketVolume>,
    ) -> web::Data<AppState> {
        let mut sources: HashMap<String, Arc<dyn TopMarketSource>> = HashMap::new();
        sources.insert(
            "coinex".to_string(),
            Arc::new(FakeTopMarketSource { markets }) as Arc<dyn TopMarketSource>,
        );

        web::Data::new(AppState {
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
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            historical_sources: Arc::new(HashMap::new()),
            top_market_sources: Arc::new(sources),
            candle_history: AppState::new_candle_history(),
            exchange_repository,
            user_repository: None,
            credential_repository: None,
            credential_cipher: None,
        })
    }

    fn coinex_exchange_repo() -> Arc<FakeExchangeRepository> {
        Arc::new(FakeExchangeRepository::new_with(
            vec![ExchangeRecord {
                name: "coinex".to_string(),
                display_name: "CoinEx".to_string(),
                ws_url: "wss://socket.coinex.com/v2/spot".to_string(),
                enabled: false,
            }],
            vec![],
        ))
    }

    #[actix_web::test]
    async fn seed_top_pairs_returns_503_when_no_source_configured() {
        let state = state_with_registry(ExchangeRegistry::new());

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/exchanges/coinex/seed-top-pairs",
            web::post().to(seed_coinex_top_pairs),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/exchanges/coinex/seed-top-pairs")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 503);
    }

    #[actix_web::test]
    async fn seed_top_pairs_inserts_exactly_count_rows() {
        let repo = coinex_exchange_repo();
        let state = state_with_top_market_source(
            ExchangeRegistry::new(),
            Some(repo.clone() as Arc<dyn ExchangeRepository>),
            fake_markets(25),
        );

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/exchanges/coinex/seed-top-pairs",
            web::post().to(seed_coinex_top_pairs),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/exchanges/coinex/seed-top-pairs?count=20")
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["pairs_seeded"], 20);

        let (_, pairs) = repo.load_all().await.unwrap();
        assert_eq!(pairs.len(), 20, "must persist exactly count rows");
    }

    #[actix_web::test]
    async fn seed_top_pairs_is_idempotent_on_rerun() {
        let repo = coinex_exchange_repo();
        let registry = Arc::new(Mutex::new(ExchangeRegistry::new()));
        let state = {
            let mut sources: HashMap<String, Arc<dyn TopMarketSource>> = HashMap::new();
            sources.insert(
                "coinex".to_string(),
                Arc::new(FakeTopMarketSource {
                    markets: fake_markets(20),
                }) as Arc<dyn TopMarketSource>,
            );
            web::Data::new(AppState {
                redis: None,
                exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
                exchange_registry: registry.clone(),
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
                top_market_sources: Arc::new(sources),
                candle_history: AppState::new_candle_history(),
                exchange_repository: Some(repo.clone() as Arc<dyn ExchangeRepository>),
                user_repository: None,
                credential_repository: None,
                credential_cipher: None,
            })
        };

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/exchanges/coinex/seed-top-pairs",
            web::post().to(seed_coinex_top_pairs),
        ))
        .await;

        for _ in 0..2 {
            let req = test::TestRequest::post()
                .uri("/admin/exchanges/coinex/seed-top-pairs?count=20")
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert_eq!(resp.status(), 200);
        }

        let (_, pairs) = repo.load_all().await.unwrap();
        assert_eq!(
            pairs.len(),
            20,
            "re-running with the same count must not duplicate rows"
        );
        assert_eq!(
            registry.lock().await.get_active_pairs("coinex", None).len(),
            20,
            "in-memory registry must also stay idempotent"
        );
    }

    #[actix_web::test]
    async fn seed_top_pairs_with_fewer_markets_than_count_seeds_all_available() {
        let repo = coinex_exchange_repo();
        let state = state_with_top_market_source(
            ExchangeRegistry::new(),
            Some(repo.clone() as Arc<dyn ExchangeRepository>),
            fake_markets(5),
        );

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/exchanges/coinex/seed-top-pairs",
            web::post().to(seed_coinex_top_pairs),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/exchanges/coinex/seed-top-pairs?count=20")
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["pairs_seeded"], 5);
    }
}
