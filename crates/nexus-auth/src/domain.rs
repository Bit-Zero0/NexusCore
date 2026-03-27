use serde::Serialize;

use nexus_shared::UserId;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    User,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Suspended,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    pub user_id: UserId,
    pub username: String,
    pub display_name: String,
    pub role: UserRole,
    pub status: UserStatus,
}
