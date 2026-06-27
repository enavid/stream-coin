use actix_web::{web, HttpResponse};

use crate::presentation::dto::candle::{BackfillRequest, BackfillResponse, CandleHistoryQuery};
use crate::presentation::responses::{success_response, ApiError, FieldError};
use crate::presentation::shared::app_state::AppState;
use crate::price::entity::TradingPair;

/// `GET /v1/candles?exchange=&pair=&interval=&limit=` — recent candle history
/// for the live chart page, oldest first. Backed by `AppState::candle_history`
/// (an in-process ring buffer populated as candles close), not
/// `candle_repository` — there is no persistent candle store wired in yet, so
/// history is only as deep as `CANDLE_HISTORY_CAPACITY` and resets on restart.
#[utoipa::path(
    get,
    path = "/v1/candles",
    tag = "Candles",
    params(
        ("exchange" = String, Query, description = "Exchange name"),
        ("pair" = String, Query, description = "Trading pair, e.g. USDT/IRT"),
        ("interval" = String, Query, description = "Candle interval, e.g. 1m"),
        ("limit" = Option<u32>, Query, description = "Max candles to return (default 300, max 1000)")
    ),
    responses(
        (status = 200, description = "Candle history (oldest first)", body = [crate::candle::entity::CandlePayload])
    )
)]
pub async fn get_candles(
    state: web::Data<AppState>,
    query: web::Query<CandleHistoryQuery>,
) -> HttpResponse {
    let limit = query.resolved_limit();
    let candles = state
        .recent_candles(&query.exchange, &query.pair, &query.interval, limit)
        .await;
    success_response("candle history", candles)
}

/// `POST /v1/candles/backfill` — fetches historical klines from the
/// exchange's `HistoricalCandleSource` (if one is registered) and persists
/// them via `CandleRepository::upsert_candles`. A separate explicit step
/// from `POST /v1/backtest/run` on purpose: a user re-running the same
/// backtest while tuning params must not silently re-fetch from the
/// exchange every time.
#[utoipa::path(
    post,
    path = "/v1/candles/backfill",
    tag = "Candles",
    request_body = BackfillRequest,
    responses(
        (status = 200, description = "Backfill complete", body = BackfillResponse),
        (status = 400, description = "Invalid range, pair, or no historical source", body = ApiError),
        (status = 503, description = "Upstream exchange unavailable or no candle repository", body = ApiError)
    )
)]
pub async fn backfill_candles(
    state: web::Data<AppState>,
    body: web::Json<BackfillRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    if req.from >= req.to {
        return ApiError::new("'from' must be before 'to'", vec![]).to_response();
    }

    let Some((base, quote)) = req
        .pair
        .split_once('/')
        .filter(|(b, q)| !b.is_empty() && !q.is_empty())
    else {
        return ApiError::new(
            "Validation failed",
            vec![FieldError::new("pair", "must be BASE/QUOTE format")],
        )
        .to_response();
    };
    let trading_pair = TradingPair::new(base, quote);

    let Some(source) = state.historical_sources.get(&req.exchange) else {
        tracing::warn!(
            exchange = %req.exchange,
            pair = %req.pair,
            "backfill rejected: exchange has no registered historical candle source"
        );
        return ApiError::new(
            &format!(
                "exchange '{}' has no historical candle source",
                req.exchange
            ),
            vec![],
        )
        .to_response();
    };

    let Some(repo) = &state.candle_repository else {
        return ApiError::service_unavailable(
            "no candle repository configured — cannot persist backfilled candles",
        )
        .to_response();
    };

    tracing::info!(
        exchange = %req.exchange,
        pair = %req.pair,
        interval = req.interval.as_str(),
        from = %req.from,
        to = %req.to,
        "starting candle backfill"
    );

    let candles = match source
        .fetch_klines(&trading_pair, req.interval, req.from, req.to)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                exchange = %req.exchange,
                pair = %req.pair,
                error = %e,
                transient = e.is_transient(),
                "candle backfill fetch failed"
            );
            return if e.is_transient() {
                ApiError::service_unavailable(&format!("upstream exchange unavailable: {e}"))
            } else {
                ApiError::new(&format!("backfill rejected by exchange: {e}"), vec![])
            }
            .to_response();
        }
    };

    if let Err(e) = repo.upsert_candles(&candles).await {
        tracing::error!(
            exchange = %req.exchange,
            pair = %req.pair,
            error = %e,
            "failed to persist backfilled candles"
        );
        return ApiError::new("failed to persist backfilled candles", vec![]).to_response();
    }

    tracing::info!(
        exchange = %req.exchange,
        pair = %req.pair,
        interval = req.interval.as_str(),
        candles_written = candles.len(),
        "candle backfill complete"
    );

    success_response(
        "Backfill complete",
        BackfillResponse {
            candles_written: candles.len(),
        },
    )
}
