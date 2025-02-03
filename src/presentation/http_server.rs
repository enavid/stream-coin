use actix_web_validator::JsonConfig;
use crate::presentation::routers::init_routes;
// use crate::application::services::ExchangeService;
use actix_web::{web, App, HttpServer, HttpResponse, middleware::Logger};

pub async fn start_server(
    host: String,
    port: String,
    json_config: JsonConfig,
    // exchange_service: web::Data<ExchangeService>,
) -> std::io::Result<()> {

    HttpServer::new(move || {
        App::new()
            .configure(init_routes)
            .wrap(Logger::default())
            .app_data(json_config.clone())
            // .app_data(web::Data::new(exchange_service.clone()))
            .default_service(
                web::route().to(|| async {
                    HttpResponse::NotFound().json("Not Found")
                }),
            )
    })
        .bind(format!("{}:{}", host, port))?
        .run()
        .await
}
