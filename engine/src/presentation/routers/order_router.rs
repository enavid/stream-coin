use actix_web::web;

use crate::presentation::handlers::order_handler;

pub fn order_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/orders")
            .route("/place", web::post().to(order_handler::place_order))
            .route("/cancel", web::post().to(order_handler::cancel_order))
            .route("", web::get().to(order_handler::list_orders)),
    );
    cfg.service(web::scope("/admin").route(
        "/circuit-breaker/reset",
        web::post().to(order_handler::reset_circuit_breaker),
    ));
}
