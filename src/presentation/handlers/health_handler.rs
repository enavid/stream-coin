use serde_json::json;
use actix_web::Responder;
use actix_web:: HttpResponse;

pub async fn health() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "success": true,
        "code": 200,
        "message": "server is up and running!"
    }))
}