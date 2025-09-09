use actix_web::web;
use crate::presentation::handlers::exchange_handler;

pub fn exchange_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchanges/futures")
            .route("/start_kline_symbol_tricker", web::get().to(exchange_handler::start_kline_symbol_tricker))
    );
}
