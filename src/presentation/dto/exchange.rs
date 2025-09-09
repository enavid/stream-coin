use utoipa::ToSchema;
use serde::{Serialize};


#[derive(Serialize, ToSchema)]
pub struct ExchangeNameList {
    pub names: Vec<String>,
}
