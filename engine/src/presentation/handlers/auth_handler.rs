use actix_web::{web, HttpResponse, Responder};
use serde_json::json;

use crate::presentation::dto::auth::{LoginRequest, RefreshRequest, TokenResponse};
use crate::presentation::middlewares::jwt::{mint_token, validate_jwt_allow_expired};
use crate::presentation::responses::success_response;
use crate::presentation::shared::app_state::AppState;

const TOKEN_EXPIRES_SECS: i64 = 86400;

fn unconfigured(what: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(json!({
        "success": false,
        "message": format!("{what} not configured on this server"),
        "errors": []
    }))
}

fn unauthorized(msg: &str) -> HttpResponse {
    HttpResponse::Unauthorized().json(json!({
        "success": false,
        "message": msg,
        "errors": []
    }))
}

/// `POST /v1/auth/token` — exchange username + password for a JWT.
/// Exempt from JWT middleware; credentials are validated here.
pub async fn login(state: web::Data<AppState>, body: web::Json<LoginRequest>) -> impl Responder {
    let (stored_user, stored_pass) = match &state.admin_credentials {
        None => return unconfigured("Admin account"),
        Some(c) => (c.0.as_str(), c.1.as_str()),
    };

    if body.username != stored_user || body.password != stored_pass {
        return unauthorized("Invalid credentials");
    }

    let secret = match &state.jwt_secret {
        None => return unconfigured("JWT secret"),
        Some(s) => s.as_str(),
    };

    let token = mint_token(&body.username, secret, TOKEN_EXPIRES_SECS);
    success_response(
        "Login successful",
        TokenResponse {
            token,
            expires_in: TOKEN_EXPIRES_SECS as u64,
        },
    )
}

/// `POST /v1/auth/refresh` — exchange a valid (or recently-expired) JWT for a fresh one.
/// Exempt from JWT middleware; token is validated here with expiry check disabled.
pub async fn refresh(
    state: web::Data<AppState>,
    body: web::Json<RefreshRequest>,
) -> impl Responder {
    let secret = match &state.jwt_secret {
        None => return unconfigured("JWT secret"),
        Some(s) => s.as_str(),
    };

    let claims = match validate_jwt_allow_expired(&body.token, secret) {
        Ok(c) => c,
        Err(e) => return unauthorized(&format!("Invalid token: {e}")),
    };

    let token = mint_token(&claims.sub, secret, TOKEN_EXPIRES_SECS);
    success_response(
        "Token refreshed",
        TokenResponse {
            token,
            expires_in: TOKEN_EXPIRES_SECS as u64,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use actix_web::{test, App};
    use tokio::sync::{Mutex, RwLock};

    use crate::exchange::registry::ExchangeRegistry;
    use crate::presentation::middlewares::jwt::mint_token;
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};

    fn state_with_auth(username: &str, password: &str) -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: Some(Arc::new("test-secret".to_string())),
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            admin_credentials: Some(Arc::new((username.to_string(), password.to_string()))),
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
        })
    }

    fn state_no_admin() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: Some(Arc::new("test-secret".to_string())),
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            admin_credentials: None,
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
        })
    }

    #[actix_web::test]
    async fn login_with_valid_credentials_returns_token() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_auth("admin", "secret"))
                .route("/auth/token", web::post().to(login)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/token")
            .set_json(json!({"username": "admin", "password": "secret"}))
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;

        assert_eq!(body["success"], true);
        assert!(
            !body["data"]["token"].as_str().unwrap_or("").is_empty(),
            "token must be non-empty"
        );
        assert_eq!(body["data"]["expires_in"], TOKEN_EXPIRES_SECS as u64);
    }

    #[actix_web::test]
    async fn login_with_wrong_password_returns_401() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_auth("admin", "correct"))
                .route("/auth/token", web::post().to(login)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/token")
            .set_json(json!({"username": "admin", "password": "wrong"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401, "wrong password must return 401");
    }

    #[actix_web::test]
    async fn login_with_wrong_username_returns_401() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_auth("admin", "secret"))
                .route("/auth/token", web::post().to(login)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/token")
            .set_json(json!({"username": "other", "password": "secret"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401, "wrong username must return 401");
    }

    #[actix_web::test]
    async fn login_when_no_admin_configured_returns_503() {
        let app = test::init_service(
            App::new()
                .app_data(state_no_admin())
                .route("/auth/token", web::post().to(login)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/token")
            .set_json(json!({"username": "admin", "password": "any"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 503, "no admin configured must return 503");
    }

    #[actix_web::test]
    async fn refresh_with_valid_token_returns_new_token() {
        let state = state_with_auth("admin", "secret");
        let existing_token = mint_token("admin", "test-secret", 3600);

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/auth/refresh", web::post().to(refresh)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/refresh")
            .set_json(json!({"token": existing_token}))
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;

        assert_eq!(body["success"], true);
        assert!(
            !body["data"]["token"].as_str().unwrap_or("").is_empty(),
            "refreshed token must be non-empty"
        );
    }

    #[actix_web::test]
    async fn refresh_with_expired_token_returns_new_token() {
        let state = state_with_auth("admin", "secret");
        let expired_token = mint_token("admin", "test-secret", -1);

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/auth/refresh", web::post().to(refresh)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/refresh")
            .set_json(json!({"token": expired_token}))
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;

        assert_eq!(
            body["success"], true,
            "expired-but-valid token must be refreshable"
        );
    }

    #[actix_web::test]
    async fn refresh_with_invalid_signature_returns_401() {
        let state = state_with_auth("admin", "secret");
        let bad_token = mint_token("admin", "wrong-secret", 3600);

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/auth/refresh", web::post().to(refresh)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/refresh")
            .set_json(json!({"token": bad_token}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401, "wrong signature must return 401");
    }

    #[actix_web::test]
    async fn refresh_when_no_jwt_secret_configured_returns_503() {
        let state = web::Data::new(AppState {
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
            admin_credentials: None,
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
        });

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/auth/refresh", web::post().to(refresh)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/refresh")
            .set_json(json!({"token": "anything"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 503, "no JWT secret must return 503");
    }
}
