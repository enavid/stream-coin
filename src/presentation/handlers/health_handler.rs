use actix_web::{web, Responder};
use serde::Serialize;

use crate::presentation::responses::success_response;
use crate::presentation::shared::app_state::AppState;

#[derive(Serialize)]
struct HealthStatus {
    status: &'static str,
    redis: &'static str,
}

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
            status: "up",
            redis: redis_status,
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

    fn app_state_disconnected() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            clients: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[actix_web::test]
    async fn health_returns_200_when_services_disconnected() {
        let state = app_state_disconnected();
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/health", web::get().to(health)),
        )
        .await;
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn health_reports_redis_disconnected() {
        let state = app_state_disconnected();
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/health", web::get().to(health)),
        )
        .await;
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;
        let body: serde_json::Value = test::read_body_json(resp).await;

        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["status"], "up");
        assert_eq!(body["data"]["redis"], "disconnected");
    }
}
