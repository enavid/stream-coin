use std::fmt;
use serde::Serialize;
use utoipa::ToSchema;
use actix_web::{HttpResponse, ResponseError, http::StatusCode};


#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    success: bool,
    message: String,
    errors: Vec<String>,
}

impl ApiError {
    pub fn new(message: &str, errors:Vec<String>) -> Self {
        ApiError {
            success: false,
            message: message.to_string(),
            errors,
        }
    }

    pub fn to_response(&self) -> HttpResponse {
        HttpResponse::BadRequest().json(self)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.errors.join(", "))
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }

    fn error_response(&self) -> HttpResponse {
        self.to_response()
    }
}

pub fn error_response(message: &str, errors: Vec<String>) -> HttpResponse {
    ApiError::new(message, errors).to_response()
}
