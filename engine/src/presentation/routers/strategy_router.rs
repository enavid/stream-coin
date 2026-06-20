use actix_web::web;

use crate::presentation::handlers::strategy_handler;

pub fn strategy_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/strategies")
            .route("/start", web::post().to(strategy_handler::start_strategy))
            .route("/stop", web::post().to(strategy_handler::stop_strategy))
            .route(
                "/register",
                web::post().to(strategy_handler::register_strategy),
            )
            .route("", web::get().to(strategy_handler::list_strategies)),
    );
}
