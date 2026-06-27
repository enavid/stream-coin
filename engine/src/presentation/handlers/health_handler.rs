use std::collections::HashMap;

use actix_web::{web, Responder};

use crate::presentation::dto::health::{worst_status, HealthStatus, ServiceStatus};
use crate::presentation::responses::success_response;
use crate::presentation::shared::app_state::AppState;

#[utoipa::path(
    get,
    path = "/v1/check/health",
    tag = "Health",
    responses(
        (status = 200, description = "Service health with per-dependency checks", body = HealthStatus)
    )
)]
/// `GET /v1/check/health` — returns service version and per-dependency status.
/// Always returns 200; callers should inspect `status` and `checks` to determine
/// actual health. `status` is the worst across all checks.
pub async fn health(state: web::Data<AppState>) -> impl Responder {
    let mut checks = HashMap::new();
    checks.insert(
        "redis".to_string(),
        if state.redis.is_some() {
            ServiceStatus::Up
        } else {
            ServiceStatus::Down
        },
    );
    checks.insert(
        "kafka".to_string(),
        if state.publisher.is_some() {
            ServiceStatus::Up
        } else {
            ServiceStatus::Down
        },
    );
    checks.insert(
        "postgres".to_string(),
        if state.ticker_repository.is_some() {
            ServiceStatus::Up
        } else {
            ServiceStatus::Down
        },
    );
    checks.insert(
        "order_manager".to_string(),
        if state.order_manager.is_some() {
            ServiceStatus::Up
        } else {
            ServiceStatus::Down
        },
    );
    let status = worst_status(&checks);

    tracing::debug!(?status, "health check");

    success_response(
        "ok",
        HealthStatus {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            status,
            checks,
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
    use tokio::sync::{Mutex, RwLock};

    use super::health;
    use crate::exchange::registry::ExchangeRegistry;
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};

    fn disconnected_state() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            historical_sources: Arc::new(HashMap::new()),
            candle_history: AppState::new_candle_history(),
            exchange_repository: None,
            asset_repository: None,
            subscription_repository: None,
            user_repository: None,
            credential_repository: None,
            credential_cipher: None,
        })
    }

    async fn call_health(state: web::Data<AppState>) -> serde_json::Value {
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/health", web::get().to(health)),
        )
        .await;
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/health").to_request())
            .await
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
        let body = call_health(disconnected_state()).await;
        assert_eq!(body["data"]["name"], "stream-coin");
    }

    #[actix_web::test]
    async fn health_returns_project_version() {
        let body = call_health(disconnected_state()).await;
        assert!(!body["data"]["version"].as_str().unwrap_or("").is_empty());
    }

    #[actix_web::test]
    async fn health_status_is_down_when_redis_disconnected() {
        let body = call_health(disconnected_state()).await;
        assert_eq!(
            body["data"]["status"], "down",
            "status must reflect redis being unreachable"
        );
    }

    #[actix_web::test]
    async fn health_checks_map_contains_redis_key() {
        let body = call_health(disconnected_state()).await;
        assert!(
            body["data"]["checks"]["redis"] != serde_json::Value::Null,
            "checks must contain a 'redis' key"
        );
    }

    #[actix_web::test]
    async fn health_checks_redis_value_is_down_when_disconnected() {
        let body = call_health(disconnected_state()).await;
        assert_eq!(body["data"]["checks"]["redis"], "down");
    }

    #[actix_web::test]
    async fn health_checks_map_contains_kafka_key() {
        let body = call_health(disconnected_state()).await;
        assert!(
            body["data"]["checks"]["kafka"] != serde_json::Value::Null,
            "checks must contain a 'kafka' key"
        );
    }

    #[actix_web::test]
    async fn health_checks_kafka_is_down_when_publisher_none() {
        let body = call_health(disconnected_state()).await;
        assert_eq!(
            body["data"]["checks"]["kafka"], "down",
            "kafka check must be 'down' when publisher is None"
        );
    }

    #[actix_web::test]
    async fn health_checks_postgres_is_down_when_repository_none() {
        let body = call_health(disconnected_state()).await;
        assert_eq!(
            body["data"]["checks"]["postgres"], "down",
            "postgres check must be 'down' when ticker_repository is None"
        );
    }

    #[actix_web::test]
    async fn health_checks_order_manager_is_down_when_none() {
        let body = call_health(disconnected_state()).await;
        assert_eq!(
            body["data"]["checks"]["order_manager"], "down",
            "order_manager check must be 'down' when None"
        );
    }

    /// Requires a running Redis instance; skipped in CI without one.
    #[actix_web::test]
    #[ignore]
    async fn health_status_is_up_when_redis_connected() {
        // To run: `REDIS_URL=redis://127.0.0.1:6379 cargo test health_status_is_up`
        // Create a real MultiplexedConnection here when integration infra is available.
    }
}
