use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use serde::Serialize;
use std::fmt;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    success: bool,
    message: String,
    errors: Vec<String>,
}

impl ApiError {
    pub fn new(message: &str, errors: Vec<String>) -> Self {
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

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::ResponseError;

    use super::*;

    #[test]
    fn api_error_success_field_is_false() {
        let err = ApiError::new("test", vec![]);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["success"], false);
    }

    #[test]
    fn api_error_message_is_set_correctly() {
        let err = ApiError::new("something went wrong", vec![]);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["message"], "something went wrong");
    }

    #[test]
    fn api_error_display_joins_errors_with_comma() {
        let err = ApiError::new(
            "msg",
            vec!["field required".to_string(), "invalid value".to_string()],
        );
        assert_eq!(err.to_string(), "field required, invalid value");
    }

    #[test]
    fn api_error_status_code_is_400() {
        let err = ApiError::new("bad", vec![]);
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn api_error_to_response_returns_400() {
        let err = ApiError::new("bad request", vec![]);
        let resp = err.to_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
