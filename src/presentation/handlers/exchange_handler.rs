use std::sync::Arc;

use actix_web::{web, Responder};
use tokio::time::{interval, Duration};

use crate::presentation::dto::ticker::{SymbolRequest, TickerStarted};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

#[utoipa::path(
    post,
    path = "/v1/exchanges/futures/start_kline_symbol_ticker",
    tag = "Exchanges",
    request_body = SymbolRequest,
    responses(
        (status = 200, description = "Ticker started successfully", body = TickerStarted),
        (status = 400, description = "Redis unavailable or ticker already running", body = ApiError)
    )
)]
pub async fn start_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: web::Json<SymbolRequest>,
) -> impl Responder {
    let repo = match &state.ticker_repository {
        Some(r) => Arc::clone(r),
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "ticker request rejected: redis unavailable"
            );
            return ApiError::new("Redis unavailable", vec![]).to_response();
        }
    };

    match repo.exists(&request.exchange, &request.symbol).await {
        Ok(true) => {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "ticker already running"
            );
            return ApiError::new("Ticker already running", vec![]).to_response();
        }
        Ok(false) => {
            if let Err(e) = repo.register(&request.exchange, &request.symbol).await {
                tracing::error!(
                    exchange = %request.exchange,
                    symbol = %request.symbol,
                    error = %e,
                    "failed to register ticker"
                );
                return ApiError::new("Redis error", vec![]).to_response();
            }
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

    let exchange = request.exchange.clone();
    let symbol = request.symbol.clone();
    let repo_clone = Arc::clone(&repo);

    tokio::spawn(async move {
        let mut heartbeat = interval(Duration::from_secs(60));
        loop {
            heartbeat.tick().await;
            if let Err(e) = repo_clone.refresh(&exchange, &symbol).await {
                tracing::warn!(error = %e, "ticker heartbeat refresh failed");
            }
        }
    });

    tracing::info!(
        exchange = %request.exchange,
        symbol = %request.symbol,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_web::http::StatusCode;
    use actix_web::{test, web, App};
    use tokio::sync::Mutex;

    use super::*;
    use crate::presentation::shared::app_state::AppState;

    fn state_without_redis() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            ticker_repository: None,
            clients: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[actix_web::test]
    async fn ticker_returns_400_when_redis_unavailable() {
        let app = test::init_service(
            App::new()
                .app_data(state_without_redis())
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDT_IRT".to_string(),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn ticker_returns_redis_unavailable_message() {
        let app = test::init_service(
            App::new()
                .app_data(state_without_redis())
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDT_IRT".to_string(),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["message"], "Redis unavailable");
    }
}
