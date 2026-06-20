use std::collections::HashMap;
use std::str::FromStr;

use actix_web::{web, HttpResponse};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::order::port::{OrderRequest, OrderSide, OrderType};
use crate::presentation::dto::order::{
    CancelOrderRequest, OrderItem, OrderListResponse, OrderPlacedResponse, PlaceOrderRequest,
};
use crate::presentation::responses::{success_response, ApiError, FieldError};
use crate::presentation::shared::app_state::AppState;

pub async fn place_order(
    state: web::Data<AppState>,
    body: web::Json<PlaceOrderRequest>,
) -> HttpResponse {
    let manager = match &state.order_manager {
        Some(m) => m.clone(),
        None => {
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "success": false,
                "message": "order manager not available"
            }))
        }
    };

    let req = body.into_inner();

    let side = match req.side.to_lowercase().as_str() {
        "buy" => OrderSide::Buy,
        "sell" => OrderSide::Sell,
        other => {
            return ApiError::new(
                "validation failed",
                vec![FieldError::new(
                    "side",
                    &format!("must be 'buy' or 'sell', got '{other}'"),
                )],
            )
            .to_response()
        }
    };

    let order_type = match req.order_type.to_lowercase().as_str() {
        "market" => OrderType::Market,
        "limit" => OrderType::Limit,
        other => {
            return ApiError::new(
                "validation failed",
                vec![FieldError::new(
                    "type",
                    &format!("must be 'market' or 'limit', got '{other}'"),
                )],
            )
            .to_response()
        }
    };

    let quantity = match Decimal::from_str(&req.quantity) {
        Ok(d) if d > Decimal::ZERO => d,
        _ => {
            return ApiError::new(
                "validation failed",
                vec![FieldError::new(
                    "quantity",
                    "must be a positive decimal number",
                )],
            )
            .to_response()
        }
    };

    let price = if let Some(p) = req.price {
        match Decimal::from_str(&p) {
            Ok(d) if d > Decimal::ZERO => Some(d),
            _ => {
                return ApiError::new(
                    "validation failed",
                    vec![FieldError::new(
                        "price",
                        "must be a positive decimal number",
                    )],
                )
                .to_response()
            }
        }
    } else {
        None
    };

    let order_req = OrderRequest {
        exchange: req.exchange,
        pair: req.pair,
        side,
        order_type,
        quantity,
        price,
        client_order_id: Uuid::new_v4().to_string(),
        strategy_id: req.strategy_id,
    };

    match manager.place_order(order_req).await {
        Ok(client_order_id) => {
            success_response("order placed", OrderPlacedResponse { client_order_id })
        }
        Err(e) => {
            tracing::error!(error = %e, "place_order failed");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

pub async fn cancel_order(
    state: web::Data<AppState>,
    body: web::Json<CancelOrderRequest>,
) -> HttpResponse {
    let manager = match &state.order_manager {
        Some(m) => m.clone(),
        None => {
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "success": false,
                "message": "order manager not available"
            }))
        }
    };

    match manager.cancel_order(&body.client_order_id).await {
        Ok(()) => success_response("order cancelled", serde_json::json!({ "cancelled": true })),
        Err(e) => {
            tracing::error!(error = %e, client_order_id = %body.client_order_id, "cancel_order failed");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

pub async fn list_orders(
    state: web::Data<AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let manager = match &state.order_manager {
        Some(m) => m.clone(),
        None => {
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "success": false,
                "message": "order manager not available"
            }))
        }
    };

    let exchange = query.get("exchange").map(|s| s.as_str());
    let pair = query.get("pair").map(|s| s.as_str());

    match manager.list_orders(exchange, pair).await {
        Ok(records) => {
            let items: Vec<OrderItem> = records
                .into_iter()
                .map(|r| OrderItem {
                    id: r.id.unwrap_or(0),
                    exchange: r.exchange,
                    pair: r.pair,
                    side: r.side,
                    order_type: r.order_type,
                    quantity: r.quantity.to_string(),
                    price: r.price.map(|p| p.to_string()),
                    status: r.status,
                    exchange_order_id: r.exchange_order_id,
                    client_order_id: r.client_order_id,
                    strategy_id: r.strategy_id,
                    created_at: r.created_at.to_rfc3339(),
                })
                .collect();
            success_response("ok", OrderListResponse { orders: items })
        }
        Err(e) => {
            tracing::error!(error = %e, "list_orders failed");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

pub async fn reset_circuit_breaker(state: web::Data<AppState>) -> HttpResponse {
    let manager = match &state.order_manager {
        Some(m) => m.clone(),
        None => {
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "success": false,
                "message": "order manager not available"
            }))
        }
    };

    manager.reset_circuit_breaker().await;
    tracing::info!("circuit breaker reset via admin endpoint");
    success_response(
        "circuit breaker reset",
        serde_json::json!({ "reset": true }),
    )
}
