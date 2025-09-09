use actix_web::web::JsonConfig;
use actix_web::error::JsonPayloadError;
use crate::presentation::responses::ApiError;

pub fn json_error_handler_config() -> JsonConfig {
    JsonConfig::default().error_handler(|err, _req| {
        match err {
            JsonPayloadError::ContentType => {
                ApiError::new(
                    "Invalid Content-Type. Expected application/json",
                    vec![]
                ).into()
            },
            JsonPayloadError::Deserialize(e) => {
                ApiError::new(
                    "Invalid request body",
                    vec![e.to_string()]
                ).into()
            },
            JsonPayloadError::Payload(e) => {
                let msg = match e {
                    actix_web::error::PayloadError::Overflow => "Payload too large",
                    _ => "Payload error",
                };
                ApiError::new(msg, vec![]).into()
            },
            _ => {
                ApiError::new(
                    "Failed to parse JSON",
                    vec![]
                ).into()
            }
        }
    })
}
