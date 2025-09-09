use crate::presentation::responses::ApiSuccess;
use crate::presentation::dto::exchange::ExchangeNameList;

#[utoipa::path(
    get,
    path = "/v1/exchanges/names",
    tag = "Exchanges",
    responses(
        (status = 200, description = "List of exchange names", body = ApiSuccess<ExchangeNameList>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_exchange_names(){}
