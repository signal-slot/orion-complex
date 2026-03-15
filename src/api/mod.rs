pub mod auth;
pub mod dashboard;
pub mod environments;
pub mod events;
pub mod health;
pub mod images;
pub mod nodes;
pub mod snapshots;
pub mod ssh_keys;
pub mod tasks;
pub mod users;
pub mod webauthn;
pub mod ws;

use axum::Router;
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;
use crate::models::Environment;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(dashboard::routes())
        .merge(health::routes())
        .merge(auth::routes())
        .merge(users::routes())
        .merge(nodes::routes())
        .merge(images::routes())
        .merge(environments::routes())
        .merge(ssh_keys::routes())
        .merge(snapshots::routes())
        .merge(tasks::routes())
        .merge(events::routes())
        .merge(webauthn::routes())
        .merge(ws::routes())
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

impl PaginationParams {
    pub fn offset(&self) -> i64 {
        self.offset.unwrap_or(0).max(0)
    }
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 200)
    }
}

pub async fn fetch_env(state: &AppState, env_id: &str) -> Result<Environment, AppError> {
    sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(env_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("environment {env_id} not found")))
}

/// Shared-resource system: all authenticated users can see and operate all environments.
pub fn check_env_owner(_user: &crate::models::User, _env: &Environment) -> Result<(), AppError> {
    Ok(())
}
