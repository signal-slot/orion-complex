use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{get, put};

use crate::AppState;
use crate::auth::{AdminUser, AuthUser};
use crate::error::AppError;
use crate::models::{SetUserDisabledRequest, UpdateUserRoleRequest, User};

use super::PaginationParams;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/users", get(list_users))
        .route("/v1/users/{user_id}", get(get_user))
        .route("/v1/users/{user_id}/role", put(update_role))
        .route("/v1/users/{user_id}/disabled", put(set_disabled))
}

async fn list_users(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<User>>, AppError> {
    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

async fn get_user(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(&user_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(user))
}

async fn update_role(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRoleRequest>,
) -> Result<Json<User>, AppError> {
    if !["admin", "user"].contains(&req.role.as_str()) {
        return Err(AppError::BadRequest(format!(
            "invalid role '{}', must be 'admin' or 'user'",
            req.role
        )));
    }

    let result = sqlx::query("UPDATE users SET role = ? WHERE id = ?")
        .bind(&req.role)
        .bind(&user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {user_id} not found")));
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(&user_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(user))
}

async fn set_disabled(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
    Json(req): Json<SetUserDisabledRequest>,
) -> Result<Json<User>, AppError> {
    let disabled_val: i64 = if req.disabled { 1 } else { 0 };

    let result = sqlx::query("UPDATE users SET disabled = ? WHERE id = ?")
        .bind(disabled_val)
        .bind(&user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {user_id} not found")));
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(&user_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(user))
}
