use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::OpenApi as OpenApiSpec;
use utoipa::{Modify, OpenApi};

use crate::backtest::entity::{
    BacktestResult, BacktestSignalRecord, ClosedTrade, TradeOutcome, TradeRecord, TradeSide,
};
use crate::candle::entity::{CandlePayload, Interval};
use crate::presentation::dto::auth::{LoginRequest, RefreshRequest, TokenResponse};
use crate::presentation::dto::backtest::BacktestRunRequest;
use crate::presentation::dto::candle::{BackfillRequest, BackfillResponse};
use crate::presentation::dto::credential::{CredentialListResponse, CredentialSummaryResponse};
use crate::presentation::dto::exchange::{
    ExchangeListResponse, ExchangeNameRequest, ExchangeResponse, PairListResponse, PairResponse,
    SeedPairsResponse,
};
use crate::presentation::dto::health::{HealthStatus, ServiceStatus};
use crate::presentation::dto::order::{
    CancelOrderRequest, OrderItem, OrderListResponse, OrderPlacedResponse, PlaceOrderRequest,
};
use crate::presentation::dto::strategy::{
    ActiveStrategy, DeployStrategyRequest, DeployedStrategy, RegisterStrategyRequest,
    StartStrategyRequest, StopStrategyRequest, StrategyList,
};
use crate::presentation::dto::subscription::{
    SubscribeRequest, SubscriptionListResponse, SubscriptionResponse, UpdateSubscriptionRequest,
};
use crate::presentation::dto::ticker::{
    ActiveTicker, SymbolRequest, TickerList, TickerStarted, TickerStopped,
};
use crate::presentation::dto::user::{
    AssignRolesRequest, CreateRoleRequest, CreateUserRequest, PermissionListResponse,
    RoleListResponse, RoleResponse, UserListResponse, UserResponse,
};
use crate::presentation::handlers::admin_handler::{
    AdminHaltRequest, AdminHaltResponse, AdminPlaceOrderRequest,
};
use crate::presentation::responses::{ApiError, FieldError};

#[derive(OpenApi)]
#[openapi(
    info(title = "stream-coin", version = "0.1.0"),
    modifiers(&StripInfo, &SecurityAddon),
    security(("bearer_jwt" = [])),
    paths(
        crate::presentation::handlers::health_handler::health,
        crate::presentation::handlers::exchange_handler::start_kline_symbol_ticker,
        crate::presentation::handlers::exchange_handler::stop_kline_symbol_ticker,
        crate::presentation::handlers::exchange_handler::list_tickers,
        crate::presentation::handlers::order_handler::place_order,
        crate::presentation::handlers::order_handler::cancel_order,
        crate::presentation::handlers::order_handler::list_orders,
        crate::presentation::handlers::order_handler::reset_circuit_breaker,
        crate::presentation::handlers::auth_handler::login,
        crate::presentation::handlers::auth_handler::refresh,
        crate::presentation::handlers::registry_handler::list_exchanges,
        crate::presentation::handlers::registry_handler::list_exchange_pairs,
        crate::presentation::handlers::registry_handler::enable_exchange,
        crate::presentation::handlers::registry_handler::disable_exchange,
        crate::presentation::handlers::registry_handler::seed_pairs_from_assets,
        crate::presentation::handlers::credential_handler::list_own_credentials,
        crate::presentation::handlers::credential_handler::set_own_credentials,
        crate::presentation::handlers::credential_handler::delete_own_credentials,
        crate::presentation::handlers::strategy_handler::start_strategy,
        crate::presentation::handlers::strategy_handler::stop_strategy,
        crate::presentation::handlers::strategy_handler::register_strategy,
        crate::presentation::handlers::strategy_handler::deploy_strategy,
        crate::presentation::handlers::strategy_handler::list_strategies,
        crate::presentation::handlers::subscription_handler::subscribe,
        crate::presentation::handlers::subscription_handler::list_subscriptions,
        crate::presentation::handlers::subscription_handler::update_subscription,
        crate::presentation::handlers::subscription_handler::delete_subscription,
        crate::presentation::handlers::backtest_handler::run_backtest,
        crate::presentation::handlers::candle_handler::get_candles,
        crate::presentation::handlers::candle_handler::backfill_candles,
        crate::presentation::handlers::user_handler::create_user,
        crate::presentation::handlers::user_handler::list_users,
        crate::presentation::handlers::user_handler::assign_user_roles,
        crate::presentation::handlers::user_handler::list_roles,
        crate::presentation::handlers::user_handler::create_role,
        crate::presentation::handlers::user_handler::list_permissions,
        crate::presentation::handlers::admin_handler::admin_place_order_for_user,
        crate::presentation::handlers::admin_handler::admin_halt_user_strategies,
    ),
    components(
        schemas(
            HealthStatus, ServiceStatus,
            SymbolRequest, TickerStarted, TickerStopped, ActiveTicker, TickerList,
            PlaceOrderRequest, CancelOrderRequest, OrderPlacedResponse, OrderItem, OrderListResponse,
            ApiError, FieldError,
            LoginRequest, RefreshRequest, TokenResponse,
            ExchangeResponse, ExchangeListResponse, PairResponse, PairListResponse,
            ExchangeNameRequest, SeedPairsResponse,
            CredentialSummaryResponse, CredentialListResponse,
            StartStrategyRequest, StopStrategyRequest, RegisterStrategyRequest,
            ActiveStrategy, StrategyList, DeployStrategyRequest, DeployedStrategy,
            SubscribeRequest, UpdateSubscriptionRequest, SubscriptionResponse, SubscriptionListResponse,
            BacktestRunRequest, BacktestResult, TradeRecord, BacktestSignalRecord,
            ClosedTrade, TradeSide, TradeOutcome,
            BackfillRequest, BackfillResponse, CandlePayload, Interval,
            CreateUserRequest, UserResponse, UserListResponse, AssignRolesRequest,
            CreateRoleRequest, RoleResponse, RoleListResponse, PermissionListResponse,
            AdminPlaceOrderRequest, AdminHaltRequest, AdminHaltResponse
        )
    ),
    tags(
        (name = "Health", description = "Service health and status"),
        (name = "Auth", description = "Login and token refresh"),
        (name = "Exchanges", description = "Exchange registry, pairs, and admin controls"),
        (name = "Credentials", description = "Per-user exchange API credentials"),
        (name = "Strategies", description = "Strategy lifecycle and Python deployment"),
        (name = "Subscriptions", description = "User strategy subscriptions"),
        (name = "Backtest", description = "Historical strategy backtesting"),
        (name = "Candles", description = "Candle history and backfill"),
        (name = "Orders", description = "Order placement, cancellation, and circuit breaker control"),
        (name = "Admin", description = "User/role administration and privileged operations")
    )
)]
pub struct ApiDoc;

struct StripInfo;

impl Modify for StripInfo {
    fn modify(&self, openapi: &mut OpenApiSpec) {
        openapi.info.contact = None;
        openapi.info.license = None;
        openapi.info.description = None;
    }
}

/// Registers the global Bearer-JWT security scheme. Most endpoints require an
/// `Authorization: Bearer <HS256 JWT>` header (validated by `jwt_middleware`);
/// the public endpoints opt out per-path with `security(())`.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut OpenApiSpec) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_jwt",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    use super::*;

    #[test]
    fn swagger_has_no_contact() {
        let api = ApiDoc::openapi();
        assert!(api.info.contact.is_none());
    }

    #[test]
    fn swagger_has_no_license() {
        let api = ApiDoc::openapi();
        assert!(api.info.license.is_none());
    }

    #[test]
    fn swagger_title_is_stream_coin() {
        let api = ApiDoc::openapi();
        assert_eq!(api.info.title, "stream-coin");
    }

    #[test]
    fn swagger_registers_health_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/check/health"));
    }

    #[test]
    fn swagger_registers_ticker_path() {
        let api = ApiDoc::openapi();
        assert!(api
            .paths
            .paths
            .contains_key("/v1/exchanges/futures/start_kline_symbol_ticker"));
    }

    #[test]
    fn swagger_registers_stop_ticker_path() {
        let api = ApiDoc::openapi();
        assert!(api
            .paths
            .paths
            .contains_key("/v1/exchanges/futures/stop_kline_symbol_ticker"));
    }

    #[test]
    fn swagger_registers_list_tickers_path() {
        let api = ApiDoc::openapi();
        assert!(api
            .paths
            .paths
            .contains_key("/v1/exchanges/futures/tickers"));
    }

    #[test]
    fn swagger_registers_place_order_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/orders/place"));
    }

    #[test]
    fn swagger_registers_cancel_order_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/orders/cancel"));
    }

    #[test]
    fn swagger_registers_list_orders_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/orders"));
    }

    #[test]
    fn swagger_registers_circuit_breaker_reset_path() {
        let api = ApiDoc::openapi();
        assert!(api
            .paths
            .paths
            .contains_key("/v1/admin/circuit-breaker/reset"));
    }

    #[test]
    fn swagger_registers_auth_token_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/auth/token"));
    }

    #[test]
    fn swagger_registers_strategies_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/strategies"));
    }

    #[test]
    fn swagger_registers_subscriptions_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/subscriptions"));
    }

    #[test]
    fn swagger_registers_candles_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/candles"));
    }

    #[test]
    fn swagger_registers_backtest_run_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/backtest/run"));
    }

    #[test]
    fn swagger_registers_admin_users_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/admin/users"));
    }

    #[test]
    fn swagger_registers_exchanges_pairs_path() {
        let api = ApiDoc::openapi();
        assert!(api.paths.paths.contains_key("/v1/exchanges/{name}/pairs"));
    }

    #[test]
    fn swagger_registers_credentials_path() {
        let api = ApiDoc::openapi();
        assert!(api
            .paths
            .paths
            .contains_key("/v1/exchanges/{name}/credentials"));
    }

    #[test]
    fn swagger_defines_bearer_jwt_security_scheme() {
        let api = ApiDoc::openapi();
        assert!(api
            .components
            .unwrap()
            .security_schemes
            .contains_key("bearer_jwt"));
    }
}
