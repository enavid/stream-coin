use std::env;
use std::sync::Arc;
use dotenv::dotenv;
use actix_web_validator::JsonConfig;
use actix_web::{error, web, HttpResponse};
use stream_coin::presentation::server::http::start_server;
use stream_coin::config::actix_web_validator::validation_errors_to_json;
use stream_coin::infrastructure::persistence::database::maria_db::{establish_connection, AppState};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    let (host, port) = (
        env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        env::var("PORT").unwrap_or_else(|_| "8080".to_string()),
    );

    let json_config = JsonConfig::default()
        .limit(4096)
        .error_handler(|err, _req| {
            let error_response = validation_errors_to_json(&err);
            error::InternalError::from_response(err, HttpResponse::BadRequest().json(error_response)).into()
        });

    let database_url = std::env::var("MYSQL_URL").expect("DATABASE_URL must be set");
    let db = establish_connection(&database_url).await;

    let app_state = web::Data::new(AppState {
        db: Arc::new(db),
    });

    start_server(host, port, app_state, json_config).await
}
