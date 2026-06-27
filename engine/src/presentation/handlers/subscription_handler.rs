use actix_web::{web, HttpRequest, HttpResponse, Responder};

use crate::infrastructure::db::subscription_repository::SubscriptionRepositoryError;
use crate::presentation::dto::subscription::{
    SubscribeRequest, SubscriptionListResponse, SubscriptionResponse, UpdateSubscriptionRequest,
};
use crate::presentation::middlewares::jwt::require_permission;
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

const WRITE: &str = "subscriptions.write";
const READ: &str = "subscriptions.read";

fn to_response(
    rec: crate::infrastructure::db::subscription_repository::SubscriptionRecord,
) -> SubscriptionResponse {
    SubscriptionResponse {
        id: rec.id,
        user_id: rec.user_id,
        strategy_id: rec.strategy_id,
        active: rec.active,
        max_position_size: rec.max_position_size,
        confidence_threshold: rec.confidence_threshold,
        created_at: rec.created_at,
    }
}

/// `POST /v1/subscriptions` — subscribe the authenticated user to a strategy.
/// Returns 409 if the user is already subscribed to that strategy.
pub async fn subscribe(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<SubscribeRequest>,
) -> impl Responder {
    let ctx = match require_permission(&req, WRITE) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let Some(repo) = &state.subscription_repository else {
        return ApiError::service_unavailable("Subscription storage not configured").to_response();
    };

    tracing::info!(
        user_id = ctx.user_id,
        strategy_id = %body.strategy_id,
        "user subscribing to strategy"
    );

    match repo
        .create(
            ctx.user_id,
            &body.strategy_id,
            body.max_position_size,
            body.confidence_threshold,
        )
        .await
    {
        Ok(rec) => {
            tracing::info!(
                user_id = ctx.user_id,
                subscription_id = rec.id,
                strategy_id = %rec.strategy_id,
                "subscription created"
            );
            success_response("Subscribed", to_response(rec))
        }
        Err(SubscriptionRepositoryError::AlreadySubscribed { .. }) => HttpResponse::Conflict()
            .json(serde_json::json!({
                "success": false,
                "message": format!("Already subscribed to strategy '{}'", body.strategy_id),
                "errors": []
            })),
        Err(e) => {
            tracing::error!(
                user_id = ctx.user_id,
                strategy_id = %body.strategy_id,
                error = %e,
                "failed to create subscription"
            );
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

/// `GET /v1/subscriptions` — list all active subscriptions for the authenticated user.
pub async fn list_subscriptions(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    let ctx = match require_permission(&req, READ) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let Some(repo) = &state.subscription_repository else {
        return ApiError::service_unavailable("Subscription storage not configured").to_response();
    };

    match repo.list_for_user(ctx.user_id).await {
        Ok(subs) => success_response(
            "Subscriptions",
            SubscriptionListResponse {
                subscriptions: subs.into_iter().map(to_response).collect(),
            },
        ),
        Err(e) => {
            tracing::error!(
                user_id = ctx.user_id,
                error = %e,
                "failed to list subscriptions"
            );
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

/// `PATCH /v1/subscriptions/{id}` — update overrides (active flag, position size,
/// confidence threshold) for a subscription owned by the authenticated user.
pub async fn update_subscription(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<i64>,
    body: web::Json<UpdateSubscriptionRequest>,
) -> impl Responder {
    let ctx = match require_permission(&req, WRITE) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let id = path.into_inner();
    let Some(repo) = &state.subscription_repository else {
        return ApiError::service_unavailable("Subscription storage not configured").to_response();
    };

    // Ownership guard: load first, reject if it belongs to a different user.
    let existing = match repo.get(id).await {
        Ok(Some(r)) => r,
        Ok(None) => return ApiError::new("Subscription not found", vec![]).to_response(),
        Err(e) => {
            tracing::error!(subscription_id = id, error = %e, "failed to get subscription");
            return ApiError::new(&e.to_string(), vec![]).to_response();
        }
    };
    if existing.user_id != ctx.user_id {
        return ApiError::forbidden("You do not own this subscription").to_response();
    }

    match repo
        .update(
            id,
            body.active,
            body.max_position_size,
            body.confidence_threshold,
        )
        .await
    {
        Ok(rec) => {
            tracing::info!(
                user_id = ctx.user_id,
                subscription_id = id,
                active = rec.active,
                "subscription updated"
            );
            success_response("Subscription updated", to_response(rec))
        }
        Err(SubscriptionRepositoryError::NotFound(_)) => {
            ApiError::new("Subscription not found", vec![]).to_response()
        }
        Err(e) => {
            tracing::error!(subscription_id = id, error = %e, "failed to update subscription");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

/// `DELETE /v1/subscriptions/{id}` — remove a subscription owned by the authenticated user.
pub async fn delete_subscription(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> impl Responder {
    let ctx = match require_permission(&req, WRITE) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let id = path.into_inner();
    let Some(repo) = &state.subscription_repository else {
        return ApiError::service_unavailable("Subscription storage not configured").to_response();
    };

    // Ownership guard
    match repo.get(id).await {
        Ok(Some(r)) if r.user_id != ctx.user_id => {
            return ApiError::forbidden("You do not own this subscription").to_response();
        }
        Ok(None) => return ApiError::new("Subscription not found", vec![]).to_response(),
        Ok(Some(_)) => {}
        Err(e) => {
            tracing::error!(subscription_id = id, error = %e, "failed to get subscription");
            return ApiError::new(&e.to_string(), vec![]).to_response();
        }
    }

    match repo.delete(id).await {
        Ok(()) => {
            tracing::info!(
                user_id = ctx.user_id,
                subscription_id = id,
                "subscription deleted"
            );
            success_response("Unsubscribed", serde_json::json!({"id": id}))
        }
        Err(e) => {
            tracing::error!(subscription_id = id, error = %e, "failed to delete subscription");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use actix_web::{test, web, App, HttpMessage};
    use serde_json::Value;
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::exchange::registry::ExchangeRegistry;
    use crate::infrastructure::db::subscription_repository::{
        FakeSubscriptionRepository, SubscriptionRepository,
    };
    use crate::infrastructure::db::user_repository::{FakeUserRepository, RoleRecord};
    use crate::presentation::middlewares::jwt::AuthContext;
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};

    fn state_with_sub_repo(repo: Arc<FakeSubscriptionRepository>) -> web::Data<AppState> {
        let user_repo = Arc::new(FakeUserRepository::with_roles(vec![RoleRecord {
            name: "trader".to_string(),
            permissions: vec![
                "subscriptions.write".to_string(),
                "subscriptions.read".to_string(),
            ],
        }]));
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
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            historical_sources: Arc::new(HashMap::new()),
            candle_history: AppState::new_candle_history(),
            exchange_repository: None,
            asset_repository: None,
            subscription_repository: Some(repo),
            user_repository: Some(user_repo),
            credential_repository: None,
            credential_cipher: None,
        })
    }

    fn state_without_sub_repo() -> web::Data<AppState> {
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

    /// Builds an `AuthContext` for user 42 with trader permissions — inserted into request
    /// extensions directly, bypassing the JWT middleware (handler unit tests don't test the
    /// middleware layer).
    fn trader_ctx() -> AuthContext {
        AuthContext {
            user_id: 42,
            permissions: ["subscriptions.write", "subscriptions.read"]
                .iter()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
        }
    }

    macro_rules! make_app {
        ($state:expr) => {
            test::init_service(
                App::new().app_data($state).service(
                    web::scope("/v1/subscriptions")
                        .route("", web::post().to(subscribe))
                        .route("", web::get().to(list_subscriptions))
                        .route("/{id}", web::patch().to(update_subscription))
                        .route("/{id}", web::delete().to(delete_subscription)),
                ),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn subscribe_returns_200_with_subscription_record() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::post()
            .uri("/v1/subscriptions")
            .set_json(serde_json::json!({"strategy_id": "spread-1"}))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200, "subscribe must succeed");
        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["data"]["strategy_id"], "spread-1");
        assert_eq!(body["data"]["user_id"], 42);
        assert!(body["data"]["active"].as_bool().unwrap());
    }

    #[actix_web::test]
    async fn subscribe_returns_409_when_already_subscribed() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let body = serde_json::json!({"strategy_id": "spread-1"});
        // First call — must succeed
        let req = test::TestRequest::post()
            .uri("/v1/subscriptions")
            .set_json(&body)
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        test::call_service(&app, req).await;

        // Second call — must conflict
        let req = test::TestRequest::post()
            .uri("/v1/subscriptions")
            .set_json(&body)
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 409, "duplicate subscription must return 409");
    }

    #[actix_web::test]
    async fn subscribe_returns_503_when_no_repository_configured() {
        let state = state_without_sub_repo();
        let app = make_app!(state);

        let req = test::TestRequest::post()
            .uri("/v1/subscriptions")
            .set_json(serde_json::json!({"strategy_id": "spread-1"}))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 503);
    }

    #[actix_web::test]
    async fn subscribe_requires_authentication() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let state = state_with_sub_repo(repo);
        let srv = actix_test::start(move || {
            App::new()
                .app_data(state.clone())
                .configure(crate::presentation::routers::init_routes)
        });

        let resp = srv
            .post("/v1/subscriptions")
            .send_json(&serde_json::json!({"strategy_id": "spread-1"}))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            401,
            "unauthenticated request must be rejected"
        );
    }

    #[actix_web::test]
    async fn list_subscriptions_returns_own_subscriptions() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        repo.create(42, "spread-1", None, None).await.unwrap();
        repo.create(42, "rsi-2", None, None).await.unwrap();
        repo.create(99, "spread-1", None, None).await.unwrap();

        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::get()
            .uri("/v1/subscriptions")
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: Value = test::read_body_json(resp).await;
        let subs = body["data"]["subscriptions"].as_array().unwrap();
        assert_eq!(
            subs.len(),
            2,
            "only user 42's subscriptions must be returned"
        );
        assert!(subs.iter().all(|s| s["user_id"] == 42));
    }

    #[actix_web::test]
    async fn list_subscriptions_returns_empty_for_user_with_no_subscriptions() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::get()
            .uri("/v1/subscriptions")
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["data"]["subscriptions"].as_array().unwrap().len(), 0);
    }

    #[actix_web::test]
    async fn update_subscription_changes_active_flag_and_overrides() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let sub = repo.create(42, "spread-1", None, None).await.unwrap();
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::patch()
            .uri(&format!("/v1/subscriptions/{}", sub.id))
            .set_json(serde_json::json!({
                "active": false,
                "max_position_size": "500",
                "confidence_threshold": 0.9
            }))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: Value = test::read_body_json(resp).await;
        assert!(!body["data"]["active"].as_bool().unwrap());
        assert_eq!(body["data"]["confidence_threshold"], 0.9);
    }

    #[actix_web::test]
    async fn update_subscription_owned_by_other_user_returns_403() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let sub = repo.create(99, "spread-1", None, None).await.unwrap();
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::patch()
            .uri(&format!("/v1/subscriptions/{}", sub.id))
            .set_json(serde_json::json!({"active": false}))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            403,
            "must not be able to update another user's subscription"
        );
    }

    #[actix_web::test]
    async fn update_nonexistent_subscription_returns_400() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::patch()
            .uri("/v1/subscriptions/999")
            .set_json(serde_json::json!({"active": true}))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn delete_subscription_returns_200() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let sub = repo.create(42, "spread-1", None, None).await.unwrap();
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::delete()
            .uri(&format!("/v1/subscriptions/{}", sub.id))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn delete_nonexistent_subscription_returns_400() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::delete()
            .uri("/v1/subscriptions/999")
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn delete_subscription_owned_by_other_user_returns_403() {
        let repo = Arc::new(FakeSubscriptionRepository::new());
        let sub = repo.create(99, "spread-1", None, None).await.unwrap();
        let state = state_with_sub_repo(repo);
        let app = make_app!(state);

        let req = test::TestRequest::delete()
            .uri(&format!("/v1/subscriptions/{}", sub.id))
            .to_request();
        req.extensions_mut().insert(trader_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 403);
    }
}
