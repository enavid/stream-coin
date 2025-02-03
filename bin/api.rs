use std::env;
use dotenv::dotenv;
use actix_web_validator::JsonConfig;
use actix_web::{error, HttpResponse};
use stream_coin::presentation::server::http::start_server;
use stream_coin::config::actix_web_validator::validation_errors_to_json;

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

    // let exchange_service = web::Data::new(ExchangeService::new());

    start_server(host, port, json_config).await
}
