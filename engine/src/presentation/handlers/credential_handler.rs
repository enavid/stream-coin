use actix_web::{web, HttpRequest, Responder};

use crate::presentation::dto::credential::{CredentialListResponse, CredentialSummaryResponse};
use crate::presentation::middlewares::jwt::require_permission;
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

const PERMISSION: &str = "exchange_credentials.write";

/// `POST /v1/exchanges/{name}/credentials` — stores the caller's own exchange connection
/// info (free-form JSON: api_key, secret, passphrase, ... — shape varies per exchange),
/// encrypted at rest. Always scoped to the authenticated user; never a path user id.
pub async fn set_own_credentials(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    let ctx = match require_permission(&req, PERMISSION) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let name = path.into_inner();

    {
        let registry = state.exchange_registry.lock().await;
        if registry.find_ws_url(&name).is_none() {
            return ApiError::new(&format!("Unknown exchange '{name}'"), vec![]).to_response();
        }
    }

    let Some(cipher) = &state.credential_cipher else {
        return ApiError::service_unavailable(
            "Credential encryption is not configured on this server",
        )
        .to_response();
    };
    let Some(repo) = &state.credential_repository else {
        return ApiError::service_unavailable("Credential storage not configured").to_response();
    };

    let plaintext = serde_json::to_vec(&*body).expect("serializing a JSON value cannot fail");
    let envelope = cipher.encrypt(&plaintext);

    if let Err(e) = repo.upsert(ctx.user_id, &name, envelope).await {
        return ApiError::new(&e.to_string(), vec![]).to_response();
    }

    success_response("Credentials saved", serde_json::json!({"exchange": name}))
}

/// `GET /v1/exchanges/credentials` — lists the caller's own configured exchanges.
/// Never decrypts or returns the secret — name + `created_at` only.
pub async fn list_own_credentials(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    let ctx = match require_permission(&req, PERMISSION) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let Some(repo) = &state.credential_repository else {
        return ApiError::service_unavailable("Credential storage not configured").to_response();
    };

    let summaries = match repo.list_for_user(ctx.user_id).await {
        Ok(s) => s,
        Err(e) => return ApiError::new(&e.to_string(), vec![]).to_response(),
    };

    success_response(
        "Credentials",
        CredentialListResponse {
            credentials: summaries
                .into_iter()
                .map(|s| CredentialSummaryResponse {
                    exchange: s.exchange_name,
                    created_at: s.created_at,
                })
                .collect(),
        },
    )
}

/// `DELETE /v1/exchanges/{name}/credentials` — removes the caller's own credential.
pub async fn delete_own_credentials(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> impl Responder {
    let ctx = match require_permission(&req, PERMISSION) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let Some(repo) = &state.credential_repository else {
        return ApiError::service_unavailable("Credential storage not configured").to_response();
    };

    let name = path.into_inner();
    if let Err(e) = repo.delete(ctx.user_id, &name).await {
        return ApiError::new(&e.to_string(), vec![]).to_response();
    }

    success_response("Credentials removed", serde_json::json!({"exchange": name}))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use actix_web::{test, App, HttpMessage};
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::exchange::registry::{ExchangeRecord, ExchangeRegistry};
    use crate::infrastructure::crypto::credential_cipher::CredentialCipher;
    use crate::infrastructure::db::credential_repository::{
        CredentialRepository, FakeCredentialRepository,
    };
    use crate::presentation::middlewares::jwt::AuthContext;
    use crate::presentation::shared::app_state::AdapterFactory;

    fn registry_with_tabdeal() -> ExchangeRegistry {
        let mut r = ExchangeRegistry::new();
        r.add_exchange(ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://tabdeal.example.com".to_string(),
            enabled: true,
        });
        r
    }

    fn state_with(
        credential_repository: Option<Arc<FakeCredentialRepository>>,
        credential_cipher: Option<Arc<CredentialCipher>>,
    ) -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(registry_with_tabdeal())),
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
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            historical_sources: Arc::new(HashMap::new()),
            candle_history: AppState::new_candle_history(),
            exchange_repository: None,
            asset_repository: None,
            subscription_repository: None,
            user_repository: None,
            credential_repository: credential_repository.map(|r| r as Arc<_>),
            credential_cipher,
        })
    }

    fn ctx(user_id: i32) -> AuthContext {
        AuthContext {
            user_id,
            permissions: HashSet::from(["exchange_credentials.write".to_string()]),
        }
    }

    fn full_state() -> (web::Data<AppState>, Arc<FakeCredentialRepository>) {
        let repo = Arc::new(FakeCredentialRepository::new(vec!["tabdeal".to_string()]));
        let cipher = Arc::new(CredentialCipher::new([3u8; 32]));
        (state_with(Some(repo.clone()), Some(cipher)), repo)
    }

    #[actix_web::test]
    async fn set_own_credentials_without_permission_returns_401() {
        let (state, _repo) = full_state();
        let app = test::init_service(App::new().app_data(state).route(
            "/exchanges/{name}/credentials",
            web::post().to(set_own_credentials),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/exchanges/tabdeal/credentials")
            .set_json(serde_json::json!({"api_key": "abc"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn set_own_credentials_for_unknown_exchange_returns_400() {
        let (state, _repo) = full_state();
        let app = test::init_service(App::new().app_data(state).route(
            "/exchanges/{name}/credentials",
            web::post().to(set_own_credentials),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/exchanges/nobitex/credentials")
            .set_json(serde_json::json!({"api_key": "abc"}))
            .to_request();
        req.extensions_mut().insert(ctx(1));

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn set_own_credentials_without_cipher_returns_503() {
        let repo = Arc::new(FakeCredentialRepository::new(vec!["tabdeal".to_string()]));
        let state = state_with(Some(repo), None);
        let app = test::init_service(App::new().app_data(state).route(
            "/exchanges/{name}/credentials",
            web::post().to(set_own_credentials),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/exchanges/tabdeal/credentials")
            .set_json(serde_json::json!({"api_key": "abc"}))
            .to_request();
        req.extensions_mut().insert(ctx(1));

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 503);
    }

    #[actix_web::test]
    async fn set_then_list_own_credentials_never_exposes_secret() {
        let (state, _repo) = full_state();
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route(
                    "/exchanges/{name}/credentials",
                    web::post().to(set_own_credentials),
                )
                .route(
                    "/exchanges/credentials",
                    web::get().to(list_own_credentials),
                ),
        )
        .await;

        let set_req = test::TestRequest::post()
            .uri("/exchanges/tabdeal/credentials")
            .set_json(serde_json::json!({"api_key": "super-secret-value"}))
            .to_request();
        set_req.extensions_mut().insert(ctx(1));
        let set_resp = test::call_service(&app, set_req).await;
        assert_eq!(set_resp.status(), 200);

        let list_req = test::TestRequest::get()
            .uri("/exchanges/credentials")
            .to_request();
        list_req.extensions_mut().insert(ctx(1));
        let body: serde_json::Value = test::call_and_read_body_json(&app, list_req).await;

        let creds = body["data"]["credentials"].as_array().unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0]["exchange"], "tabdeal");
        let serialized = body.to_string();
        assert!(
            !serialized.contains("super-secret-value"),
            "response must never contain the plaintext secret"
        );
    }

    #[actix_web::test]
    async fn list_own_credentials_only_returns_callers_own() {
        let (state, repo) = full_state();
        repo.upsert(
            2,
            "tabdeal",
            CredentialCipher::new([3u8; 32]).encrypt(b"{}"),
        )
        .await
        .unwrap();

        let app = test::init_service(App::new().app_data(state).route(
            "/exchanges/credentials",
            web::get().to(list_own_credentials),
        ))
        .await;

        let req = test::TestRequest::get()
            .uri("/exchanges/credentials")
            .to_request();
        req.extensions_mut().insert(ctx(1));
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert!(body["data"]["credentials"].as_array().unwrap().is_empty());
    }

    #[actix_web::test]
    async fn delete_own_credentials_removes_it() {
        let (state, repo) = full_state();
        repo.upsert(
            1,
            "tabdeal",
            CredentialCipher::new([3u8; 32]).encrypt(b"{}"),
        )
        .await
        .unwrap();

        let app = test::init_service(App::new().app_data(state).route(
            "/exchanges/{name}/credentials",
            web::delete().to(delete_own_credentials),
        ))
        .await;

        let req = test::TestRequest::delete()
            .uri("/exchanges/tabdeal/credentials")
            .to_request();
        req.extensions_mut().insert(ctx(1));
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        assert!(repo.get(1, "tabdeal").await.unwrap().is_none());
    }
}
