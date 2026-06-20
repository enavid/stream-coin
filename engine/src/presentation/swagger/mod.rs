use utoipa::openapi::OpenApi as OpenApiSpec;
use utoipa::{Modify, OpenApi};

use crate::presentation::dto::health::{HealthStatus, ServiceStatus};
use crate::presentation::dto::order::{
    CancelOrderRequest, OrderItem, OrderListResponse, OrderPlacedResponse, PlaceOrderRequest,
};
use crate::presentation::dto::ticker::{
    ActiveTicker, SymbolRequest, TickerList, TickerStarted, TickerStopped,
};
use crate::presentation::responses::{ApiError, FieldError};

#[derive(OpenApi)]
#[openapi(
    info(title = "stream-coin", version = "0.1.0"),
    modifiers(&StripInfo),
    paths(
        crate::presentation::handlers::health_handler::health,
        crate::presentation::handlers::exchange_handler::start_kline_symbol_ticker,
        crate::presentation::handlers::exchange_handler::stop_kline_symbol_ticker,
        crate::presentation::handlers::exchange_handler::list_tickers,
        crate::presentation::handlers::order_handler::place_order,
        crate::presentation::handlers::order_handler::cancel_order,
        crate::presentation::handlers::order_handler::list_orders,
        crate::presentation::handlers::order_handler::reset_circuit_breaker,
    ),
    components(
        schemas(
            HealthStatus, ServiceStatus,
            SymbolRequest, TickerStarted, TickerStopped, ActiveTicker, TickerList,
            PlaceOrderRequest, CancelOrderRequest, OrderPlacedResponse, OrderItem, OrderListResponse,
            ApiError, FieldError
        )
    ),
    tags(
        (name = "Health", description = "Service health and status"),
        (name = "Exchanges", description = "APIs for retrieving exchange data"),
        (name = "Orders", description = "Order placement, cancellation, and circuit breaker control")
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
}
