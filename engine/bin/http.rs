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

use stream_coin::exchange::coinex::{CoinexHistoricalAdapter, CoinexMarketSeeder, CoinexWsAdapter};
use stream_coin::exchange::historical_port::HistoricalCandleSource;
use stream_coin::exchange::hitobit::HitobitWsAdapter;
use stream_coin::exchange::market_seed_port::TopMarketSource;
use stream_coin::exchange::port::ExchangeAdapter;
use stream_coin::exchange::registry::{ExchangeRecord, ExchangeRegistry, TradingPairRecord};
use stream_coin::exchange::tabdeal::TabdealWsAdapter;
use stream_coin::infrastructure::cache::redis;
use stream_coin::infrastructure::crypto::credential_cipher::CredentialCipher;
use stream_coin::infrastructure::db::candle_repository::CandleRepository;
use stream_coin::infrastructure::db::credential_repository::CredentialRepository;
use stream_coin::infrastructure::db::exchange_repository::ExchangeRepository;
use stream_coin::infrastructure::db::order_repository::FakeOrderRepository;
use stream_coin::infrastructure::db::postgres::{
    PostgresCandleRepository, PostgresCredentialRepository, PostgresExchangeRepository,
    PostgresTickerRepository, PostgresUserRepository,
};
use stream_coin::infrastructure::db::ticker_repository::TickerRepository;
use stream_coin::infrastructure::db::user_repository::{seed_admin_if_empty, UserRepository};
use stream_coin::kafka::port::MessagePublisher;
use stream_coin::kafka::KafkaProducer;
use stream_coin::order::entity::SafetyConfig;
use stream_coin::order::manager::{spawn_order_manager_listener, OrderManager};
use stream_coin::order::port::OrderAdapter;
use stream_coin::order::tabdeal::TabdealOrderAdapter;
use stream_coin::presentation::handlers::exchange_handler::restore_tickers;
use stream_coin::presentation::handlers::strategy_handler::restore_python_strategies;
use stream_coin::presentation::middlewares::cors::configure_cors;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
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

    // Hard-coded registry of top-market-by-volume sources — same sparsity
    // rationale as `historical_sources`.
    let mut top_market_sources: HashMap<String, Arc<dyn TopMarketSource>> = HashMap::new();
    top_market_sources.insert(
        "coinex".to_string(),
        Arc::new(CoinexMarketSeeder::new()) as Arc<dyn TopMarketSource>,
    );
    let top_market_sources = Arc::new(top_market_sources);

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

    let jwt_secret = match env::var("JWT_SECRET") {
        Ok(s) if !s.is_empty() => {
            tracing::info!("JWT auth enabled");
            Some(Arc::new(s))
        }
        _ => {
            tracing::warn!("JWT_SECRET not set — running without authentication");
            None
        }
    };

    let broadcaster = AppState::new_broadcaster();

    // Order adapters are configured at runtime via POST /v1/admin/exchanges/{name}/credentials.
    // API keys are never read from environment variables — they are set by operators through
    // the API after startup. Set TABDEAL_API_KEY for convenience in development only.
    let mut order_adapter_map: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
    if let Ok(api_key) = env::var("TABDEAL_API_KEY") {
        order_adapter_map.insert(
            "tabdeal".to_string(),
            Arc::new(TabdealOrderAdapter::new(&api_key)),
        );
        tracing::info!("tabdeal order adapter loaded from env (use API in production)");
    } else {
        tracing::info!(
            "TABDEAL_API_KEY not set — configure via POST /v1/admin/exchanges/tabdeal/credentials"
        );
    }
    let order_adapters = Arc::new(tokio::sync::RwLock::new(order_adapter_map));

    let order_repository = Arc::new(FakeOrderRepository::new());

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

    let order_manager = Arc::new(OrderManager::new(
        order_adapters.clone(),
        order_repository,
        broadcaster.clone(),
        safety_config,
    ));
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
        python_strategy_repository: None,
        candle_repository,
        candle_history: AppState::new_candle_history(),
        historical_sources,
        top_market_sources,
        exchange_repository,
        user_repository,
        credential_repository,
        credential_cipher,
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
