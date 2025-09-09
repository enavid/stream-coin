use std::env;
use std::sync::Arc;
use dotenv::dotenv;
use utoipa::OpenApi;
use serde_json::json;
use tokio::sync::Mutex;
use std::collections::HashMap;
use utoipa_swagger_ui::SwaggerUi;
use actix_web::middleware::Logger;
use stream_coin::infrastructure::cache::redis;
use stream_coin::presentation::swagger::ApiDoc;
use stream_coin::presentation::routers::init_routes;
use actix_web::{web, App, HttpResponse, HttpServer};
use stream_coin::presentation::shared::app_state::AppState;
use stream_coin::infrastructure::persistence::database::postgres;
use stream_coin::infrastructure::brokers::kafka_producer::establish_kafka_producer;
use stream_coin::presentation::middlewares::json_error_handler::json_error_handler_config;

use stream_coin::infrastructure::websocket::ws_client_trait::WebSocketClient;


#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    let (host, port) = (
        env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        env::var("PORT").unwrap_or_else(|_| "8080".to_string()),
    );

    // let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    // let db = postgres::establish_db_connection(&database_url).await;

    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = redis::establish_redis_connection(&redis_url).await;

    let kafka_broker = std::env::var("KAFKA_BROKER").expect("KAFKA_BROKER must be set");
    let kafka = establish_kafka_producer(&kafka_broker);

    let app_state = web::Data::new(AppState {
        // db: Arc::new(db.expect("REASON")),
        kafka: Arc::new(kafka.expect("REASON")),
        clients: Arc::new(Mutex::new(HashMap::new())),
        redis: Arc::new(tokio::sync::Mutex::new(redis.expect("REASON"))),
    });

    HttpServer::new(move || {
        App::new()
            .configure(init_routes)
            .wrap(Logger::default())
            .app_data(app_state.clone())
            .app_data(json_error_handler_config())
            .default_service(
                web::route().to(|| async {
                    HttpResponse::NotFound().json(json!({
                        "success": false,
                        "code": 404,
                        "message": "Resource not found"
                    }))
                }),
            )
            .service(SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()))
    })
        .bind(format!("{}:{}", host, port))?
        .run()
        .await
}
