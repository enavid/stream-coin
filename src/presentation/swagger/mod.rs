mod exchange_api;

use utoipa::OpenApi;
use exchange_api::start_exchange;
use crate::presentation::dto::exchange_request::ExchangeRequest;
use crate::presentation::swagger::exchange_api::__path_start_exchange;


#[derive(OpenApi)]
#[openapi(
    paths(
        start_exchange,
    ),
    components(schemas(ExchangeRequest)),
    tags(
        (name = "Exchange", description = "APIs related to exchange management")
    )
)]
pub struct ApiDoc;