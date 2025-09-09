use serde::Serialize;
use serde::Deserialize;
use validator::Validate;


#[derive(Debug, Deserialize, Validate)]
pub struct TestRequest {
    #[validate(length(min = 1))]
    pub name: String,

    #[validate(range(min = 1, max = 120))]
    pub age: u8,
}

#[derive(Debug, Serialize)]
pub struct TestResponse {
    pub message: String,
}
