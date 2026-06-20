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

use stream_coin::exchange::hitobit::HitobitWsAdapter;
use stream_coin::exchange::port::ExchangeAdapter;
use stream_coin::exchange::registry::{ExchangeRecord, ExchangeRegistry, TradingPairRecord};
use stream_coin::exchange::tabdeal::TabdealWsAdapter;
use stream_coin::infrastructure::cache::redis;
use stream_coin::infrastructure::db::postgres::PostgresTickerRepository;
use stream_coin::infrastructure::db::ticker_repository::TickerRepository;
use stream_coin::kafka::port::MessagePublisher;
use stream_coin::kafka::KafkaProducer;
use stream_coin::presentation::handlers::exchange_handler::restore_tickers;
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
    // The registry (DB in 1d) controls which are active; this map provides constructors.
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
    let adapter_factories = Arc::new(factories);

    // Bootstrap registry from environment. In Loop 1d this moves to PostgreSQL.
    let mut registry = ExchangeRegistry::new();
    registry.add_exchange(ExchangeRecord {
        name: "tabdeal".to_string(),
        display_name: "Tabdeal".to_string(),
        ws_url: "wss://api1.tabdeal.org/stream/".to_string(),
        enabled: true,
    });
    registry.add_exchange(ExchangeRecord {
        name: "hitobit".to_string(),
        display_name: "Hitobit".to_string(),
        ws_url: "wss://stream.hitobit.com:443".to_string(),
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
        exchange_name: "hitobit".to_string(),
        base: "USDT".to_string(),
        quote: "IRT".to_string(),
        market_type: MarketType::Spot,
        active: true,
    });

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

    let ticker_repository: Option<Arc<dyn TickerRepository>> = match env::var("DATABASE_URL") {
        Ok(url) => match sqlx::PgPool::connect(&url).await {
            Ok(pool) => {
                if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
                    tracing::error!(error = %e, "database migration failed");
                    None
                } else {
                    tracing::info!(url = %url, "postgres connected, migrations applied");
                    Some(Arc::new(PostgresTickerRepository::new(pool)))
                }
            }
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

    let app_state = web::Data::new(AppState {
        redis: redis_conn,
        exchange_adapters: Arc::new(RwLock::new(adapters)),
        exchange_registry: Arc::new(Mutex::new(registry)),
        adapter_factories,
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret,
        ticker_repository,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(HashMap::new()),
    });

    restore_tickers(&app_state).await;

    tracing::info!(host = %host, port = %port, "server starting");

    HttpServer::new(move || {
        App::new()
            .configure(init_routes)
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
