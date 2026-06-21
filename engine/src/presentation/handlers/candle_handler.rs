use actix_web::{web, HttpResponse};

use crate::presentation::dto::candle::CandleHistoryQuery;
use crate::presentation::responses::success_response;
use crate::presentation::shared::app_state::AppState;

/// `GET /v1/candles?exchange=&pair=&interval=&limit=` — recent candle history
/// for the live chart page, oldest first. Backed by `AppState::candle_history`
/// (an in-process ring buffer populated as candles close), not
/// `candle_repository` — there is no persistent candle store wired in yet, so
/// history is only as deep as `CANDLE_HISTORY_CAPACITY` and resets on restart.
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
