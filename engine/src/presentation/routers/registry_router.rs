use actix_web::web;

use crate::presentation::handlers::registry_handler;

pub fn registry_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchanges")
            .route("", web::get().to(registry_handler::list_exchanges))
            .route(
                "/{name}/pairs",
                web::get().to(registry_handler::list_exchange_pairs),
            ),
    )
    .service(
        web::scope("/admin/exchanges")
            .route("/enable", web::post().to(registry_handler::enable_exchange))
            .route(
                "/disable",
                web::post().to(registry_handler::disable_exchange),
            )
            .route(
                "/{name}/credentials",
                web::post().to(registry_handler::set_exchange_credentials),
            ),
    );
}
