use crate::presentation::responses::ApiError;
use actix_web::error::JsonPayloadError;
use actix_web::web::JsonConfig;

pub fn json_error_handler_config() -> JsonConfig {
    JsonConfig::default().error_handler(|err, _req| match err {
        JsonPayloadError::ContentType => {
            ApiError::new("Invalid Content-Type. Expected application/json", vec![]).into()
        }
        JsonPayloadError::Deserialize(e) => {
            ApiError::new("Invalid request body", vec![e.to_string()]).into()
        }
        JsonPayloadError::Payload(e) => {
            let msg = match e {
                actix_web::error::PayloadError::Overflow => "Payload too large",
                _ => "Payload error",
            };
            ApiError::new(msg, vec![]).into()
        }
        _ => ApiError::new("Failed to parse JSON", vec![]).into(),
    })
}

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::{test, web, App, HttpResponse};
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct TestBody {
        #[allow(dead_code)]
        name: String,
    }

    async fn dummy_handler(_: web::Json<TestBody>) -> HttpResponse {
        HttpResponse::Ok().finish()
    }

    #[actix_web::test]
    async fn wrong_content_type_returns_400() {
        let app = test::init_service(
            App::new()
                .app_data(json_error_handler_config())
                .route("/", web::post().to(dummy_handler)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/")
            .insert_header(("Content-Type", "text/plain"))
            .set_payload("{\"name\":\"test\"}")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn invalid_json_returns_400_with_message() {
        let app = test::init_service(
            App::new()
                .app_data(json_error_handler_config())
                .route("/", web::post().to(dummy_handler)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/")
            .insert_header(("Content-Type", "application/json"))
            .set_payload("{not valid json}")
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["message"], "Invalid request body");
    }
}
