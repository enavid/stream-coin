mod user_routes;
use actix_web::web;
use user_routes::user_router;

pub fn init_routes(cfg: &mut web::ServiceConfig) {
    user_router(cfg);
}