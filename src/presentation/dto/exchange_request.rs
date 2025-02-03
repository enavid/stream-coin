use serde::Deserialize;
use utoipa::{ToSchema, IntoParams};
use validator::{Validate, ValidationError};

fn validate_exchange_name(exchange_name: &str) -> Result<(), ValidationError> {
    if exchange_name != "kucoin" && exchange_name != "binance" {
        let mut error = ValidationError::new("invalid_exchange_name");
        error.message = Some("Exchange name must be 'kucoin' or 'binance'".into());
        return Err(error);
    }
    Ok(())
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ExchangeRequest {
    #[validate(custom(function = "validate_exchange_name"))]
    pub exchange_name: String,

    #[validate(length(min = 1, message = "At least one symbol must be provided"))]
    pub symbols: Vec<String>,
}
