use utoipa::OpenApi;
use actix_web::web::Data;
use utoipa_swagger_ui::SwaggerUi;
use actix_web_validator::JsonConfig;
use crate::presentation::swagger::ApiDoc;
use crate::presentation::routers::init_routes;
use crate::infrastructure::persistence::database::maria_db::AppState;
use actix_web::{web, App, HttpServer, HttpResponse, middleware::Logger};


pub async fn start_server(
    host: String,
    port: String,
    app_state: Data<AppState>,
    json_config: JsonConfig,
) -> std::io::Result<()> {

    HttpServer::new(move || {
        App::new()
            .configure(init_routes)
            .wrap(Logger::default())
            .app_data(json_config.clone())
            .app_data(app_state.clone())
            .default_service(
                web::route().to(|| async {
                    HttpResponse::NotFound().json("Not Found")
                }),
            )
            .service(SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()))
    })
        .bind(format!("{}:{}", host, port))?
        .run()
        .await
}
