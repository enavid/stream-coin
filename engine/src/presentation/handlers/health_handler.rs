use actix_web::{web, Responder};

use crate::presentation::dto::health::{Dependencies, HealthStatus};
use crate::presentation::responses::success_response;
use crate::presentation::shared::app_state::AppState;

#[utoipa::path(
    get,
    path = "/v1/check/health",
    tag = "Health",
    responses(
        (status = 200, description = "Service is up", body = HealthStatus)
    )
)]
pub async fn health(state: web::Data<AppState>) -> impl Responder {
    let redis_status = if state.redis.is_some() {
        "connected"
    } else {
        "disconnected"
    };

    tracing::debug!(redis = %redis_status, "health check");

    success_response(
        "ok",
        HealthStatus {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            status: "up",
            dependencies: Dependencies {
                redis: redis_status,
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::{web, App};
    use tokio::sync::Mutex;

    use super::health;
    use crate::presentation::shared::app_state::AppState;

    fn disconnected_state() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
        })
    }

    #[actix_web::test]
    async fn health_returns_200() {
        let app = test::init_service(
            App::new()
                .app_data(disconnected_state())
                .route("/health", web::get().to(health)),
        )
        .await;
        let resp =
            test::call_service(&app, test::TestRequest::get().uri("/health").to_request()).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn health_returns_project_name() {
        let app = test::init_service(
            App::new()
                .app_data(disconnected_state())
                .route("/health", web::get().to(health)),
        )
        .await;
        let body: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/health").to_request(),
        )
        .await;
        assert_eq!(body["data"]["name"], "stream-coin");
    }

    #[actix_web::test]
    async fn health_returns_project_version() {
        let app = test::init_service(
            App::new()
                .app_data(disconnected_state())
                .route("/health", web::get().to(health)),
        )
        .await;
        let body: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/health").to_request(),
        )
        .await;
        assert!(!body["data"]["version"].as_str().unwrap_or("").is_empty());
    }

    #[actix_web::test]
    async fn health_returns_status_up() {
        let app = test::init_service(
            App::new()
                .app_data(disconnected_state())
                .route("/health", web::get().to(health)),
        )
        .await;
        let body: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/health").to_request(),
        )
        .await;
        assert_eq!(body["data"]["status"], "up");
    }

    #[actix_web::test]
    async fn health_reports_redis_disconnected_in_dependencies() {
        let app = test::init_service(
            App::new()
                .app_data(disconnected_state())
                .route("/health", web::get().to(health)),
        )
        .await;
        let body: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/health").to_request(),
        )
        .await;
        assert_eq!(body["data"]["dependencies"]["redis"], "disconnected");
    }
}
