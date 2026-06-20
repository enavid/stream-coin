use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct PlaceOrderRequest {
    pub exchange: String,
    pub pair: String,
    pub side: String,
    #[serde(rename = "type")]
    #[schema(rename = "type")]
    pub order_type: String,
    pub quantity: String,
    pub price: Option<String>,
    pub strategy_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CancelOrderRequest {
    pub client_order_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct OrderPlacedResponse {
    pub client_order_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct OrderItem {
    pub id: i64,
    pub exchange: String,
    pub pair: String,
    pub side: String,
    pub order_type: String,
    pub quantity: String,
    pub price: Option<String>,
    pub status: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: String,
    pub strategy_id: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct OrderListResponse {
    pub orders: Vec<OrderItem>,
}
