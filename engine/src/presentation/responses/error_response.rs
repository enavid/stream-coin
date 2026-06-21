use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use serde::Serialize;
use std::fmt;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    pub fn new(field: &str, message: &str) -> Self {
        FieldError {
            field: field.to_string(),
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    success: bool,
    message: String,
    errors: Vec<FieldError>,
    #[serde(skip)]
    status: StatusCode,
}

impl ApiError {
    pub fn new(message: &str, errors: Vec<FieldError>) -> Self {
        ApiError {
            success: false,
            message: message.to_string(),
            errors,
            status: StatusCode::BAD_REQUEST,
        }
    }

    /// 401 — missing or invalid credentials.
    pub fn unauthorized(message: &str) -> Self {
        ApiError {
            success: false,
            message: message.to_string(),
            errors: vec![],
            status: StatusCode::UNAUTHORIZED,
        }
    }

    /// 403 — authenticated, but missing the required permission.
    pub fn forbidden(message: &str) -> Self {
        ApiError {
            success: false,
            message: message.to_string(),
            errors: vec![],
            status: StatusCode::FORBIDDEN,
        }
    }

    /// 503 — a required dependency (e.g. credential encryption key) is not configured.
    pub fn service_unavailable(message: &str) -> Self {
        ApiError {
            success: false,
            message: message.to_string(),
            errors: vec![],
            status: StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    pub fn to_response(&self) -> HttpResponse {
        HttpResponse::build(self.status).json(self)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msgs: Vec<String> = self
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect();
        write!(f, "{}", msgs.join(", "))
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        self.status
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
    fn field_error_serializes_with_both_field_and_message_keys() {
        let fe = FieldError::new("symbol", "must be BASE/QUOTE format");
        let json = serde_json::to_value(&fe).unwrap();
        assert_eq!(json["field"], "symbol");
        assert_eq!(json["message"], "must be BASE/QUOTE format");
    }

    #[test]
    fn api_error_errors_is_array_of_objects() {
        let err = ApiError::new(
            "Validation failed",
            vec![FieldError::new("symbol", "must be BASE/QUOTE format")],
        );
        let json = serde_json::to_value(&err).unwrap();
        let errors = json["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].is_object(),
            "each error must be an object with field and message keys"
        );
        assert_eq!(errors[0]["field"], "symbol");
    }

    #[test]
    fn api_error_success_is_always_false() {
        let err = ApiError::new("anything", vec![]);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["success"], false);
    }

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
            vec![
                FieldError::new("name", "field required"),
                FieldError::new("value", "invalid value"),
            ],
        );
        assert_eq!(
            err.to_string(),
            "name: field required, value: invalid value"
        );
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

    #[actix_web::test]
    async fn unauthorized_returns_401() {
        let err = ApiError::unauthorized("missing token");
        assert_eq!(err.to_response().status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn forbidden_returns_403() {
        let err = ApiError::forbidden("missing permission");
        assert_eq!(err.to_response().status(), StatusCode::FORBIDDEN);
    }

    #[actix_web::test]
    async fn service_unavailable_returns_503() {
        let err = ApiError::service_unavailable("encryption key not configured");
        assert_eq!(err.to_response().status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn forbidden_message_is_set_correctly() {
        let err = ApiError::forbidden("missing permission: users.manage");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["message"], "missing permission: users.manage");
        assert_eq!(json["success"], false);
    }
}
