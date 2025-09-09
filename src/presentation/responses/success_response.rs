use serde::Serialize;
use utoipa::ToSchema;
use actix_web::HttpResponse;

#[derive(Serialize, ToSchema)]
pub struct ApiSuccess<T: Serialize> {
    pub success: bool,
    pub message: String,
    pub data: T,
}

pub fn success_response<T: Serialize>(message: &str, data: T) -> HttpResponse {
    HttpResponse::Ok().json(ApiSuccess {
        success: true,
        message: message.to_string(),
        data,
    })
}
