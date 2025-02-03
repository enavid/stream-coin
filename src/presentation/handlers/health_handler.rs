use serde_json::json;
use actix_web::Responder;
use actix_web:: HttpResponse;

pub async fn health() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "response":"server is up and running!",
    }))
}