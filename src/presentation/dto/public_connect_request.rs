use utoipa::ToSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublicConnectRequest {
    pub exchange: String,
    pub symbols: Vec<String>,
    pub channels: Vec<String>,
}
