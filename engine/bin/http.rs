use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use actix_web::{web, App, HttpResponse, HttpServer};
use dotenv::dotenv;
use serde_json::json;
use tokio::sync::{Mutex, RwLock};
use tracing_actix_web::TracingLogger;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use stream_coin::exchange::coinex::{CoinexHistoricalAdapter, CoinexWsAdapter};
use stream_coin::exchange::historical_port::HistoricalCandleSource;
use stream_coin::exchange::hitobit::HitobitWsAdapter;
use stream_coin::exchange::port::ExchangeAdapter;
use stream_coin::exchange::registry::{ExchangeRecord, ExchangeRegistry, TradingPairRecord};
use stream_coin::exchange::tabdeal::TabdealWsAdapter;
use stream_coin::infrastructure::cache::redis;
use stream_coin::infrastructure::config::{reject_placeholder_secret, resolve_or_default};
use stream_coin::infrastructure::crypto::credential_cipher::CredentialCipher;
use stream_coin::infrastructure::db::asset_repository::AssetRepository;
use stream_coin::infrastructure::db::candle_repository::CandleRepository;
use stream_coin::infrastructure::db::credential_repository::CredentialRepository;
use stream_coin::infrastructure::db::exchange_repository::ExchangeRepository;
use stream_coin::infrastructure::db::order_repository::{FakeOrderRepository, OrderRepository};
use stream_coin::infrastructure::db::postgres::{
    PostgresAssetRepository, PostgresCandleRepository, PostgresCircuitBreakerStore,
    PostgresCredentialRepository, PostgresExchangeRepository, PostgresOrderRepository,
    PostgresPythonStrategyRepository, PostgresSubscriptionRepository, PostgresTickerRepository,
    PostgresUserRepository,
};
use stream_coin::infrastructure::db::python_strategy_repository::PythonStrategyRepository;
use stream_coin::infrastructure::db::subscription_repository::SubscriptionRepository;
use stream_coin::infrastructure::db::ticker_repository::TickerRepository;
use stream_coin::infrastructure::db::user_repository::{seed_admin_if_empty, UserRepository};
use stream_coin::kafka::port::MessagePublisher;
use stream_coin::kafka::KafkaProducer;
use stream_coin::order::credential_resolver::{LiveCredentialResolver, OrderAdapterFactory};
use stream_coin::order::entity::SafetyConfig;
use stream_coin::order::exir::ExirOrderAdapter;
use stream_coin::order::hitobit::HitobitOrderAdapter;
use stream_coin::order::manager::{spawn_order_manager_listener, OrderManager};
use stream_coin::order::port::OrderAdapter;
use stream_coin::order::tabdeal::TabdealOrderAdapter;
use stream_coin::presentation::handlers::exchange_handler::restore_tickers;
use stream_coin::presentation::handlers::strategy_handler::restore_python_strategies;
use stream_coin::presentation::middlewares::cors::configure_cors;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
use stream_coin::presentation::middlewares::jwt::resolve_jwt_secret;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};
use stream_coin::presentation::swagger::ApiDoc;
use stream_coin::price::entity::MarketType;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    match env::var("LOG_FORMAT").as_deref() {
        Ok("json") => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_current_span(false)
                .init();
        }
        _ => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }

    let (host, port) = (
        env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        env::var("PORT").unwrap_or_else(|_| "8080".to_string()),
    );

    let redis_conn = match env::var("REDIS_URL") {
        Ok(url) => match redis::establish_redis_connection(&url).await {
            Ok(conn) => {
                tracing::info!(url = %url, "redis connected");
                Some(conn)
            }
            Err(e) => {
                tracing::warn!(error = %e, "redis unavailable, starting without cache");
                None
            }
        },
        Err(_) => {
            tracing::warn!("REDIS_URL not set, starting without cache");
            None
        }
    };

    // Hard-coded factory map: the only place exchange names appear in code.
    // The registry (DB-backed when DATABASE_URL is set) controls which are active;
    // this map provides constructors.
    // NEVER add: nobitex, wallex, bitpin, ramzinex — OFAC sanctioned 2026-06-02
    let mut factories: HashMap<String, AdapterFactory> = HashMap::new();
    factories.insert(
        "tabdeal".to_string(),
        Arc::new(|ws_url: &str| {
            Arc::new(TabdealWsAdapter::with_url(ws_url.to_string())) as Arc<dyn ExchangeAdapter>
        }),
    );
    factories.insert(
        "hitobit".to_string(),
        Arc::new(|ws_url: &str| {
            Arc::new(HitobitWsAdapter::with_url(ws_url.to_string())) as Arc<dyn ExchangeAdapter>
        }),
    );
    factories.insert(
        "coinex".to_string(),
        Arc::new(|ws_url: &str| {
            Arc::new(CoinexWsAdapter::with_url(ws_url.to_string())) as Arc<dyn ExchangeAdapter>
        }),
    );
    let adapter_factories = Arc::new(factories);

    // Hard-coded registry of historical REST candle sources — mirrors
    // `adapter_factories` above but deliberately sparse: only exchanges with
    // a public historical-kline endpoint get an entry (Tabdeal/Hitobit do not).
    let mut historical_sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    historical_sources.insert(
        "coinex".to_string(),
        Arc::new(CoinexHistoricalAdapter::new()) as Arc<dyn HistoricalCandleSource>,
    );
    let historical_sources = Arc::new(historical_sources);

    // Fail closed on any secret left at its shipped `change-me` placeholder —
    // a default JWT key, DB password, or encryption key in a real deployment is
    // a critical hole. Checked before anything connects or serves.
    for var in ["JWT_SECRET", "DATABASE_URL", "CREDENTIALS_ENCRYPTION_KEY"] {
        if let Ok(value) = env::var(var) {
            if let Err(e) = reject_placeholder_secret(var, &value) {
                tracing::error!("{e}");
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, e));
            }
        }
    }

    let db_pool: Option<sqlx::PgPool> = match env::var("DATABASE_URL") {
        Ok(url) => match sqlx::PgPool::connect(&url).await {
            Ok(pool) => match sqlx::migrate!("./migrations").run(&pool).await {
                Ok(()) => {
                    tracing::info!(url = %url, "postgres connected, migrations applied");
                    Some(pool)
                }
                Err(e) => {
                    tracing::error!(error = %e, "database migration failed");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "postgres unavailable, starting without DB persistence");
                None
            }
        },
        Err(_) => {
            tracing::warn!("DATABASE_URL not set, starting without DB persistence");
            None
        }
    };

    let exchange_repository: Option<Arc<dyn ExchangeRepository>> = db_pool
        .clone()
        .map(|pool| Arc::new(PostgresExchangeRepository::new(pool)) as Arc<dyn ExchangeRepository>);

    let asset_repository: Option<Arc<dyn AssetRepository>> = db_pool
        .clone()
        .map(|pool| Arc::new(PostgresAssetRepository::new(pool)) as Arc<dyn AssetRepository>);

    let ticker_repository: Option<Arc<dyn TickerRepository>> = db_pool
        .clone()
        .map(|pool| Arc::new(PostgresTickerRepository::new(pool)) as Arc<dyn TickerRepository>);

    let user_repository: Option<Arc<dyn UserRepository>> = db_pool
        .clone()
        .map(|pool| Arc::new(PostgresUserRepository::new(pool)) as Arc<dyn UserRepository>);

    let credential_repository: Option<Arc<dyn CredentialRepository>> =
        db_pool.clone().map(|pool| {
            Arc::new(PostgresCredentialRepository::new(pool)) as Arc<dyn CredentialRepository>
        });

    let candle_repository: Option<Arc<dyn CandleRepository>> = db_pool
        .clone()
        .map(|pool| Arc::new(PostgresCandleRepository::new(pool)) as Arc<dyn CandleRepository>);

    let subscription_repository: Option<Arc<dyn SubscriptionRepository>> =
        db_pool.clone().map(|pool| {
            Arc::new(PostgresSubscriptionRepository::new(pool)) as Arc<dyn SubscriptionRepository>
        });

    let python_strategy_repository: Option<Arc<dyn PythonStrategyRepository>> =
        db_pool.clone().map(|pool| {
            Arc::new(PostgresPythonStrategyRepository::new(pool))
                as Arc<dyn PythonStrategyRepository>
        });

    let credential_cipher = match CredentialCipher::from_env() {
        Some(c) => {
            tracing::info!("credential encryption configured");
            Some(Arc::new(c))
        }
        None => {
            tracing::warn!(
                "CREDENTIALS_ENCRYPTION_KEY not set or invalid — exchange credential endpoints return 503"
            );
            None
        }
    };

    // Bootstrap the registry from the database when available; fall back to hardcoded
    // defaults otherwise (dev without Postgres, or migration 0007 hasn't been seeded yet).
    let mut registry = ExchangeRegistry::new();
    let mut loaded_from_db = false;
    if let Some(repo) = &exchange_repository {
        match repo.load_all().await {
            Ok((exchanges, pairs)) if !exchanges.is_empty() => {
                for exchange in exchanges {
                    registry.add_exchange(exchange);
                }
                for pair in pairs {
                    registry.add_pair(pair);
                }
                tracing::info!("exchange registry loaded from database");
                loaded_from_db = true;
            }
            Ok(_) => {
                tracing::warn!("exchanges table is empty, falling back to hardcoded registry");
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to load exchange registry from database, falling back to hardcoded registry");
            }
        }
    }
    if !loaded_from_db {
        registry.add_exchange(ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://api1.tabdeal.org/stream/".to_string(),
            enabled: true,
        });
        registry.add_exchange(ExchangeRecord {
            name: "hitobit".to_string(),
            display_name: "Hitobit".to_string(),
            ws_url: "wss://stream.hitobit.com/stream".to_string(),
            enabled: true,
        });
        registry.add_exchange(ExchangeRecord {
            name: "coinex".to_string(),
            display_name: "CoinEx".to_string(),
            ws_url: "wss://socket.coinex.com/v2/spot".to_string(),
            enabled: false,
        });
        registry.add_pair(TradingPairRecord {
            exchange_name: "tabdeal".to_string(),
            base: "USDT".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Spot,
            active: true,
        });
        registry.add_pair(TradingPairRecord {
            exchange_name: "hitobit".to_string(),
            base: "USDT".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Spot,
            active: true,
        });
    }

    // Build the live adapter map from the registry — only enabled exchanges get adapters.
    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    for exchange in registry.get_enabled_exchanges() {
        if let Some(factory) = adapter_factories.get(&exchange.name) {
            adapters.insert(exchange.name.clone(), factory(&exchange.ws_url));
            tracing::info!(exchange = %exchange.name, "adapter loaded from registry");
        }
    }

    let publisher: Option<Arc<dyn MessagePublisher>> = match env::var("KAFKA_URL") {
        Ok(url) => match KafkaProducer::new(&url) {
            Ok(p) => {
                tracing::info!(url = %url, "kafka producer connected");
                Some(Arc::new(p))
            }
            Err(e) => {
                tracing::warn!(error = %e, "kafka unavailable, starting without publisher");
                None
            }
        },
        Err(_) => {
            tracing::warn!("KAFKA_URL not set, starting without publisher");
            None
        }
    };

    let allow_insecure = env::var("ALLOW_INSECURE_NO_AUTH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let jwt_secret =
        match resolve_jwt_secret(env::var("JWT_SECRET").ok().as_deref(), allow_insecure) {
            Ok(Some(s)) => {
                tracing::info!("JWT auth enabled");
                Some(Arc::new(s))
            }
            Ok(None) => {
                tracing::warn!(
                "ALLOW_INSECURE_NO_AUTH set — running WITHOUT authentication (development only)"
            );
                None
            }
            Err(e) => {
                tracing::error!("{e}");
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, e));
            }
        };

    let broadcaster = AppState::new_broadcaster();

    // Order adapters are configured at runtime via POST /v1/admin/exchanges/{name}/credentials.
    // API keys are never read from environment variables — they are set by operators through
    // the API after startup. Set TABDEAL_API_KEY for convenience in development only.
    let mut order_adapter_map: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
    if let Ok(api_key) = env::var("TABDEAL_API_KEY") {
        // TABDEAL_API_SECRET is optional: when set, the system adapter signs its
        // requests (C10); without it the adapter is unsigned (logged per request).
        let api_secret = env::var("TABDEAL_API_SECRET").ok();
        order_adapter_map.insert(
            "tabdeal".to_string(),
            Arc::new(TabdealOrderAdapter::with_credentials(
                "https://api1.tabdeal.org",
                &api_key,
                api_secret,
            )),
        );
        tracing::info!("tabdeal order adapter loaded from env (use API in production)");
    } else {
        tracing::info!(
            "TABDEAL_API_KEY not set — configure via POST /v1/admin/exchanges/tabdeal/credentials"
        );
    }
    let order_adapters = Arc::new(tokio::sync::RwLock::new(order_adapter_map));

    // Persist orders in Postgres when a DB is configured (M11) so open-order
    // state, position limits and timeout reconciliation survive a restart; fall
    // back to the in-memory repo only when running without a database.
    let order_repository: Arc<dyn OrderRepository> = match db_pool.clone() {
        Some(pool) => {
            tracing::info!("order repository: Postgres (orders persisted)");
            Arc::new(PostgresOrderRepository::new(pool))
        }
        None => {
            tracing::warn!(
                "order repository: in-memory fake — orders are NOT persisted across restarts"
            );
            Arc::new(FakeOrderRepository::new())
        }
    };

    // Safety config driven by environment variables so operators can tune without redeploying.
    // All values fall back to SafetyConfig::default() which has dry_run=true for safety.
    let safety_config = {
        let mut cfg = SafetyConfig::default();
        if let Ok(v) = env::var("DRY_RUN") {
            cfg.dry_run = v.to_lowercase() != "false";
        }
        if let Ok(v) = env::var("MIN_CONFIDENCE") {
            if let Ok(f) = v.parse::<f64>() {
                cfg.min_confidence = f;
            }
        }
        if let Ok(v) = env::var("MAX_POSITION_SIZE") {
            if let Ok(d) = v.parse::<rust_decimal::Decimal>() {
                cfg.max_position_size = d;
            }
        }
        if let Ok(v) = env::var("DEFAULT_ORDER_QUANTITY") {
            if let Ok(d) = v.parse::<rust_decimal::Decimal>() {
                cfg.default_order_quantity = d;
            }
        }
        if let Ok(v) = env::var("CIRCUIT_BREAKER_MAX_ORDERS") {
            if let Ok(n) = v.parse::<u32>() {
                cfg.circuit_breaker_max_orders = n;
            }
        }
        if let Ok(v) = env::var("CIRCUIT_BREAKER_WINDOW_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                cfg.circuit_breaker_window_secs = n;
            }
        }
        cfg
    };
    tracing::info!(
        dry_run = safety_config.dry_run,
        min_confidence = safety_config.min_confidence,
        max_position_size = %safety_config.max_position_size,
        circuit_breaker_max_orders = safety_config.circuit_breaker_max_orders,
        "order manager starting"
    );

    // Seeds the very first admin account from env vars, once, when `users` is empty.
    // After this, accounts are created via POST /v1/admin/users — env vars are a
    // one-time bootstrap, not an ongoing login path.
    if let Some(repo) = &user_repository {
        match (env::var("ADMIN_USERNAME"), env::var("ADMIN_PASSWORD")) {
            (Ok(u), Ok(p)) if !u.is_empty() && !p.is_empty() => {
                match seed_admin_if_empty(repo.as_ref(), &u, &p).await {
                    Ok(()) => tracing::info!("admin account bootstrap checked"),
                    Err(e) => tracing::error!(error = %e, "failed to seed admin account"),
                }
            }
            _ => {
                tracing::warn!("ADMIN_USERNAME/ADMIN_PASSWORD not set — skipping admin bootstrap");
            }
        }
    }

    // Build per-exchange adapter factories for credential-based order placement.
    // Each factory decrypts a user's credentials JSON and constructs a signed
    // adapter. `api_key` is required; `api_secret` is optional but, when present,
    // turns on per-request HMAC signing (C10).
    fn extract_key(creds: &serde_json::Value, exchange: &str) -> Result<String, String> {
        creds["api_key"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("{exchange} credentials must contain 'api_key'"))
    }
    fn extract_secret(creds: &serde_json::Value) -> Option<String> {
        creds["api_secret"].as_str().map(str::to_string)
    }

    // Broker REST base URLs: production defaults baked into each adapter, but
    // overridable per deployment (e.g. a sandbox in staging) without a rebuild.
    let base_url =
        |var: &str, default: &str| resolve_or_default(env::var(var).ok().as_deref(), default);
    let tabdeal_base_url = base_url(
        "TABDEAL_REST_BASE_URL",
        TabdealOrderAdapter::DEFAULT_BASE_URL,
    );
    let hitobit_base_url = base_url(
        "HITOBIT_REST_BASE_URL",
        HitobitOrderAdapter::DEFAULT_BASE_URL,
    );
    let exir_base_url = base_url("EXIR_REST_BASE_URL", ExirOrderAdapter::DEFAULT_BASE_URL);

    let mut order_adapter_factories: HashMap<String, OrderAdapterFactory> = HashMap::new();
    order_adapter_factories.insert(
        "tabdeal".to_string(),
        Arc::new(move |creds: &serde_json::Value| {
            Ok(Arc::new(TabdealOrderAdapter::with_credentials(
                &tabdeal_base_url,
                &extract_key(creds, "tabdeal")?,
                extract_secret(creds),
            )) as Arc<dyn OrderAdapter>)
        }),
    );
    order_adapter_factories.insert(
        "hitobit".to_string(),
        Arc::new(move |creds: &serde_json::Value| {
            Ok(Arc::new(HitobitOrderAdapter::with_credentials(
                &hitobit_base_url,
                &extract_key(creds, "hitobit")?,
                extract_secret(creds),
            )) as Arc<dyn OrderAdapter>)
        }),
    );
    order_adapter_factories.insert(
        "exir".to_string(),
        Arc::new(move |creds: &serde_json::Value| {
            Ok(Arc::new(ExirOrderAdapter::with_credentials(
                &exir_base_url,
                &extract_key(creds, "exir")?,
                extract_secret(creds),
            )) as Arc<dyn OrderAdapter>)
        }),
    );

    let credential_resolver: Option<
        Arc<dyn stream_coin::order::credential_resolver::CredentialResolver>,
    > = match (credential_repository.clone(), credential_cipher.clone()) {
        (Some(repo), Some(cipher)) => {
            let resolver = LiveCredentialResolver::new(repo, cipher, order_adapter_factories);
            tracing::info!("credential resolver configured — per-user order adapters enabled");
            Some(Arc::new(resolver))
        }
        _ => {
            tracing::warn!(
                "credential resolver not configured \
                     (CREDENTIALS_ENCRYPTION_KEY unset or no DB) — \
                     admin order-for-user endpoints return errors"
            );
            None
        }
    };

    let mut order_manager_builder = OrderManager::new(
        order_adapters.clone(),
        order_repository,
        broadcaster.clone(),
        safety_config,
    );
    if let Some(sub_repo) = subscription_repository.clone() {
        order_manager_builder = order_manager_builder.with_subscription_repository(sub_repo);
        tracing::info!("order manager: subscription fanout enabled");
    }
    if let Some(resolver) = credential_resolver {
        order_manager_builder = order_manager_builder.with_credential_resolver(resolver);
        tracing::info!("order manager: per-user credential resolver attached");
    }
    // Persist the circuit-breaker trip when a DB is configured (M9) so a halt
    // survives a restart / crash-loop instead of silently re-arming.
    if let Some(pool) = db_pool.clone() {
        order_manager_builder = order_manager_builder
            .with_circuit_breaker_store(Arc::new(PostgresCircuitBreakerStore::new(pool)));
        tracing::info!("order manager: circuit breaker state persisted to Postgres");
    }
    let order_manager = Arc::new(order_manager_builder);
    // Hydrate the breaker from persisted state before serving any orders.
    order_manager.restore_circuit_breaker().await;
    let _listener_handle =
        spawn_order_manager_listener(Arc::clone(&order_manager), broadcaster.clone());

    let app_state = web::Data::new(AppState {
        redis: redis_conn,
        exchange_adapters: Arc::new(RwLock::new(adapters)),
        exchange_registry: Arc::new(Mutex::new(registry)),
        adapter_factories,
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher,
        broadcaster,
        jwt_secret,
        ticker_repository,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters,
        order_manager: Some(order_manager),
        python_strategy_repository,
        candle_repository,
        candle_history: AppState::new_candle_history(),
        historical_sources,
        exchange_repository,
        asset_repository,
        user_repository,
        credential_repository,
        credential_cipher,
        subscription_repository,
    });

    restore_tickers(&app_state).await;
    restore_python_strategies(&app_state).await;

    tracing::info!(host = %host, port = %port, "server starting");

    HttpServer::new(move || {
        App::new()
            .configure(init_routes)
            .wrap(configure_cors())
            .wrap(TracingLogger::default())
            .app_data(app_state.clone())
            .app_data(json_error_handler_config())
            .default_service(web::route().to(|| async {
                HttpResponse::NotFound().json(json!({
                    "success": false,
                    "code": 404,
                    "message": "Resource not found"
                }))
            }))
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-docs/openapi.json", ApiDoc::openapi()),
            )
    })
    .bind(format!("{}:{}", host, port))?
    .run()
    .await
}
