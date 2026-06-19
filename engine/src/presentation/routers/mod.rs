mod exchange_routers;
mod health_router;
mod registry_router;

use actix_web::web;

use crate::presentation::handlers::ws_handler;

pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/v1")
            .configure(health_router::health_router)
            .configure(exchange_routers::exchange_router)
            .configure(registry_router::registry_router)
            .route("/ws", web::get().to(ws_handler::ws_index)),
    );
}
