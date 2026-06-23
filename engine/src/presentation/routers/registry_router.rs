use actix_web::web;

use crate::presentation::handlers::{credential_handler, registry_handler};

pub fn registry_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchanges")
            .route("", web::get().to(registry_handler::list_exchanges))
            .route(
                "/{name}/pairs",
                web::get().to(registry_handler::list_exchange_pairs),
            )
            .route(
                "/credentials",
                web::get().to(credential_handler::list_own_credentials),
            )
            .route(
                "/{name}/credentials",
                web::post().to(credential_handler::set_own_credentials),
            )
            .route(
                "/{name}/credentials",
                web::delete().to(credential_handler::delete_own_credentials),
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
                "/coinex/seed-top-pairs",
                web::post().to(registry_handler::seed_coinex_top_pairs),
            ),
    );
}
