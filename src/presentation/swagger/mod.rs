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

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    use super::*;

    #[test]
    fn swagger_has_no_contact() {
        let api = ApiDoc::openapi();
        assert!(api.info.contact.is_none());
    }

    #[test]
    fn swagger_has_no_license() {
        let api = ApiDoc::openapi();
        assert!(api.info.license.is_none());
    }

    #[test]
    fn swagger_title_is_stream_coin() {
        let api = ApiDoc::openapi();
        assert_eq!(api.info.title, "stream-coin");
    }
}
