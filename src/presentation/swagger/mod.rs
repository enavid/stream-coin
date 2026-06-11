use utoipa::openapi::OpenApi as OpenApiSpec;
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(title = "stream-coin", version = "0.1.0"),
    modifiers(&StripInfo),
    paths(),
    components(),
    tags(
        (name = "Exchanges", description = "APIs for retrieving exchange data")
    )
)]
pub struct ApiDoc;

struct StripInfo;

impl Modify for StripInfo {
    fn modify(&self, openapi: &mut OpenApiSpec) {
        openapi.info.contact = None;
        openapi.info.license = None;
        openapi.info.description = None;
    }
}
