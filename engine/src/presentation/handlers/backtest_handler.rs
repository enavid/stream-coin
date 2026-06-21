use actix_web::{web, HttpResponse};

use crate::backtest::engine::{BacktestEngine, BacktestError};
use crate::backtest::venue::FillModel;
use crate::presentation::dto::backtest::BacktestRunRequest;
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

/// `POST /v1/backtest/run` — replay historical candles through a deployed Python
/// strategy and return total return, max drawdown, trade log, and signal log.
///
/// The strategy subprocess uses the same launcher script (including the seccomp
/// preamble on Linux) as the live deployment path, so backtest and live behaviour
/// are guaranteed to be identical.
pub async fn run_backtest(
    state: web::Data<AppState>,
    body: web::Json<BacktestRunRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    if req.strategy_id.is_empty() {
        return ApiError::new("strategy_id must not be empty", vec![]).to_response();
    }
    if req.from >= req.to {
        return ApiError::new("'from' must be before 'to'", vec![]).to_response();
    }

    // Load the strategy code from the repository.
    let code = match &state.python_strategy_repository {
        None => {
            return ApiError::new(
                "no python strategy repository configured — cannot load strategy code",
                vec![],
            )
            .to_response();
        }
        Some(repo) => match repo.get(&req.strategy_id).await {
            Ok(record) => record.code,
            Err(_) => {
                return ApiError::new(&format!("strategy '{}' not found", req.strategy_id), vec![])
                    .to_response();
            }
        },
    };

    // Load historical candles for the requested time range.
    let candles = match &state.candle_repository {
        None => {
            return ApiError::new(
                "no candle repository configured — cannot load historical data",
                vec![],
            )
            .to_response();
        }
        Some(repo) => {
            match repo
                .list_candles(&req.exchange, &req.pair, &req.interval, req.from, req.to)
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        strategy_id = %req.strategy_id,
                        exchange = %req.exchange,
                        pair = %req.pair,
                        interval = %req.interval,
                        "failed to load candles for backtest"
                    );
                    return ApiError::new("failed to load historical candles", vec![])
                        .to_response();
                }
            }
        }
    };

    if candles.is_empty() {
        return ApiError::new(
            "no candles found for the specified exchange/pair/interval/range",
            vec![],
        )
        .to_response();
    }

    tracing::info!(
        strategy_id = %req.strategy_id,
        exchange = %req.exchange,
        pair = %req.pair,
        interval = %req.interval,
        from = %req.from,
        to = %req.to,
        candle_count = candles.len(),
        "starting backtest"
    );

    let engine = BacktestEngine::new(req.strategy_id.clone(), code, FillModel::LastClose);
    let result = match engine.run(&candles).await {
        Ok(r) => r,
        Err(BacktestError::ScriptWrite(e)) => {
            tracing::error!(error = %e, strategy_id = %req.strategy_id, "backtest script write failed");
            return ApiError::new("failed to write strategy script to disk", vec![]).to_response();
        }
        Err(BacktestError::SubprocessSpawn(e)) => {
            tracing::error!(error = %e, strategy_id = %req.strategy_id, "backtest subprocess spawn failed");
            return ApiError::new("failed to launch python3 — is it installed?", vec![])
                .to_response();
        }
    };

    tracing::info!(
        strategy_id = %req.strategy_id,
        exchange = %req.exchange,
        pair = %req.pair,
        signal_count = result.signal_count,
        trade_count = result.trade_log.len(),
        total_return_pct = result.total_return_pct,
        max_drawdown_pct = result.max_drawdown_pct,
        "backtest finished"
    );

    success_response("Backtest complete", result)
}
