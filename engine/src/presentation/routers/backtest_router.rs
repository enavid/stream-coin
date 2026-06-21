use actix_web::web;

use crate::presentation::handlers::backtest_handler;

pub fn backtest_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/backtest").route("/run", web::post().to(backtest_handler::run_backtest)),
    );
}
