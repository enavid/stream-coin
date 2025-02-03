use serde_json::json;
use actix_web::{web, HttpResponse, Responder};
// use crate::application::services::ExchangeService;
use crate::presentation::dto::exchange_request::ExchangeRequest;


use actix_web_validator::Json;
pub async fn connect_websocket(
    request: Json<ExchangeRequest>,
    // exchange_service: web::Data<ExchangeService>,
) -> impl Responder {
    // let exchange_name = &request.exchange_name;
    // let symbols = &request.symbols;

    HttpResponse::Ok().json(serde_json::json!({
        "fn": "connect_websocket",
    }))


     //match exchange_service.connect(exchange_name, symbols.clone()).await {
    //     Ok(_) => HttpResponse::Ok().json(format!("Connected to {} with symbols {:?}", exchange_name, symbols)),
    //     Err(e) => HttpResponse::BadRequest().json(format!("Error: {}", e)),
    // }
}

pub async fn disconnect_websocket(
    request: web::Json<ExchangeRequest>,
    // exchange_service: web::Data<ExchangeService>,
) -> impl Responder {
    let exchange_name = &request.exchange_name;

    HttpResponse::Ok().json(json!({
        "fn": "disconnect_websocket",
    }))

    // match exchange_service.disconnect(exchange_name).await {
    //     Ok(_) => HttpResponse::Ok().json(format!("Disconnected from {}", exchange_name)),
    //     Err(e) => HttpResponse::BadRequest().json(format!("Error: {}", e)),
    // }
}
