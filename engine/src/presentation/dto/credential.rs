use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialSummaryResponse {
    pub exchange: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialListResponse {
    pub credentials: Vec<CredentialSummaryResponse>,
}
