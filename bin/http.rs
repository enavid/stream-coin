use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use actix_web::{web, App, HttpResponse, HttpServer};
use dotenv::dotenv;
use serde_json::json;
use tokio::sync::Mutex;
use tracing_actix_web::TracingLogger;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use stream_coin::exchange::port::ExchangeAdapter;
use stream_coin::exchange::tabdeal::TabdealWsAdapter;
use stream_coin::infrastructure::cache::redis;
use stream_coin::infrastructure::cache::ticker_repository::RedisTickerRepository;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::AppState;
use stream_coin::presentation::swagger::ApiDoc;
use stream_coin::ticker::port::TickerRepository;

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

    let (redis_conn, ticker_repository) = match env::var("REDIS_URL") {
        Ok(url) => match redis::establish_redis_connection(&url).await {
            Ok(conn) => {
                tracing::info!(url = %url, "redis connected");
                let repo: Arc<dyn TickerRepository> =
                    Arc::new(RedisTickerRepository::new(conn.clone()));
                (Some(conn), Some(repo))
            }
            Err(e) => {
                tracing::warn!(error = %e, "redis unavailable, starting without cache");
                (None, None)
            }
        },
        Err(_) => {
            tracing::warn!("REDIS_URL not set, starting without cache");
            (None, None)
        }
    };

    let mut adapters: HashMap<String, Arc<dyn ExchangeAdapter>> = HashMap::new();
    adapters.insert("tabdeal".to_string(), Arc::new(TabdealWsAdapter));

    let app_state = web::Data::new(AppState {
        redis: redis_conn,
        ticker_repository,
        exchange_adapters: Arc::new(adapters),
        clients: Arc::new(Mutex::new(HashMap::new())),
    });

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
