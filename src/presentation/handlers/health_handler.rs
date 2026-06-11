use actix_web::Responder;
use serde::Serialize;

use crate::presentation::responses::success_response;

#[derive(Serialize)]
struct HealthStatus {
    status: &'static str,
}

pub async fn health() -> impl Responder {
    success_response("ok", HealthStatus { status: "up" })
}

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::{web, App};

    use super::health;

    #[actix_web::test]
    async fn health_returns_200() {
        let app = test::init_service(App::new().route("/health", web::get().to(health))).await;
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn health_body_has_success_true_and_status_up() {
        let app = test::init_service(App::new().route("/health", web::get().to(health))).await;
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;
        let body: serde_json::Value = test::read_body_json(resp).await;

        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["status"], "up");
    }
}
