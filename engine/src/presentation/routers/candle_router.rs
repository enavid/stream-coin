use actix_web::web;

use crate::presentation::handlers::candle_handler;

pub fn candle_router(cfg: &mut web::ServiceConfig) {
    cfg.route("/candles", web::get().to(candle_handler::get_candles));
    cfg.route(
        "/candles/backfill",
        web::post().to(candle_handler::backfill_candles),
    );
}
