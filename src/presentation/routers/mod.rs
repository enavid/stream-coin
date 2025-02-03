mod health_router;
use actix_web::web;
mod exchange_routers;


pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/v1")
            .configure(health_router::health_router)
            .configure(exchange_routers::exchange_router)
    );
}