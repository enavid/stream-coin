use actix_web::web;

use crate::presentation::handlers::subscription_handler;

pub fn subscription_router(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/subscriptions")
            .route("", web::post().to(subscription_handler::subscribe))
            .route("", web::get().to(subscription_handler::list_subscriptions))
            .route(
                "/{id}",
                web::patch().to(subscription_handler::update_subscription),
            )
            .route(
                "/{id}",
                web::delete().to(subscription_handler::delete_subscription),
            ),
    );
}
