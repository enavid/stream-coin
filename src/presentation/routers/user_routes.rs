use actix_web::web;
use crate::presentation::handlers::user_handler;

pub fn user_router(cfg: &mut web::ServiceConfig){
    cfg.service(
        web::scope("/users")
            .route("/{id}", web::get().to(user_handler::get_user))
            .route("", web::post().to(user_handler::create_user))
    );
}

