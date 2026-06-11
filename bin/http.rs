use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer};
use dotenv::dotenv;
use serde_json::json;
use tokio::sync::Mutex;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use stream_coin::infrastructure::brokers::kafka_producer::establish_kafka_producer;
use stream_coin::infrastructure::cache::redis;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::AppState;
use stream_coin::presentation::swagger::ApiDoc;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (host, port) = (
        env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        env::var("PORT").unwrap_or_else(|_| "8080".to_string()),
    );

    let kafka = match env::var("KAFKA_BROKER") {
        Ok(broker) => match establish_kafka_producer(&broker) {
            Ok(producer) => {
                log::info!("Kafka connected: {}", broker);
                Some(Arc::new(producer))
            }
            Err(e) => {
                log::warn!("Kafka unavailable: {}", e);
                None
            }
        },
        Err(_) => {
            log::warn!("KAFKA_BROKER not set, running without Kafka");
            None
        }
    };

    let redis = match env::var("REDIS_URL") {
        Ok(url) => match redis::establish_redis_connection(&url).await {
            Ok(conn) => {
                log::info!("Redis connected: {}", url);
                Some(conn)
            }
            Err(e) => {
                log::warn!("Redis unavailable: {}", e);
                None
            }
        },
        Err(_) => {
            log::warn!("REDIS_URL not set, running without Redis");
            None
        }
    };

    let app_state = web::Data::new(AppState {
        kafka,
        redis,
        clients: Arc::new(Mutex::new(HashMap::new())),
    });

    HttpServer::new(move || {
        App::new()
            .configure(init_routes)
            .wrap(Logger::default())
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
