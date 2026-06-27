use std::str::FromStr;

use actix_web::{web, HttpRequest, HttpResponse};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::order::port::{OrderRequest, OrderSide, OrderType};
use crate::presentation::middlewares::jwt::require_permission;
use crate::presentation::responses::{success_response, ApiError, FieldError};
use crate::presentation::shared::app_state::AppState;

const PERMISSION: &str = "orders.admin";

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct AdminPlaceOrderRequest {
    /// Target user whose stored credentials are used to place the order.
    pub user_id: i32,
    /// Canonical exchange name (e.g. `"tabdeal"`).
    pub exchange: String,
    /// Trading pair in `"BASE/QUOTE"` form (e.g. `"USDT/IRT"`).
    pub pair: String,
    /// `"buy"` or `"sell"`.
    pub side: String,
    /// `"market"` or `"limit"`.
    #[serde(rename = "type")]
    pub order_type: String,
    /// Quantity in base currency (decimal string, e.g. `"100.5"`).
    pub quantity: String,
    /// Limit price (required for limit orders).
    pub price: Option<String>,
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct AdminHaltRequest {
    /// User whose signal subscriptions should be deactivated.
    pub user_id: i32,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct AdminHaltResponse {
    pub halted: u64,
}

/// `POST /v1/admin/orders/place` — places an order on behalf of a specific user using
/// their stored exchange credentials.  Requires `orders.admin` permission.
///
/// The order adapter is constructed dynamically from the user's decrypted credentials
/// via the credential resolver; the global order adapter registry is not used.
#[utoipa::path(
    post,
    path = "/v1/admin/orders/place",
    tag = "Admin",
    request_body = AdminPlaceOrderRequest,
    responses(
        (status = 200, description = "Order placed"),
        (status = 400, description = "Validation error or missing credentials", body = ApiError),
        (status = 401, description = "Not authenticated or missing permission"),
        (status = 503, description = "Order manager not available")
    )
)]
pub async fn admin_place_order_for_user(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<AdminPlaceOrderRequest>,
) -> HttpResponse {
    let _ctx = match require_permission(&req, PERMISSION) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let manager = match &state.order_manager {
        Some(m) => m.clone(),
        None => {
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "success": false,
                "message": "order manager not available"
            }))
        }
    };

    let body = body.into_inner();

    if body.pair.chars().filter(|&c| c == '/').count() != 1 {
        return ApiError::new(
            "validation failed",
            vec![FieldError::new("pair", "must be in BASE/QUOTE format")],
        )
        .to_response();
    }

    let side = match body.side.to_lowercase().as_str() {
        "buy" => OrderSide::Buy,
        "sell" => OrderSide::Sell,
        other => {
            return ApiError::new(
                "validation failed",
                vec![FieldError::new(
                    "side",
                    &format!("must be 'buy' or 'sell', got '{other}'"),
                )],
            )
            .to_response()
        }
    };

    let order_type = match body.order_type.to_lowercase().as_str() {
        "market" => OrderType::Market,
        "limit" => OrderType::Limit,
        other => {
            return ApiError::new(
                "validation failed",
                vec![FieldError::new(
                    "type",
                    &format!("must be 'market' or 'limit', got '{other}'"),
                )],
            )
            .to_response()
        }
    };

    let quantity = match Decimal::from_str(&body.quantity) {
        Ok(d) if d > Decimal::ZERO => d,
        _ => {
            return ApiError::new(
                "validation failed",
                vec![FieldError::new("quantity", "must be a positive decimal")],
            )
            .to_response()
        }
    };

    let price = if let Some(p) = body.price {
        match Decimal::from_str(&p) {
            Ok(d) if d > Decimal::ZERO => Some(d),
            _ => {
                return ApiError::new(
                    "validation failed",
                    vec![FieldError::new("price", "must be a positive decimal")],
                )
                .to_response()
            }
        }
    } else {
        None
    };

    let order_req = OrderRequest {
        exchange: body.exchange.clone(),
        pair: body.pair.clone(),
        side,
        order_type,
        quantity,
        price,
        client_order_id: Uuid::new_v4().to_string(),
        strategy_id: None,
    };

    tracing::info!(
        admin_user_id = _ctx.user_id,
        target_user_id = body.user_id,
        exchange = %body.exchange,
        pair = %body.pair,
        "admin: placing order for user"
    );

    match manager.place_order_for_user(body.user_id, order_req).await {
        Ok(client_order_id) => success_response(
            "order placed",
            serde_json::json!({ "client_order_id": client_order_id }),
        ),
        Err(e) => {
            use crate::order::manager::OrderManagerError;
            match &e {
                OrderManagerError::NoCredentialResolver
                | OrderManagerError::NoCredentialsForUser { .. } => {
                    tracing::warn!(
                        target_user_id = body.user_id,
                        error = %e,
                        "admin place_order: credential not available"
                    );
                }
                _ => {
                    tracing::error!(
                        target_user_id = body.user_id,
                        error = %e,
                        "admin place_order failed"
                    );
                }
            }
            crate::presentation::handlers::order_handler::order_manager_error_to_api(&e)
                .to_response()
        }
    }
}

/// `POST /v1/admin/strategies/halt` — deactivates all signal subscriptions for the
/// specified user, stopping automatic order placement for that user on all strategies.
/// Requires `orders.admin` permission.
#[utoipa::path(
    post,
    path = "/v1/admin/strategies/halt",
    tag = "Admin",
    request_body = AdminHaltRequest,
    responses(
        (status = 200, description = "Subscriptions halted", body = AdminHaltResponse),
        (status = 401, description = "Not authenticated or missing permission"),
        (status = 503, description = "Subscription repository not available")
    )
)]
pub async fn admin_halt_user_strategies(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<AdminHaltRequest>,
) -> HttpResponse {
    let ctx = match require_permission(&req, PERMISSION) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let Some(sub_repo) = &state.subscription_repository else {
        return ApiError::service_unavailable("Subscription repository not configured")
            .to_response();
    };

    let user_id = body.user_id;

    tracing::warn!(
        admin_user_id = ctx.user_id,
        target_user_id = user_id,
        "admin: halting all strategy subscriptions for user"
    );

    match sub_repo.halt_for_user(user_id).await {
        Ok(halted) => {
            tracing::info!(
                admin_user_id = ctx.user_id,
                target_user_id = user_id,
                halted,
                "admin halt: subscriptions deactivated"
            );
            success_response("subscriptions halted", AdminHaltResponse { halted })
        }
        Err(e) => {
            tracing::error!(error = %e, target_user_id = user_id, "admin halt_for_user failed");
            ApiError::internal_error().to_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::time::Duration;

    use actix_web::{test, App, HttpMessage};
    use rust_decimal::Decimal;
    use tokio::sync::{broadcast, RwLock};

    use super::*;
    use crate::infrastructure::db::order_repository::FakeOrderRepository;
    use crate::infrastructure::db::subscription_repository::{
        FakeSubscriptionRepository, SubscriptionRepository,
    };
    use crate::order::credential_resolver::FakeCredentialResolver;
    use crate::order::entity::SafetyConfig;
    use crate::order::fake::FakeOrderAdapter;
    use crate::order::manager::OrderManager;
    use crate::order::port::OrderAdapter;
    use crate::presentation::middlewares::jwt::AuthContext;
    use crate::presentation::shared::app_state::AdapterFactory;

    fn admin_ctx() -> AuthContext {
        AuthContext {
            user_id: 1,
            permissions: HashSet::from(["orders.admin".to_string()]),
        }
    }

    fn build_state_with_manager(
        manager: Arc<OrderManager>,
        sub_repo: Arc<FakeSubscriptionRepository>,
    ) -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(tokio::sync::Mutex::new(
                crate::exchange::registry::ExchangeRegistry::new(),
            )),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: Some(Arc::new("test-secret".to_string())),
            ticker_repository: None,
            running_strategies: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            order_manager: Some(manager),
            python_strategy_repository: None,
            candle_repository: None,
            historical_sources: Arc::new(HashMap::new()),
            candle_history: AppState::new_candle_history(),
            exchange_repository: None,
            asset_repository: None,
            subscription_repository: Some(sub_repo),
            user_repository: None,
            credential_repository: None,
            credential_cipher: None,
        })
    }

    fn build_manager_with_resolver(user_adapter: Arc<FakeOrderAdapter>) -> Arc<OrderManager> {
        let resolver = Arc::new(FakeCredentialResolver::returning(
            Arc::clone(&user_adapter) as Arc<dyn OrderAdapter>
        ));
        let (broadcaster, _) = broadcast::channel(16);
        Arc::new(
            OrderManager::with_poll_interval(
                Arc::new(RwLock::new(HashMap::new())),
                Arc::new(FakeOrderRepository::new()),
                broadcaster,
                SafetyConfig {
                    dry_run: false,
                    default_order_quantity: Decimal::new(100, 0),
                    max_position_size: Decimal::new(10_000, 0),
                    min_confidence: 0.7,
                    circuit_breaker_max_orders: 50,
                    circuit_breaker_window_secs: 60,
                },
                Duration::from_millis(10),
            )
            .with_credential_resolver(resolver),
        )
    }

    // -- admin_place_order_for_user ------------------------------------------

    #[actix_web::test]
    async fn admin_place_order_without_permission_returns_401() {
        let user_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let manager = build_manager_with_resolver(Arc::clone(&user_adapter));
        let state = build_state_with_manager(manager, Arc::new(FakeSubscriptionRepository::new()));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/orders/place",
            web::post().to(admin_place_order_for_user),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/orders/place")
            .set_json(serde_json::json!({
                "user_id": 42, "exchange": "tabdeal", "pair": "USDT/IRT",
                "side": "buy", "type": "market", "quantity": "100"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn admin_place_order_with_resolver_uses_user_credentials() {
        let user_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let manager = build_manager_with_resolver(Arc::clone(&user_adapter));
        let state = build_state_with_manager(manager, Arc::new(FakeSubscriptionRepository::new()));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/orders/place",
            web::post().to(admin_place_order_for_user),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/orders/place")
            .set_json(serde_json::json!({
                "user_id": 42, "exchange": "tabdeal", "pair": "USDT/IRT",
                "side": "buy", "type": "market", "quantity": "100"
            }))
            .to_request();
        req.extensions_mut().insert(admin_ctx());

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(user_adapter.placed_count().await, 1);
    }

    #[actix_web::test]
    async fn admin_place_order_invalid_pair_returns_400() {
        let user_adapter = Arc::new(FakeOrderAdapter::new("tabdeal"));
        let manager = build_manager_with_resolver(Arc::clone(&user_adapter));
        let state = build_state_with_manager(manager, Arc::new(FakeSubscriptionRepository::new()));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/orders/place",
            web::post().to(admin_place_order_for_user),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/orders/place")
            .set_json(serde_json::json!({
                "user_id": 42, "exchange": "tabdeal", "pair": "USDTIRT",
                "side": "buy", "type": "market", "quantity": "100"
            }))
            .to_request();
        req.extensions_mut().insert(admin_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn admin_place_order_no_credentials_returns_400() {
        let resolver = Arc::new(FakeCredentialResolver::none());
        let (broadcaster, _) = broadcast::channel(16);
        let manager = Arc::new(
            OrderManager::with_poll_interval(
                Arc::new(RwLock::new(HashMap::new())),
                Arc::new(FakeOrderRepository::new()),
                broadcaster,
                SafetyConfig::default(),
                Duration::from_millis(10),
            )
            .with_credential_resolver(resolver),
        );
        let state = build_state_with_manager(manager, Arc::new(FakeSubscriptionRepository::new()));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/orders/place",
            web::post().to(admin_place_order_for_user),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/orders/place")
            .set_json(serde_json::json!({
                "user_id": 42, "exchange": "tabdeal", "pair": "USDT/IRT",
                "side": "buy", "type": "market", "quantity": "100"
            }))
            .to_request();
        req.extensions_mut().insert(admin_ctx());
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    // -- admin_halt_user_strategies ------------------------------------------

    #[actix_web::test]
    async fn admin_halt_without_permission_returns_401() {
        let (broadcaster, _) = broadcast::channel(16);
        let manager = Arc::new(OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(FakeOrderRepository::new()),
            broadcaster,
            SafetyConfig::default(),
            Duration::from_millis(10),
        ));
        let state = build_state_with_manager(manager, Arc::new(FakeSubscriptionRepository::new()));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/strategies/halt",
            web::post().to(admin_halt_user_strategies),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/strategies/halt")
            .set_json(serde_json::json!({"user_id": 5}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn admin_halt_deactivates_all_user_subscriptions() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        sub_repo.create(5, "s1", None, None).await.unwrap();
        sub_repo.create(5, "s2", None, None).await.unwrap();
        sub_repo.create(7, "s1", None, None).await.unwrap();

        let (broadcaster, _) = broadcast::channel(16);
        let manager = Arc::new(OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(FakeOrderRepository::new()),
            broadcaster,
            SafetyConfig::default(),
            Duration::from_millis(10),
        ));
        let state = build_state_with_manager(manager, Arc::clone(&sub_repo));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/strategies/halt",
            web::post().to(admin_halt_user_strategies),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/strategies/halt")
            .set_json(serde_json::json!({"user_id": 5}))
            .to_request();
        req.extensions_mut().insert(admin_ctx());

        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["halted"], 2);

        let user5 = sub_repo.list_for_user(5).await.unwrap();
        assert!(
            user5.iter().all(|s| !s.active),
            "user 5 subs must be inactive"
        );
        let user7 = sub_repo.list_for_user(7).await.unwrap();
        assert!(
            user7.iter().all(|s| s.active),
            "user 7 sub must remain active"
        );
    }

    #[actix_web::test]
    async fn admin_halt_returns_zero_when_user_has_no_subscriptions() {
        let sub_repo = Arc::new(FakeSubscriptionRepository::new());
        let (broadcaster, _) = broadcast::channel(16);
        let manager = Arc::new(OrderManager::with_poll_interval(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(FakeOrderRepository::new()),
            broadcaster,
            SafetyConfig::default(),
            Duration::from_millis(10),
        ));
        let state = build_state_with_manager(manager, Arc::clone(&sub_repo));

        let app = test::init_service(App::new().app_data(state).route(
            "/admin/strategies/halt",
            web::post().to(admin_halt_user_strategies),
        ))
        .await;

        let req = test::TestRequest::post()
            .uri("/admin/strategies/halt")
            .set_json(serde_json::json!({"user_id": 99}))
            .to_request();
        req.extensions_mut().insert(admin_ctx());

        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["data"]["halted"], 0);
    }
}
