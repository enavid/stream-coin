use std::future::Future;
use std::pin::Pin;

use actix_web::dev::Payload;
use actix_web::{web, FromRequest, HttpRequest};
use serde::de::DeserializeOwned;

use crate::presentation::responses::{ApiError, FieldError};

/// JSON extractor that uses `serde_path_to_error` to produce field-qualified
/// `FieldError`s on validation failures instead of opaque serde messages.
pub struct ValidatedJson<T>(pub T);

impl<T> std::ops::Deref for ValidatedJson<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: DeserializeOwned + 'static> FromRequest for ValidatedJson<T> {
    type Error = actix_web::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let bytes_fut = web::Bytes::from_request(req, payload);
        Box::pin(async move {
            let bytes = bytes_fut
                .await
                .map_err(|e| actix_web::error::ErrorBadRequest(e.to_string()))?;

            let de = &mut serde_json::Deserializer::from_slice(&bytes);
            serde_path_to_error::deserialize(de)
                .map(ValidatedJson)
                .map_err(|e| {
                    let path = e.path().to_string();
                    let raw_msg = e.inner().to_string();
                    let message = raw_msg
                        .split(" at line ")
                        .next()
                        .unwrap_or(&raw_msg)
                        .to_string();
                    // serde reports "missing field" errors at the struct level (path = ".").
                    // Extract the field name from the error message in that case.
                    let field = if path == "." {
                        raw_msg
                            .strip_prefix("missing field `")
                            .and_then(|s| s.split('`').next())
                            .unwrap_or(&path)
                            .to_string()
                    } else {
                        path
                    };
                    ApiError::new("Validation failed", vec![FieldError::new(&field, &message)])
                        .into()
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::{test, web, App, HttpResponse};
    use serde::Deserialize;

    use super::*;
    use crate::price::entity::TradingPair;

    #[derive(Deserialize)]
    struct Payload {
        #[allow(dead_code)]
        symbol: TradingPair,
    }

    async fn dummy(body: ValidatedJson<Payload>) -> HttpResponse {
        let _ = &body.symbol;
        HttpResponse::Ok().finish()
    }

    #[actix_web::test]
    async fn valid_payload_calls_handler() {
        let app = test::init_service(App::new().route("/", web::post().to(dummy))).await;
        let req = test::TestRequest::post()
            .uri("/")
            .insert_header(("Content-Type", "application/json"))
            .set_payload(r#"{"symbol":"USDT/IRT"}"#)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn missing_field_returns_400_with_field_name() {
        let app = test::init_service(App::new().route("/", web::post().to(dummy))).await;
        let req = test::TestRequest::post()
            .uri("/")
            .insert_header(("Content-Type", "application/json"))
            .set_payload(r#"{}"#)
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["errors"][0]["field"], "symbol");
    }

    #[actix_web::test]
    async fn custom_validation_error_returns_400_with_field_name() {
        let app = test::init_service(App::new().route("/", web::post().to(dummy))).await;
        let req = test::TestRequest::post()
            .uri("/")
            .insert_header(("Content-Type", "application/json"))
            .set_payload(r#"{"symbol":"USDTIRT"}"#)
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["errors"][0]["field"], "symbol");
    }

    #[actix_web::test]
    async fn malformed_json_returns_400() {
        let app = test::init_service(App::new().route("/", web::post().to(dummy))).await;
        let req = test::TestRequest::post()
            .uri("/")
            .insert_header(("Content-Type", "application/json"))
            .set_payload("{not json}")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
