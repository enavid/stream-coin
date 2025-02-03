use actix_web::web;
use crate::presentation::handlers::exchange_handler;

pub fn exchange_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchange")
            .route("/connect", web::post().to(exchange_handler::connect_websocket))
            .route("/disconnect", web::post().to(exchange_handler::disconnect_websocket))
    );
}
