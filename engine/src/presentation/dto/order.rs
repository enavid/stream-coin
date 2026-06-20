use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct PlaceOrderRequest {
    pub exchange: String,
    pub pair: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub quantity: String,
    pub price: Option<String>,
    pub strategy_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CancelOrderRequest {
    pub client_order_id: String,
}

#[derive(Serialize)]
pub struct OrderPlacedResponse {
    pub client_order_id: String,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct OrderListResponse {
    pub orders: Vec<OrderItem>,
}
