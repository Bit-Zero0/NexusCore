use nexus_shared::{AppError, AppResult};

use crate::AuthenticatedUser;

pub trait AuthService: Send + Sync {
    fn authenticate(&self, bearer_token: Option<&str>) -> AppResult<AuthenticatedUser>;
}

pub struct DevAuthService {
    expected_token: String,
    user: AuthenticatedUser,
}

impl DevAuthService {
    pub fn new(expected_token: impl Into<String>, user: AuthenticatedUser) -> Self {
        Self {
            expected_token: expected_token.into(),
            user,
        }
    }
}

impl AuthService for DevAuthService {
    fn authenticate(&self, bearer_token: Option<&str>) -> AppResult<AuthenticatedUser> {
        match bearer_token {
            Some(token) if token == self.expected_token => Ok(self.user.clone()),
            _ => Err(AppError::Unauthorized),
        }
    }
}
