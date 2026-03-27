mod domain;
mod service;

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, routing::get, Json, Router};
pub use domain::{AuthenticatedUser, UserRole, UserStatus};
use nexus_shared::{AppResult, UserId};
pub use service::{AuthService, DevAuthService};

#[derive(Clone)]
struct AuthState {
    service: Arc<dyn AuthService>,
}

pub fn build_router(service: Arc<dyn AuthService>) -> Router {
    Router::new()
        .route("/api/v1/auth/me", get(current_user))
        .with_state(AuthState { service })
}

async fn current_user(
    headers: HeaderMap,
    State(state): State<AuthState>,
) -> AppResult<Json<AuthenticatedUser>> {
    let authorization = extract_bearer_token(&headers);
    let user = state.service.authenticate(authorization.as_deref())?;
    Ok(Json(user))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value.strip_prefix("Bearer ").map(str::to_owned)
}

pub fn build_dev_auth_service() -> Arc<dyn AuthService> {
    Arc::new(DevAuthService::new(
        "dev-token",
        AuthenticatedUser {
            user_id: UserId::from("u-dev-admin"),
            username: "dev_admin".to_owned(),
            display_name: "Dev Admin".to_owned(),
            role: UserRole::Admin,
            status: UserStatus::Active,
        },
    ))
}
