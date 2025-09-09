use actix_web::web;
use crate::presentation::handlers::exchange_handler;

pub fn exchange_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchanges")
            .route("/names", web::get().to(exchange_handler::get_exchange_names))
    );
}
