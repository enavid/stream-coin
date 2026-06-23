use actix_web::{web, HttpRequest, Responder};

use crate::infrastructure::crypto::password::hash_password;
use crate::presentation::dto::user::{
    AssignRolesRequest, CreateRoleRequest, CreateUserRequest, PermissionListResponse,
    RoleListResponse, RoleResponse, UserListResponse, UserResponse,
};
use crate::presentation::middlewares::jwt::require_permission;
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

/// `POST /v1/admin/users` — creates a user and assigns the given roles. Gated by `users.manage`.
pub async fn create_user(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<CreateUserRequest>,
) -> impl Responder {
    if let Err(resp) = require_permission(&req, "users.manage") {
        return resp;
    }
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured").to_response();
    };

    let user = match repo
        .create_user(&body.username, &hash_password(&body.password))
        .await
    {
        Ok(u) => u,
        Err(e) => return ApiError::new(&e.to_string(), vec![]).to_response(),
    };

    if !body.roles.is_empty() {
        if let Err(e) = repo.assign_roles(user.id, &body.roles).await {
            return ApiError::new(&e.to_string(), vec![]).to_response();
        }
    }

    success_response(
        "User created",
        UserResponse {
            id: user.id,
            username: user.username,
            roles: body.roles.clone(),
            created_at: user.created_at,
        },
    )
}

/// `GET /v1/admin/users` — lists every user with their assigned roles. Gated by `users.manage`.
pub async fn list_users(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(resp) = require_permission(&req, "users.manage") {
        return resp;
    }
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured").to_response();
    };

    let users = match repo.list_users().await {
        Ok(u) => u,
        Err(e) => return ApiError::new(&e.to_string(), vec![]).to_response(),
    };

    let mut responses = Vec::with_capacity(users.len());
    for user in users {
        let roles = match repo.roles_for_user(user.id).await {
            Ok(r) => r,
            Err(e) => return ApiError::new(&e.to_string(), vec![]).to_response(),
        };
        responses.push(UserResponse {
            id: user.id,
            username: user.username,
            roles,
            created_at: user.created_at,
        });
    }

    success_response("Users", UserListResponse { users: responses })
}

/// `POST /v1/admin/users/{id}/roles` — replaces a user's role assignment. Gated by `users.manage`.
pub async fn assign_user_roles(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<i32>,
    body: web::Json<AssignRolesRequest>,
) -> impl Responder {
    if let Err(resp) = require_permission(&req, "users.manage") {
        return resp;
    }
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured").to_response();
    };

    let user_id = path.into_inner();
    if let Err(e) = repo.assign_roles(user_id, &body.roles).await {
        return ApiError::new(&e.to_string(), vec![]).to_response();
    }

    success_response(
        "Roles assigned",
        serde_json::json!({"user_id": user_id, "roles": body.roles}),
    )
}

/// `GET /v1/admin/roles` — lists every role with its permissions. Gated by `users.manage`.
pub async fn list_roles(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(resp) = require_permission(&req, "users.manage") {
        return resp;
    }
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured").to_response();
    };

    let roles = match repo.list_roles().await {
        Ok(r) => r,
        Err(e) => return ApiError::new(&e.to_string(), vec![]).to_response(),
    };

    success_response(
        "Roles",
        RoleListResponse {
            roles: roles
                .into_iter()
                .map(|r| RoleResponse {
                    name: r.name,
                    permissions: r.permissions,
                })
                .collect(),
        },
    )
}

/// `POST /v1/admin/roles` — creates a new role with the given permissions. Gated by `roles.manage`.
pub async fn create_role(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<CreateRoleRequest>,
) -> impl Responder {
    if let Err(resp) = require_permission(&req, "roles.manage") {
        return resp;
    }
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured").to_response();
    };

    if let Err(e) = repo.create_role(&body.name, &body.permissions).await {
        return ApiError::new(&e.to_string(), vec![]).to_response();
    }

    success_response(
        "Role created",
        RoleResponse {
            name: body.name.clone(),
            permissions: body.permissions.clone(),
        },
    )
}

/// `GET /v1/admin/permissions` — lists the fixed permission catalog. Gated by `users.manage`.
pub async fn list_permissions(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(resp) = require_permission(&req, "users.manage") {
        return resp;
    }
    let Some(repo) = &state.user_repository else {
        return ApiError::service_unavailable("User storage not configured").to_response();
    };

    let permissions = match repo.list_permissions().await {
        Ok(p) => p,
        Err(e) => return ApiError::new(&e.to_string(), vec![]).to_response(),
    };

    success_response("Permissions", PermissionListResponse { permissions })
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use actix_web::{test, App, HttpMessage};
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::exchange::registry::ExchangeRegistry;
    use crate::infrastructure::db::user_repository::{FakeUserRepository, RoleRecord};
    use crate::presentation::middlewares::jwt::AuthContext;
    use crate::presentation::shared::app_state::AdapterFactory;

    fn seeded_repo() -> Arc<FakeUserRepository> {
        Arc::new(FakeUserRepository::with_roles(vec![
            RoleRecord {
                name: "admin".to_string(),
                permissions: vec!["users.manage".to_string(), "roles.manage".to_string()],
            },
            RoleRecord {
                name: "trader".to_string(),
                permissions: vec!["exchange_credentials.write".to_string()],
            },
        ]))
    }

    fn state_with(repo: Arc<FakeUserRepository>) -> web::Data<AppState> {
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
            user_repository: Some(repo),
            credential_repository: None,
            credential_cipher: None,
        })
    }

    /// Builds an `AuthContext` directly, bypassing the JWT middleware — these are handler
    /// unit tests, so the context is inserted into request extensions the same way
    /// `jwt_middleware` would after a successful decode, without re-testing the JWT layer.
    fn app_with_auth_context(permissions: &[&str]) -> AuthContext {
        AuthContext {
            user_id: 1,
            permissions: permissions
                .iter()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
        }
    }

    #[actix_web::test]
    async fn create_user_without_permission_returns_403() {
        let repo = seeded_repo();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo))
                .route("/admin/users", web::post().to(create_user)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/users")
            .set_json(serde_json::json!({"username": "bob", "password": "pw", "roles": []}))
            .to_request();
        // No AuthContext extension inserted -> unauthenticated.
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn create_user_with_permission_succeeds() {
        let repo = seeded_repo();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo))
                .route("/admin/users", web::post().to(create_user)),
        )
        .await;

        let ctx = app_with_auth_context(&["users.manage"]);
        let req = test::TestRequest::post()
            .uri("/admin/users")
            .set_json(serde_json::json!({"username": "bob", "password": "pw", "roles": ["trader"]}))
            .to_request();
        req.extensions_mut().insert(ctx);

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn create_user_with_wrong_permission_returns_403() {
        let repo = seeded_repo();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo))
                .route("/admin/users", web::post().to(create_user)),
        )
        .await;

        let ctx = app_with_auth_context(&["exchange_credentials.write"]);
        let req = test::TestRequest::post()
            .uri("/admin/users")
            .set_json(serde_json::json!({"username": "bob", "password": "pw", "roles": []}))
            .to_request();
        req.extensions_mut().insert(ctx);

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 403);
    }

    #[actix_web::test]
    async fn list_roles_returns_seeded_roles_with_permissions() {
        let repo = seeded_repo();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo))
                .route("/admin/roles", web::get().to(list_roles)),
        )
        .await;

        let ctx = app_with_auth_context(&["users.manage"]);
        let req = test::TestRequest::get().uri("/admin/roles").to_request();
        req.extensions_mut().insert(ctx);

        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let roles = body["data"]["roles"].as_array().unwrap();
        assert_eq!(roles.len(), 2);
    }

    #[actix_web::test]
    async fn create_role_requires_roles_manage_not_users_manage() {
        let repo = seeded_repo();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo))
                .route("/admin/roles", web::post().to(create_role)),
        )
        .await;

        // users.manage alone is not enough — create_role requires roles.manage.
        let ctx = app_with_auth_context(&["users.manage"]);
        let req = test::TestRequest::post()
            .uri("/admin/roles")
            .set_json(serde_json::json!({"name": "ops", "permissions": []}))
            .to_request();
        req.extensions_mut().insert(ctx);

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 403);
    }

    #[actix_web::test]
    async fn assign_user_roles_updates_assignment() {
        let repo = seeded_repo();
        let user = repo.create_user("carol", "hash").await.unwrap();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo.clone()))
                .route("/admin/users/{id}/roles", web::post().to(assign_user_roles)),
        )
        .await;

        let ctx = app_with_auth_context(&["users.manage"]);
        let req = test::TestRequest::post()
            .uri(&format!("/admin/users/{}/roles", user.id))
            .set_json(serde_json::json!({"roles": ["trader"]}))
            .to_request();
        req.extensions_mut().insert(ctx);

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        use crate::infrastructure::db::user_repository::UserRepository;
        assert_eq!(
            repo.roles_for_user(user.id).await.unwrap(),
            vec!["trader".to_string()]
        );
    }

    #[actix_web::test]
    async fn list_permissions_returns_catalog() {
        let repo = seeded_repo();
        let app = test::init_service(
            App::new()
                .app_data(state_with(repo))
                .route("/admin/permissions", web::get().to(list_permissions)),
        )
        .await;

        let ctx = app_with_auth_context(&["users.manage"]);
        let req = test::TestRequest::get()
            .uri("/admin/permissions")
            .to_request();
        req.extensions_mut().insert(ctx);

        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let perms = body["data"]["permissions"].as_array().unwrap();
        assert!(perms.iter().any(|p| p == "users.manage"));
    }
}
