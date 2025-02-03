use actix_web::web;
mod user_routes;
mod health_router;
mod exchange_routers;


pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/v1")
            .configure(user_routes::user_router)
            .configure(health_router::health_router)
            .configure(exchange_routers::exchange_router)
    );
}