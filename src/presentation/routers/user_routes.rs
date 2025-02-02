use actix_web::{web, Responder};
use crate::presentation::handlers::{get_user, create_user};

pub fn user_router(cfg: &mut web::ServiceConfig) {
    cfg.route("/users/{id}", web::get().to(get_user));
    cfg.route("/users", web::get().to(create_user));
}


