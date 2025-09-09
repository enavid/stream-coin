mod exchange_api;
use utoipa::OpenApi;
use exchange_api::{get_exchange_names};
use crate::presentation::responses::ApiSuccess;
use crate::presentation::dto::exchange::{ExchangeNameList};


use crate::presentation::swagger::exchange_api::{
    __path_get_exchange_names
};

#[derive(OpenApi)]
#[openapi(
    paths(
        get_exchange_names
    ),
    components(
        schemas(ExchangeNameList, ApiSuccess<ExchangeNameList>)
    ),
    tags(
        (name = "Exchanges", description = "APIs for retrieving exchange data")
    )
)]
pub struct ApiDoc;
