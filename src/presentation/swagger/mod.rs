use utoipa::openapi::OpenApi as OpenApiSpec;
use utoipa::{Modify, OpenApi};

use crate::presentation::dto::health::{Dependencies, HealthStatus};
use crate::presentation::dto::ticker::{SymbolRequest, TickerStarted};
use crate::presentation::responses::ApiError;

#[derive(OpenApi)]
#[openapi(
    info(title = "stream-coin", version = "0.1.0"),
    modifiers(&StripInfo),
    paths(
        crate::presentation::handlers::health_handler::health,
        crate::presentation::handlers::exchange_handler::start_kline_symbol_ticker,
    ),
    components(
        schemas(HealthStatus, Dependencies, SymbolRequest, TickerStarted, ApiError)
    ),
    tags(
        (name = "Health", description = "Service health and status"),
        (name = "Exchanges", description = "APIs for retrieving exchange data")
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
}
