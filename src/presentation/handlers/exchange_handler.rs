use actix_web::{web, Responder};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::time::{interval, Duration};

use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

#[derive(Serialize, Deserialize)]
pub struct SymbolRequest {
    pub exchange: String,
    pub symbol: String,
}

#[derive(Serialize)]
struct TickerStarted {
    exchange: String,
    symbol: String,
}

pub async fn start_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: web::Json<SymbolRequest>,
) -> impl Responder {
    let mut redis_conn = match state.redis.clone() {
        Some(conn) => conn,
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "ticker request rejected: redis unavailable"
            );
            return ApiError::new("Redis unavailable", vec![]).to_response();
        }
    };

    let redis_key = format!("ticker:{}:{}", request.exchange, request.symbol);
    let exists: Result<bool, _> = redis_conn.exists(&redis_key).await;

    match exists {
        Ok(true) => {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                redis_key = %redis_key,
                "ticker already running"
            );
            return ApiError::new("Ticker already running", vec![]).to_response();
        }
        Ok(false) => {
            let ticker_info = serde_json::json!({
                "exchange": request.exchange,
                "symbol": request.symbol,
                "status": "running",
                "started_at": chrono::Utc::now().timestamp()
            });
            let _: Result<(), _> = redis_conn
                .set_ex(&redis_key, ticker_info.to_string(), 120)
                .await;
        }
        Err(e) => {
            tracing::error!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                error = %e,
                "redis error while checking ticker"
            );
            return ApiError::new("Redis error", vec![]).to_response();
        }
    }

    let redis = state.redis.clone();
    let redis_key_clone = redis_key.clone();

    tokio::spawn(async move {
        let mut heartbeat = interval(Duration::from_secs(60));
        loop {
            heartbeat.tick().await;
            if let Some(mut conn) = redis.clone() {
                let _: Result<(), _> = conn.expire(&redis_key_clone, 120).await;
            }
        }
    });

    tracing::info!(
        exchange = %request.exchange,
        symbol = %request.symbol,
        redis_key = %redis_key,
        "ticker started"
    );

    success_response(
        "Ticker started",
        TickerStarted {
            exchange: request.exchange.clone(),
            symbol: request.symbol.clone(),
        },
    )
}
