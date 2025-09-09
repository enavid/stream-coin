use actix_web::web;
use crate::presentation::handlers::health_handler;

pub fn health_router(cfg: &mut web::ServiceConfig){
    cfg.service(
        web::scope("/check")
            .route("/health", web::get().to(health_handler::health))
    );
}
