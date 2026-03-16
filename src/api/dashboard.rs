use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/dashboard", get(dashboard_data))
}

async fn dashboard_data(State(state): State<AppState>) -> impl IntoResponse {
    let now = crate::unix_now();

    let (total_nodes,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM nodes")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (online_nodes,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM nodes WHERE online = 1")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (total_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (running_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'running'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (creating_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'creating'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (suspended_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'suspended'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (failed_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'failed'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (total_images,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM images")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (total_users,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (pending_tasks,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE state IN ('pending', 'running')")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    // Nodes list
    let nodes: Vec<crate::models::Node> =
        sqlx::query_as("SELECT * FROM nodes ORDER BY name LIMIT 20")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    let nodes_json: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let heartbeat_ago = n
                .last_heartbeat_at
                .map(|hb| now - hb)
                .unwrap_or(-1);
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "host_os": n.host_os,
                "host_arch": n.host_arch,
                "cpu_cores": n.cpu_cores,
                "memory_gb": n.memory_bytes.map(|b| b / 1_073_741_824),
                "disk_gb": n.disk_bytes_total.map(|b| b / 1_073_741_824),
                "online": n.online == Some(1),
                "heartbeat_ago_secs": heartbeat_ago,
            })
        })
        .collect();

    // Recent environments
    let envs: Vec<crate::models::Environment> = sqlx::query_as(
        "SELECT * FROM environments ORDER BY created_at DESC LIMIT 20",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // For running libvirt envs, try to get VNC/SSH info
    let mut envs_json: Vec<serde_json::Value> = Vec::new();
    for e in &envs {
        let ttl_remaining = e.expires_at.map(|exp| exp - now);
        let mut entry = serde_json::json!({
            "id": e.id,
            "state": e.state,
            "provider": e.provider,
            "guest_os": e.guest_os,
            "guest_arch": e.guest_arch,
            "vcpus": e.vcpus,
            "memory_gb": e.memory_bytes.map(|b| b / 1_073_741_824),
            "disk_gb": e.disk_bytes.map(|b| b / 1_073_741_824),
            "owner_user_id": e.owner_user_id,
            "node_id": e.node_id,
            "ttl_remaining_secs": ttl_remaining,
            "created_at": e.created_at,
        });

        if e.state.as_deref() == Some("running") {
            let provider_id = format!("libvirt-{}", e.id);
            if let Ok(info) = state.vm_provider.get_vm_info(&provider_id).await {
                entry["vnc_host"] = serde_json::json!(info.vnc_host);
                entry["vnc_port"] = serde_json::json!(info.vnc_port);
                entry["ssh_host"] = serde_json::json!(info.ssh_host);
                entry["ssh_port"] = serde_json::json!(info.ssh_port);
                entry["ssh_user"] = serde_json::json!("ubuntu");
                entry["ssh_password"] = serde_json::json!("orion");
            }
        }

        envs_json.push(entry);
    }

    // Recent events
    let events: Vec<crate::models::EnvironmentEvent> = sqlx::query_as(
        "SELECT * FROM environment_events ORDER BY created_at DESC LIMIT 20",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let events_json: Vec<serde_json::Value> = events
        .iter()
        .map(|ev| {
            let ago = now - ev.created_at;
            serde_json::json!({
                "env_id": ev.env_id,
                "event_type": ev.event_type,
                "from_state": ev.from_state,
                "to_state": ev.to_state,
                "triggered_by": ev.triggered_by,
                "message": ev.message,
                "ago_secs": ago,
            })
        })
        .collect();

    axum::Json(serde_json::json!({
        "timestamp": now,
        "summary": {
            "nodes_total": total_nodes,
            "nodes_online": online_nodes,
            "environments_total": total_envs,
            "environments_running": running_envs,
            "environments_creating": creating_envs,
            "environments_suspended": suspended_envs,
            "environments_failed": failed_envs,
            "images": total_images,
            "users": total_users,
            "tasks_active": pending_tasks,
        },
        "nodes": nodes_json,
        "environments": envs_json,
        "events": events_json,
    }))
}
