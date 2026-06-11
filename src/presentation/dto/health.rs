use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct Dependencies {
    pub redis: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct HealthStatus {
    pub name: &'static str,
    pub version: &'static str,
    pub status: &'static str,
    pub dependencies: Dependencies,
}
