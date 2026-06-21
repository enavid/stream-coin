use actix_web::web;

use crate::presentation::handlers::auth_handler;

pub fn auth_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth")
            .route("/token", web::post().to(auth_handler::login))
            .route("/refresh", web::post().to(auth_handler::refresh)),
    );
}
