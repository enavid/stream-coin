use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(),
    components(),
    tags(
        (name = "Exchanges", description = "APIs for retrieving exchange data")
    )
)]
pub struct ApiDoc;
