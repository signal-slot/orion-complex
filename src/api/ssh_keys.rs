use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use serde::Deserialize;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::UserSshKey;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/ssh-keys", get(list_keys))
        .route("/v1/ssh-keys", post(add_key))
        .route("/v1/ssh-keys/{key_id}", axum::routing::delete(delete_key))
        .route("/v1/users/{user_id}/ssh-keys", get(list_user_keys))
}

async fn list_keys(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<UserSshKey>>, AppError> {
    let keys =
        sqlx::query_as::<_, UserSshKey>("SELECT * FROM user_ssh_keys WHERE user_id = ?")
            .bind(&user.0.id)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(keys))
}

#[derive(Debug, Deserialize)]
struct AddKeyRequest {
    public_key: String,
}

async fn add_key(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<AddKeyRequest>,
) -> Result<(StatusCode, Json<UserSshKey>), AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    // Derive a simple fingerprint from the key (first 16 chars of the key body)
    let fingerprint = req
        .public_key
        .split_whitespace()
        .nth(1)
        .map(|body| {
            let len = body.len().min(16);
            format!("SHA256:{}", &body[..len])
        })
        .unwrap_or_default();

    sqlx::query(
        "INSERT INTO user_ssh_keys (id, user_id, public_key, fingerprint, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&user.0.id)
    .bind(&req.public_key)
    .bind(&fingerprint)
    .bind(now)
    .execute(&state.db)
    .await?;

    let key = sqlx::query_as::<_, UserSshKey>("SELECT * FROM user_ssh_keys WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(key)))
}

async fn delete_key(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let result =
        sqlx::query("DELETE FROM user_ssh_keys WHERE id = ? AND user_id = ?")
            .bind(&key_id)
            .bind(&user.0.id)
            .execute(&state.db)
            .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("ssh key {key_id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Provisioning endpoint: returns SSH keys for a given user.
/// Used by node agents to sync keys into guest VMs.
async fn list_user_keys(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<UserSshKey>>, AppError> {
    let keys =
        sqlx::query_as::<_, UserSshKey>("SELECT * FROM user_ssh_keys WHERE user_id = ?")
            .bind(&user_id)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(keys))
}
