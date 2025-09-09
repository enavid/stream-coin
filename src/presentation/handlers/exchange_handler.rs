use serde_json::json;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::time::{interval, Duration};
use actix_web::{web, HttpResponse, Responder};
use crate::presentation::shared::app_state::AppState;
use crate::infrastructure::brokers::kafka_producer::send_to_kafka;


#[derive(Serialize, Deserialize)]
pub struct SymbolRequest {
    pub exchange: String,
    pub symbol: String,
}

pub async fn start_kline_symbol_tricker(
    state: web::Data<AppState>,
    request: web::Json<SymbolRequest>,
) -> impl Responder {

    let redis_key = format!("ticker:{}:{}", request.exchange, request.symbol);

    // Check if ticker already exists
    let mut redis_conn = state.redis.lock().await;
    let exists: Result<bool, _> = redis_conn.exists(&redis_key).await;

    match exists {
        Ok(true) => {
            return HttpResponse::Conflict().json(json!({
                "success": false,
                "message": "Ticker already running",
                "exchange": request.exchange,
                "symbol": request.symbol
            }));
        }
        Ok(false) => {
            // Create ticker entry in Redis
            let ticker_info = json!({
                "exchange": request.exchange,
                "symbol": request.symbol,
                "status": "running",
                "started_at": chrono::Utc::now().timestamp()
            });

            let _: Result<(), _> = redis_conn.set_ex(&redis_key, ticker_info.to_string(), 120).await;
        }
        Err(_) => {
            return HttpResponse::InternalServerError().json(json!({
                "success": false,
                "message": "Redis connection error"
            }));
        }
    }

    drop(redis_conn);

    let kafka_producer = state.kafka.clone();
    let redis_client = state.redis.clone();
    let symbol = request.symbol.clone();
    let exchange = request.exchange.clone();
    let redis_key_clone = redis_key.clone();

    tokio::spawn(async move {
        let mut delay = interval(Duration::from_secs(1));
        let mut heartbeat_interval = interval(Duration::from_secs(60));
        let topic = "kline-symbol-ticker";
        let key = "ticker-update";

        loop {
            tokio::select! {
                _ = delay.tick() => {
                    let data = json!({
                        "exchange": exchange,
                        "symbol": symbol,
                        "timestamp": chrono::Utc::now().timestamp_millis()
                    });

                    let payload = data.to_string();
                    let _ = send_to_kafka(&kafka_producer, topic, key, &payload).await;
                }
                _ = heartbeat_interval.tick() => {
                    // Refresh TTL every minute
                    let mut redis_conn = redis_client.lock().await;
                    let _: Result<(), _> = redis_conn.expire(&redis_key_clone, 120).await;
                }
            }
        }
    });

    HttpResponse::Ok().json(json!({
        "success": true,
        "message": "Ticker started",
        "exchange": request.exchange,
        "symbol": request.symbol
    }))
}