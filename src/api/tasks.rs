use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::get;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::Task;

use super::PaginationParams;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/tasks", get(list_tasks))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route(
            "/v1/environments/{env_id}/tasks",
            get(list_tasks_for_env),
        )
}

async fn list_tasks(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<Task>>, AppError> {
    let tasks = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(tasks))
}

async fn get_task(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<Task>, AppError> {
    let task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
        .bind(&task_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(task))
}

async fn list_tasks_for_env(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(env_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<Task>>, AppError> {

    let tasks = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE env_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(&env_id)
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(tasks))
}
