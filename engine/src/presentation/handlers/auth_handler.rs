use actix_web::{web, Responder};

use crate::infrastructure::crypto::password::verify_password;
use crate::presentation::dto::auth::{LoginRequest, RefreshRequest, TokenResponse};
use crate::presentation::middlewares::jwt::{
    mint_token_with_permissions, validate_jwt_allow_expired,
};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

const TOKEN_EXPIRES_SECS: i64 = 86400;

/// `POST /v1/auth/token` — exchange username + password for a JWT carrying the
/// user's flattened permission set. Exempt from JWT middleware.
pub async fn login(state: web::Data<AppState>, body: web::Json<LoginRequest>) -> impl Responder {
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured on this server")
            .to_response();
    };
    let Some(secret) = &state.jwt_secret else {
        return ApiError::service_unavailable("JWT secret not configured on this server")
            .to_response();
    };

    let user = match repo.find_by_username(&body.username).await {
        Ok(Some(u)) => u,
        Ok(None) => return ApiError::unauthorized("Invalid credentials").to_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to look up user during login");
            return ApiError::service_unavailable("User lookup failed").to_response();
        }
    };

    if !verify_password(&body.password, &user.password_hash) {
        return ApiError::unauthorized("Invalid credentials").to_response();
    }

    let permissions = match repo.permissions_for_user(user.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to load permissions during login");
            return ApiError::service_unavailable("Permission lookup failed").to_response();
        }
    };

    let token = mint_token_with_permissions(
        &user.id.to_string(),
        secret.as_str(),
        TOKEN_EXPIRES_SECS,
        &permissions,
    );
    success_response(
        "Login successful",
        TokenResponse {
            token,
            expires_in: TOKEN_EXPIRES_SECS as u64,
        },
    )
}

/// `POST /v1/auth/refresh` — exchange a valid (or recently-expired) JWT for a fresh one.
/// The new token carries the same permissions as the old one — a permission change
/// only takes effect on the next full login, not on refresh. Exempt from JWT middleware.
pub async fn refresh(
    state: web::Data<AppState>,
    body: web::Json<RefreshRequest>,
) -> impl Responder {
    let Some(secret) = &state.jwt_secret else {
        return ApiError::service_unavailable("JWT secret not configured on this server")
            .to_response();
    };

    let claims = match validate_jwt_allow_expired(&body.token, secret.as_str()) {
        Ok(c) => c,
        Err(e) => return ApiError::unauthorized(&format!("Invalid token: {e}")).to_response(),
    };

    let token = mint_token_with_permissions(
        &claims.sub,
        secret.as_str(),
        TOKEN_EXPIRES_SECS,
        &claims.permissions,
    );
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
    use serde_json::json;
    use tokio::sync::{Mutex, RwLock};

    use crate::exchange::registry::ExchangeRegistry;
    use crate::infrastructure::crypto::password::hash_password;
    use crate::infrastructure::db::user_repository::{
        FakeUserRepository, RoleRecord, UserRepository,
    };
    use crate::presentation::middlewares::jwt::mint_token;
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};

    async fn repo_with_user(username: &str, password: &str) -> Arc<FakeUserRepository> {
        let repo = Arc::new(FakeUserRepository::with_roles(vec![RoleRecord {
            name: "admin".to_string(),
            permissions: vec!["users.manage".to_string()],
        }]));
        let user = repo
            .create_user(username, &hash_password(password))
            .await
            .unwrap();
        repo.assign_roles(user.id, &["admin".to_string()])
            .await
            .unwrap();
        repo
    }

    fn state_with(
        user_repository: Option<Arc<FakeUserRepository>>,
        jwt_secret: Option<&str>,
    ) -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: jwt_secret.map(|s| Arc::new(s.to_string())),
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            exchange_repository: None,
            user_repository: user_repository.map(|r| r as Arc<_>),
            credential_repository: None,
            credential_cipher: None,
        })
    }

    #[actix_web::test]
    async fn login_with_valid_credentials_returns_token() {
        let repo = repo_with_user("admin", "secret").await;
        let app = test::init_service(
            App::new()
                .app_data(state_with(Some(repo), Some("test-secret")))
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
    async fn login_token_carries_user_permissions() {
        let repo = repo_with_user("admin", "secret").await;
        let secret = "test-secret";
        let app = test::init_service(
            App::new()
                .app_data(state_with(Some(repo), Some(secret)))
                .route("/auth/token", web::post().to(login)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/token")
            .set_json(json!({"username": "admin", "password": "secret"}))
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let token = body["data"]["token"].as_str().unwrap();

        let claims = crate::presentation::middlewares::jwt::validate_jwt(token, secret).unwrap();
        assert_eq!(claims.permissions, vec!["users.manage".to_string()]);
    }

    #[actix_web::test]
    async fn login_with_wrong_password_returns_401() {
        let repo = repo_with_user("admin", "correct").await;
        let app = test::init_service(
            App::new()
                .app_data(state_with(Some(repo), Some("test-secret")))
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
        let repo = repo_with_user("admin", "secret").await;
        let app = test::init_service(
            App::new()
                .app_data(state_with(Some(repo), Some("test-secret")))
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
    async fn login_when_no_user_repository_returns_503() {
        let app = test::init_service(
            App::new()
                .app_data(state_with(None, Some("test-secret")))
                .route("/auth/token", web::post().to(login)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/token")
            .set_json(json!({"username": "admin", "password": "any"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 503, "no user repository must return 503");
    }

    #[actix_web::test]
    async fn refresh_with_valid_token_returns_new_token() {
        let repo = repo_with_user("admin", "secret").await;
        let state = state_with(Some(repo), Some("test-secret"));
        let existing_token = mint_token("1", "test-secret", 3600);

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
        let state = state_with(None, Some("test-secret"));
        let expired_token = mint_token("1", "test-secret", -1);

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
        let state = state_with(None, Some("test-secret"));
        let bad_token = mint_token("1", "wrong-secret", 3600);

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
        let state = state_with(None, None);

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
