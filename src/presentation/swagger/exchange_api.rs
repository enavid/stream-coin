use crate::presentation::dto::exchange_request::ExchangeRequest;

#[utoipa::path(
    post,
    path = "/exchange/connect",
    request_body = ExchangeRequest,
    responses(
        (status = 200, description = "Exchange started successfully"),
        (status = 400, description = "Invalid request")
    ),
    tag = "Exchange"
)]
pub async fn start_exchange() {}

#[utoipa::path(
    post,
    path = "/exchange/disconnect",
    request_body = ExchangeRequest,
    responses(
        (status = 200, description = "Exchange stopped successfully"),
        (status = 400, description = "Invalid request")
    ),
    tag = "Exchange"
)]
pub async fn stop_exchange() {}
