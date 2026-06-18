use crate::presentation::handlers::exchange_handler;
use actix_web::web;

pub fn exchange_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchanges/futures")
            .route(
                "/start_kline_symbol_ticker",
                web::post().to(exchange_handler::start_kline_symbol_ticker),
            )
            .route(
                "/stop_kline_symbol_ticker",
                web::post().to(exchange_handler::stop_kline_symbol_ticker),
            )
            .route("/tickers", web::get().to(exchange_handler::list_tickers)),
    );
}
