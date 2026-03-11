use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};

use crate::AppState;
use crate::auth::{AdminUser, AuthUser};
use crate::error::AppError;
use crate::models::{Node, RegisterNodeRequest, UpdateNodeRequest};

use super::PaginationParams;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/nodes", get(list_nodes))
        .route("/v1/nodes", post(register_node))
        .route("/v1/nodes/{node_id}", get(get_node))
        .route("/v1/nodes/{node_id}", put(update_node))
        .route(
            "/v1/nodes/{node_id}",
            axum::routing::delete(delete_node),
        )
        .route("/v1/nodes/{node_id}/heartbeat", post(heartbeat))
        .route("/v1/nodes/{node_id}/usage", get(node_usage))
}

async fn list_nodes(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<Node>>, AppError> {
    let nodes = sqlx::query_as::<_, Node>(
        "SELECT * FROM nodes ORDER BY name LIMIT ? OFFSET ?",
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(nodes))
}

async fn get_node(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(node_id): Path<String>,
) -> Result<Json<Node>, AppError> {
    let node = sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(node))
}

async fn register_node(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(req): Json<RegisterNodeRequest>,
) -> Result<(StatusCode, Json<Node>), AppError> {
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO nodes (id, name, host_os, host_arch, cpu_cores, memory_bytes, disk_bytes_total,
         max_cpu_utilization_ratio, max_memory_utilization_ratio, max_disk_utilization_ratio,
         max_running_envs, online)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&req.host_os)
    .bind(&req.host_arch)
    .bind(req.cpu_cores)
    .bind(req.memory_bytes)
    .bind(req.disk_bytes_total)
    .bind(req.max_cpu_utilization_ratio.unwrap_or(0.7))
    .bind(req.max_memory_utilization_ratio.unwrap_or(0.6))
    .bind(req.max_disk_utilization_ratio.unwrap_or(0.8))
    .bind(req.max_running_envs.unwrap_or(8))
    .execute(&state.db)
    .await?;

    let node = sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(node)))
}

async fn update_node(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(node_id): Path<String>,
    Json(req): Json<UpdateNodeRequest>,
) -> Result<Json<Node>, AppError> {
    // Verify exists
    sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_one(&state.db)
        .await?;

    if let Some(name) = &req.name {
        sqlx::query("UPDATE nodes SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&node_id)
            .execute(&state.db)
            .await?;
    }
    if let Some(v) = req.max_cpu_utilization_ratio {
        sqlx::query("UPDATE nodes SET max_cpu_utilization_ratio = ? WHERE id = ?")
            .bind(v)
            .bind(&node_id)
            .execute(&state.db)
            .await?;
    }
    if let Some(v) = req.max_memory_utilization_ratio {
        sqlx::query("UPDATE nodes SET max_memory_utilization_ratio = ? WHERE id = ?")
            .bind(v)
            .bind(&node_id)
            .execute(&state.db)
            .await?;
    }
    if let Some(v) = req.max_disk_utilization_ratio {
        sqlx::query("UPDATE nodes SET max_disk_utilization_ratio = ? WHERE id = ?")
            .bind(v)
            .bind(&node_id)
            .execute(&state.db)
            .await?;
    }
    if let Some(v) = req.max_running_envs {
        sqlx::query("UPDATE nodes SET max_running_envs = ? WHERE id = ?")
            .bind(v)
            .bind(&node_id)
            .execute(&state.db)
            .await?;
    }
    if let Some(v) = req.online {
        sqlx::query("UPDATE nodes SET online = ? WHERE id = ?")
            .bind(v)
            .bind(&node_id)
            .execute(&state.db)
            .await?;
    }

    let node = sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(node))
}

async fn delete_node(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(node_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let active_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM environments WHERE node_id = ? AND state NOT IN ('failed', 'destroying')",
    )
    .bind(&node_id)
    .fetch_one(&state.db)
    .await?;

    if active_count.0 > 0 {
        return Err(AppError::BadRequest(format!(
            "node {node_id} still has {0} active environments",
            active_count.0
        )));
    }

    let result = sqlx::query("DELETE FROM nodes WHERE id = ?")
        .bind(&node_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("node {node_id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn node_usage(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(node_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let node = sqlx::query_as::<_, Node>("SELECT * FROM nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("node {node_id} not found")))?;

    let (env_count, used_vcpus, used_memory, used_disk): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),
                COALESCE(SUM(COALESCE(vcpus, 0)), 0),
                COALESCE(SUM(COALESCE(memory_bytes, 0)), 0),
                COALESCE(SUM(COALESCE(disk_bytes, 0)), 0)
         FROM environments
         WHERE node_id = ? AND state IN ('creating', 'running', 'suspending', 'resuming', 'rebooting')"
    )
    .bind(&node_id)
    .fetch_one(&state.db)
    .await?;

    let cpu_capacity = (node.cpu_cores.unwrap_or(0) as f64)
        * node.max_cpu_utilization_ratio.unwrap_or(0.7);
    let memory_capacity = (node.memory_bytes.unwrap_or(0) as f64)
        * node.max_memory_utilization_ratio.unwrap_or(0.6);
    let disk_capacity = (node.disk_bytes_total.unwrap_or(0) as f64)
        * node.max_disk_utilization_ratio.unwrap_or(0.8);

    Ok(Json(serde_json::json!({
        "node_id": node_id,
        "environment_count": env_count,
        "max_environments": node.max_running_envs,
        "vcpus_used": used_vcpus,
        "vcpus_capacity": cpu_capacity as i64,
        "memory_bytes_used": used_memory,
        "memory_bytes_capacity": memory_capacity as i64,
        "disk_bytes_used": used_disk,
        "disk_bytes_capacity": disk_capacity as i64,
    })))
}

async fn heartbeat(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(node_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let now = crate::unix_now();

    let result = sqlx::query(
        "UPDATE nodes SET last_heartbeat_at = ?, online = 1 WHERE id = ?",
    )
    .bind(now)
    .bind(&node_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("node {node_id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}
