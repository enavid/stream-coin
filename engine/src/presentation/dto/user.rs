use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    pub id: i32,
    pub username: String,
    pub roles: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserListResponse {
    pub users: Vec<UserResponse>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignRolesRequest {
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRoleRequest {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleResponse {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleListResponse {
    pub roles: Vec<RoleResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PermissionListResponse {
    pub permissions: Vec<String>,
}
