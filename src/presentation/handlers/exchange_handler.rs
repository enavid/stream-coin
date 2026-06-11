use std::sync::Arc;

use actix_web::{web, Responder};
use tokio::sync::mpsc;

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
        (status = 400, description = "Exchange not supported or ticker already running", body = ApiError)
    )
)]
pub async fn start_kline_symbol_ticker(
    state: web::Data<AppState>,
    request: web::Json<SymbolRequest>,
) -> impl Responder {
    let adapter = match state.exchange_adapters.get(&request.exchange) {
        Some(a) => Arc::clone(a),
        None => {
            tracing::warn!(
                exchange = %request.exchange,
                "ticker request rejected: exchange not supported"
            );
            return ApiError::new("Exchange not supported", vec![]).to_response();
        }
    };

    let key = format!("{}:{}", request.exchange, request.symbol);
    {
        let clients = state.clients.lock().await;
        if clients.contains_key(&key) {
            tracing::warn!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                "ticker already running"
            );
            return ApiError::new("Ticker already running", vec![]).to_response();
        }
    }

    let (tx, mut rx) = mpsc::channel(100);

    let abort = match adapter.subscribe(&request.symbol, tx).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(
                exchange = %request.exchange,
                symbol = %request.symbol,
                error = %e,
                "failed to start ticker"
            );
            return ApiError::new("Failed to start ticker", vec![]).to_response();
        }
    };

    tokio::spawn(async move {
        while let Some(price) = rx.recv().await {
            tracing::info!(
                exchange = %price.exchange,
                pair = %price.pair,
                bid = price.bid,
                ask = price.ask,
                "price update"
            );
        }
    });

    {
        let mut clients = state.clients.lock().await;
        clients.insert(key, abort);
    }

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

    fn state_without_adapters() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            ticker_repository: None,
            exchange_adapters: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[actix_web::test]
    async fn ticker_returns_400_when_exchange_not_supported() {
        let app = test::init_service(
            App::new()
                .app_data(state_without_adapters())
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn ticker_returns_exchange_not_supported_message() {
        let app = test::init_service(
            App::new()
                .app_data(state_without_adapters())
                .route("/ticker", web::post().to(start_kline_symbol_ticker)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/ticker")
            .set_json(SymbolRequest {
                exchange: "tabdeal".to_string(),
                symbol: "USDTIRT".to_string(),
            })
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["success"], false);
        assert_eq!(body["message"], "Exchange not supported");
    }
}
