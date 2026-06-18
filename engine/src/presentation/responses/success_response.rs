use actix_web::HttpResponse;
use serde::Serialize;
use utoipa::ToSchema;

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

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::{test, web, App};
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct TestData {
        value: i32,
    }

    #[actix_web::test]
    async fn success_response_returns_200() {
        let app = test::init_service(App::new().route(
            "/",
            web::get().to(|| async { success_response("ok", TestData { value: 1 }) }),
        ))
        .await;
        let req = test::TestRequest::get().uri("/").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn success_response_body_has_correct_fields() {
        let app = test::init_service(App::new().route(
            "/",
            web::get().to(|| async { success_response("done", TestData { value: 42 }) }),
        ))
        .await;
        let req = test::TestRequest::get().uri("/").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], true);
        assert_eq!(body["message"], "done");
        assert_eq!(body["data"]["value"], 42);
    }
}
