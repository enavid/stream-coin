use std::collections::HashMap;
use std::str::FromStr;

use actix_web::{web, HttpRequest, HttpResponse};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::order::port::{OrderRequest, OrderSide, OrderType};
use crate::presentation::dto::order::{
    CancelOrderRequest, OrderItem, OrderListResponse, OrderPlacedResponse, PlaceOrderRequest,
};
use crate::presentation::middlewares::jwt::require_permission;
use crate::presentation::responses::{success_response, ApiError, FieldError};
use crate::presentation::shared::app_state::AppState;

/// Permission required to place, cancel, or list orders.
const ORDERS_MANAGE: &str = "orders.manage";
/// Permission required for the circuit-breaker safety control.
const ORDERS_ADMIN: &str = "orders.admin";

#[utoipa::path(
    post,
    path = "/v1/orders/place",
    tag = "Orders",
    request_body = PlaceOrderRequest,
    responses(
        (status = 200, description = "Order placed successfully", body = OrderPlacedResponse),
        (status = 400, description = "Validation error or position limit exceeded", body = ApiError),
        (status = 503, description = "Order manager not available")
    )
)]
pub async fn place_order(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<PlaceOrderRequest>,
) -> HttpResponse {
    let ctx = match require_permission(&req, ORDERS_MANAGE) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

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

    if req.pair.chars().filter(|&c| c == '/').count() != 1 {
        return ApiError::new(
            "validation failed",
            vec![FieldError::new(
                "pair",
                "must be in BASE/QUOTE format (e.g. USDT/IRT)",
            )],
        )
        .to_response();
    }

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

    tracing::info!(
        actor_user_id = ctx.user_id,
        exchange = %order_req.exchange,
        pair = %order_req.pair,
        side = ?order_req.side,
        "order placement requested"
    );

    match manager.place_order(order_req).await {
        Ok(client_order_id) => {
            tracing::info!(
                actor_user_id = ctx.user_id,
                client_order_id = %client_order_id,
                "order placed"
            );
            success_response("order placed", OrderPlacedResponse { client_order_id })
        }
        Err(e) => {
            tracing::error!(actor_user_id = ctx.user_id, error = %e, "place_order failed");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/orders/cancel",
    tag = "Orders",
    request_body = CancelOrderRequest,
    responses(
        (status = 200, description = "Order cancelled successfully"),
        (status = 400, description = "Order not found or already closed", body = ApiError),
        (status = 503, description = "Order manager not available")
    )
)]
pub async fn cancel_order(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<CancelOrderRequest>,
) -> HttpResponse {
    let ctx = match require_permission(&req, ORDERS_MANAGE) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

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
        Ok(()) => {
            tracing::info!(
                actor_user_id = ctx.user_id,
                client_order_id = %body.client_order_id,
                "order cancelled"
            );
            success_response("order cancelled", serde_json::json!({ "cancelled": true }))
        }
        Err(e) => {
            tracing::error!(actor_user_id = ctx.user_id, error = %e, client_order_id = %body.client_order_id, "cancel_order failed");
            ApiError::new(&e.to_string(), vec![]).to_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/orders",
    tag = "Orders",
    params(
        ("exchange" = Option<String>, Query, description = "Filter by exchange name"),
        ("pair" = Option<String>, Query, description = "Filter by trading pair (e.g. USDT/IRT)")
    ),
    responses(
        (status = 200, description = "Order list", body = OrderListResponse),
        (status = 503, description = "Order manager not available")
    )
)]
pub async fn list_orders(
    req: HttpRequest,
    state: web::Data<AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    if let Err(resp) = require_permission(&req, ORDERS_MANAGE) {
        return resp;
    }

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

#[utoipa::path(
    post,
    path = "/v1/admin/circuit-breaker/reset",
    tag = "Orders",
    responses(
        (status = 200, description = "Circuit breaker reset successfully"),
        (status = 503, description = "Order manager not available")
    )
)]
pub async fn reset_circuit_breaker(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let ctx = match require_permission(&req, ORDERS_ADMIN) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

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
    tracing::warn!(
        actor_user_id = ctx.user_id,
        "circuit breaker reset via admin endpoint"
    );
    success_response(
        "circuit breaker reset",
        serde_json::json!({ "reset": true }),
    )
}
