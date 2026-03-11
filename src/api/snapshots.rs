use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};

use crate::AppState;
use crate::api::environments::is_agent_managed;
use crate::api::{check_env_owner, fetch_env};
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::{CreateSnapshotRequest, Environment, Snapshot};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/environments/{env_id}/snapshots",
            get(list_snapshots),
        )
        .route(
            "/v1/environments/{env_id}/snapshots",
            post(create_snapshot),
        )
        .route(
            "/v1/environments/{env_id}/snapshots/{snapshot_id}",
            axum::routing::delete(delete_snapshot),
        )
        .route(
            "/v1/environments/{env_id}/snapshots/{snapshot_id}/restore",
            post(restore_snapshot),
        )
}

async fn list_snapshots(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Vec<Snapshot>>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let snapshots = sqlx::query_as::<_, Snapshot>("SELECT * FROM snapshots WHERE env_id = ?")
        .bind(&env_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(snapshots))
}

async fn create_snapshot(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    body: Option<Json<CreateSnapshotRequest>>,
) -> Result<(StatusCode, Json<Snapshot>), AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let current_state = env.state.as_deref().unwrap_or("");
    if !["running", "suspended"].contains(&current_state) {
        return Err(AppError::BadRequest(format!(
            "cannot snapshot environment in state '{current_state}'"
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let name = body.and_then(|b| b.0.name);
    let now = crate::unix_now();

    sqlx::query("INSERT INTO snapshots (id, env_id, name, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(&env_id)
        .bind(&name)
        .bind(now)
        .execute(&state.db)
        .await?;

    // Dispatch snapshot creation to VM provider
    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if !is_agent_managed(provider) {
        let provider_id = format!("libvirt-{}", env.id);
        if let Err(e) = state.vm_provider.create_snapshot(&provider_id, &id).await {
            // Clean up the DB record on failure
            let _ = sqlx::query("DELETE FROM snapshots WHERE id = ?")
                .bind(&id)
                .execute(&state.db)
                .await;
            return Err(AppError::Internal(e));
        }
    }

    let snapshot = sqlx::query_as::<_, Snapshot>("SELECT * FROM snapshots WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(snapshot)))
}

async fn delete_snapshot(
    State(state): State<AppState>,
    user: AuthUser,
    Path((env_id, snapshot_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    // Dispatch snapshot deletion to VM provider
    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if !is_agent_managed(provider) {
        let provider_id = format!("libvirt-{}", env.id);
        state
            .vm_provider
            .delete_snapshot(&provider_id, &snapshot_id)
            .await
            .map_err(AppError::Internal)?;
    }

    let result = sqlx::query("DELETE FROM snapshots WHERE id = ? AND env_id = ?")
        .bind(&snapshot_id)
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "snapshot {snapshot_id} not found"
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn restore_snapshot(
    State(state): State<AppState>,
    user: AuthUser,
    Path((env_id, snapshot_id)): Path<(String, String)>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let current_state = env.state.as_deref().unwrap_or("");
    if !["running", "suspended"].contains(&current_state) {
        return Err(AppError::BadRequest(format!(
            "cannot restore snapshot in state '{current_state}'"
        )));
    }

    // Verify snapshot exists
    let _snapshot = sqlx::query_as::<_, Snapshot>(
        "SELECT * FROM snapshots WHERE id = ? AND env_id = ?",
    )
    .bind(&snapshot_id)
    .bind(&env_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("snapshot {snapshot_id} not found")))?;

    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if !is_agent_managed(provider) {
        let provider_id = format!("libvirt-{}", env.id);
        state
            .vm_provider
            .restore_snapshot(&provider_id, &snapshot_id)
            .await
            .map_err(AppError::Internal)?;
    }

    crate::events::emit(
        &state.db,
        &env_id,
        "snapshot_restored",
        None,
        None,
        Some(&user.0.id),
        Some(&format!("restored snapshot {snapshot_id}")),
    )
    .await;

    let env = fetch_env(&state, &env_id).await?;
    Ok(Json(env))
}
