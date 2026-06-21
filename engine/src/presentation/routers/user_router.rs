use actix_web::web;

use crate::presentation::handlers::{order_handler, user_handler};

/// All `/admin/*` routes live in this single scope — actix-web only routes to the
/// first-registered scope sharing an exact path prefix, so a second `web::scope("/admin")`
/// anywhere else in the router tree would silently become unreachable.
pub fn user_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/admin")
            .route("/users", web::post().to(user_handler::create_user))
            .route("/users", web::get().to(user_handler::list_users))
            .route(
                "/users/{id}/roles",
                web::post().to(user_handler::assign_user_roles),
            )
            .route("/roles", web::get().to(user_handler::list_roles))
            .route("/roles", web::post().to(user_handler::create_role))
            .route(
                "/permissions",
                web::get().to(user_handler::list_permissions),
            )
            .route(
                "/circuit-breaker/reset",
                web::post().to(order_handler::reset_circuit_breaker),
            ),
    );
}
