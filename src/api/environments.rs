use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use serde::Deserialize;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::events;
use crate::models::{
    CaptureImageRequest, CreateEnvironmentRequest, Environment, ExtendTtlRequest, MigrateRequest,
    RenameEnvironmentRequest, UsbAttachment,
};
use crate::tasks;
use crate::vm::VmCreateParams;

use super::{PaginationParams, check_env_owner, fetch_env};

const DEFAULT_VCPUS: i64 = 2;
const DEFAULT_MEMORY_BYTES: i64 = 4 * 1024 * 1024 * 1024; // 4 GB
const DEFAULT_DISK_BYTES: i64 = 20 * 1024 * 1024 * 1024; // 20 GB

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/environments", get(list_environments))
        .route("/v1/environments", post(create_environment))
        .route("/v1/environments/{env_id}", get(get_environment))
        .route(
            "/v1/environments/{env_id}",
            axum::routing::delete(destroy_environment),
        )
        .route("/v1/environments/{env_id}/state", put(update_state))
        .route(
            "/v1/environments/{env_id}/ssh-endpoint",
            get(ssh_endpoint),
        )
        .route(
            "/v1/environments/{env_id}/vnc-endpoint",
            get(vnc_endpoint),
        )
        .route("/v1/environments/{env_id}/suspend", post(suspend))
        .route("/v1/environments/{env_id}/resume", post(resume))
        .route("/v1/environments/{env_id}/reboot", post(reboot))
        .route(
            "/v1/environments/{env_id}/force-reboot",
            post(force_reboot),
        )
        .route("/v1/environments/{env_id}/restart", post(restart))
        .route("/v1/environments/{env_id}/migrate", post(migrate))
        .route(
            "/v1/environments/{env_id}/extend-ttl",
            post(extend_ttl),
        )
        .route(
            "/v1/environments/{env_id}/port-forwarding",
            post(toggle_port_forwarding),
        )
        .route(
            "/v1/environments/{env_id}/endpoints",
            put(update_endpoints),
        )
        .route(
            "/v1/environments/{env_id}/name",
            put(rename_environment),
        )
        .route(
            "/v1/environments/{env_id}/capture-image",
            post(capture_image),
        )
        .route(
            "/v1/environments/{env_id}/iso-url",
            put(update_iso_url),
        )
}

/// Returns true if the provider is managed by an external node agent (macOS)
/// rather than the in-process VmProvider (libvirt).
pub fn is_agent_managed(provider: &str) -> bool {
    matches!(provider, "macos" | "virtualization")
}

/// Used by node agents to report state changes back to the control plane.
#[derive(Debug, Deserialize)]
struct UpdateStateRequest {
    state: String,
}

async fn update_state(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<UpdateStateRequest>,
) -> Result<Json<Environment>, AppError> {
    let valid_states = [
        "creating", "running", "suspending", "suspended", "resuming",
        "rebooting", "migrating", "destroying", "capturing", "failed",
    ];
    if !valid_states.contains(&req.state.as_str()) {
        return Err(AppError::BadRequest(format!(
            "invalid state '{}', must be one of: {}",
            req.state,
            valid_states.join(", ")
        )));
    }

    let env = fetch_env(&state, &env_id).await?;
    let old_state = env.state.clone();

    let result = sqlx::query("UPDATE environments SET state = ? WHERE id = ?")
        .bind(&req.state)
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "environment {env_id} not found"
        )));
    }

    events::emit(
        &state.db,
        &env_id,
        "state_change",
        old_state.as_deref(),
        Some(&req.state),
        Some("agent"),
        None,
    )
    .await;

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

async fn list_environments(
    State(state): State<AppState>,
    user: AuthUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<Environment>>, AppError> {
    // All users can see all environments — this system manages shared resources
    let envs = sqlx::query_as::<_, Environment>(
        "SELECT * FROM environments ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(envs))
}

async fn get_environment(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    Ok(Json(env))
}

async fn create_environment(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateEnvironmentRequest>,
) -> Result<(StatusCode, Json<Environment>), AppError> {
    // Determine provider/guest info from image or from explicit fields (ISO mode)
    let (image_id, iso_url, provider, guest_os, guest_arch) = if let Some(ref img_id) = req.image_id
    {
        let image =
            sqlx::query_as::<_, crate::models::Image>("SELECT * FROM images WHERE id = ?")
                .bind(img_id)
                .fetch_optional(&state.db)
                .await?
                .ok_or_else(|| AppError::BadRequest(format!("image {img_id} not found")))?;
        (
            Some(img_id.clone()),
            None::<String>,
            image.provider.unwrap_or_else(|| "libvirt".into()),
            image.guest_os.unwrap_or_else(|| "linux".into()),
            image.guest_arch.unwrap_or_else(|| "x86_64".into()),
        )
    } else if let Some(ref url) = req.iso_url {
        // ISO-based creation: no base image.
        // Default to "virtualization" (macOS agent) unless provider is explicitly set.
        (
            None,
            Some(url.clone()),
            req.provider.clone().unwrap_or_else(|| "virtualization".to_string()),
            req.guest_os.clone().unwrap_or_else(|| "windows".into()),
            req.guest_arch.clone().unwrap_or_else(|| "x86_64".into()),
        )
    } else {
        return Err(AppError::BadRequest(
            "either image_id or iso_url is required".into(),
        ));
    };

    let vcpus = req.vcpus.unwrap_or(DEFAULT_VCPUS);
    let memory_bytes = req.memory_bytes.unwrap_or(DEFAULT_MEMORY_BYTES);
    let disk_bytes = req.disk_bytes.unwrap_or(DEFAULT_DISK_BYTES);

    // Pick a node: use requested node or find one with capacity
    let node = match req.node_id {
        Some(ref id) => {
            let node =
                sqlx::query_as::<_, crate::models::Node>("SELECT * FROM nodes WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&state.db)
                    .await?
                    .ok_or_else(|| AppError::BadRequest(format!("node {id} not found")))?;
            if node.online != Some(1) {
                return Err(AppError::BadRequest(format!("node {id} is offline")));
            }
            node
        }
        None => {
            // Scheduler: pick first online node matching host_os + under capacity limits
            let host_os_filter = if is_agent_managed(&provider) {
                "macos"
            } else {
                "linux"
            };
            let is_agent: i32 = if is_agent_managed(&provider) { 1 } else { 0 };
            sqlx::query_as::<_, crate::models::Node>(
                "SELECT n.* FROM nodes n
                 WHERE n.online = 1
                   AND n.host_os = ?
                   AND (n.host_arch = ? OR (n.host_arch IN ('arm64', 'aarch64') AND ? IN ('arm64', 'aarch64'))
                        OR (? = 1 AND n.host_arch IN ('arm64', 'aarch64') AND ? = 'x86_64'))
                   AND (SELECT COUNT(*) FROM environments e
                        WHERE e.node_id = n.id AND e.state IN ('creating', 'running', 'suspending', 'resuming', 'rebooting'))
                       < COALESCE(n.max_running_envs, 9999)
                   AND COALESCE((SELECT SUM(COALESCE(e.vcpus, 0)) FROM environments e
                        WHERE e.node_id = n.id AND e.state IN ('creating', 'running', 'suspending', 'resuming', 'rebooting')), 0) + ?
                       <= n.cpu_cores * COALESCE(n.max_cpu_utilization_ratio, 2.0)
                   AND COALESCE((SELECT SUM(COALESCE(e.memory_bytes, 0)) FROM environments e
                        WHERE e.node_id = n.id AND e.state IN ('creating', 'running', 'suspending', 'resuming', 'rebooting')), 0) + ?
                       <= n.memory_bytes * COALESCE(n.max_memory_utilization_ratio, 0.95)
                   AND COALESCE((SELECT SUM(COALESCE(e.disk_bytes, 0)) FROM environments e
                        WHERE e.node_id = n.id AND e.state IN ('creating', 'running', 'suspending', 'resuming', 'rebooting')), 0) + ?
                       <= n.disk_bytes_total * COALESCE(n.max_disk_utilization_ratio, 0.95)
                 LIMIT 1",
            )
            .bind(host_os_filter)
            .bind(&guest_arch)
            .bind(&guest_arch)
            .bind(is_agent)
            .bind(&guest_arch)
            .bind(vcpus)
            .bind(memory_bytes)
            .bind(disk_bytes)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "no available {host_os_filter} nodes with sufficient capacity for provider '{provider}'"
                ))
            })?
        }
    };

    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    let expires_at = req.ttl_seconds.map(|ttl| now + ttl);

    // Generate a default name
    let name = req.name.unwrap_or_else(|| {
        if iso_url.is_some() {
            format!("{}-install-{}", guest_os, &id[..8])
        } else {
            // image_id path — look up image name from DB (already fetched above)
            format!("env-{}", &id[..8])
        }
    });
    // For image-based creation, use image name if available
    let name = if iso_url.is_none() && name.starts_with("env-") {
        if let Some(ref img_id) = image_id {
            let img =
                sqlx::query_as::<_, crate::models::Image>("SELECT * FROM images WHERE id = ?")
                    .bind(img_id)
                    .fetch_optional(&state.db)
                    .await?;
            if let Some(img) = img {
                if let Some(ref img_name) = img.name {
                    format!("{}-{}", img_name, &id[..8])
                } else {
                    name
                }
            } else {
                name
            }
        } else {
            name
        }
    } else {
        name
    };

    let win_opts_json = req
        .win_install_options
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());

    sqlx::query(
        "INSERT INTO environments (id, name, image_id, iso_url, owner_user_id, node_id, provider, guest_os, guest_arch, state, created_at, expires_at, vcpus, memory_bytes, disk_bytes, win_install_options)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'creating', ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&image_id)
    .bind(&iso_url)
    .bind(&user.0.id)
    .bind(&node.id)
    .bind(&provider)
    .bind(&guest_os)
    .bind(&guest_arch)
    .bind(now)
    .bind(expires_at)
    .bind(vcpus)
    .bind(memory_bytes)
    .bind(disk_bytes)
    .bind(&win_opts_json)
    .execute(&state.db)
    .await?;

    events::emit(
        &state.db,
        &id,
        "created",
        None,
        Some("creating"),
        Some(&user.0.id),
        None,
    )
    .await;

    // Only dispatch in-process for libvirt provider.
    // macOS environments stay in "creating" state until the node agent picks them up.
    if !is_agent_managed(&provider) {
        // Fetch user's SSH keys for injection into the VM
        let ssh_keys: Vec<String> = sqlx::query_as::<_, crate::models::UserSshKey>(
            "SELECT * FROM user_ssh_keys WHERE user_id = ?",
        )
        .bind(&user.0.id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|k| k.public_key)
        .collect();

        let task_id = tasks::create_task_for_env(&state.db, &format!("create_env:{id}"), Some(&id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;

        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let env_id_clone = id.clone();
        let guest_os_clone = guest_os.clone();
        let guest_arch_clone = guest_arch.clone();
        let node_name = node.name.clone().unwrap_or_default();
        let iso_url_clone = iso_url.clone();
        let win_opts_json_clone = win_opts_json.clone();
        // Look up image name for libvirt
        let image_name = if let Some(ref img_id) = image_id {
            sqlx::query_as::<_, crate::models::Image>("SELECT * FROM images WHERE id = ?")
                .bind(img_id)
                .fetch_optional(&state.db)
                .await?
                .and_then(|i| i.name)
                .unwrap_or_default()
        } else {
            String::new()
        };
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;

            let params = VmCreateParams {
                env_id: env_id_clone.clone(),
                image_name,
                guest_os: guest_os_clone,
                guest_arch: guest_arch_clone,
                node_host: node_name,
                vcpus,
                memory_bytes,
                disk_bytes,
                ssh_authorized_keys: ssh_keys,
                iso_url: iso_url_clone,
                win_install_options: win_opts_json_clone,
            };

            match vm_provider.create_vm(params).await {
                Ok(_info) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'running' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::update_task_state(&db, &task_id, "completed").await;
                    events::emit(
                        &db,
                        &env_id_clone,
                        "state_change",
                        Some("creating"),
                        Some("running"),
                        Some("system"),
                        None,
                    )
                    .await;
                    tracing::info!(env_id = %env_id_clone, "environment is now running");
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(
                        &db,
                        &env_id_clone,
                        "state_change",
                        Some("creating"),
                        Some("failed"),
                        Some("system"),
                        Some(&e),
                    )
                    .await;
                    tracing::error!(env_id = %env_id_clone, error = %e, "failed to create VM");
                }
            }
        });
    }

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(env)))
}

async fn destroy_environment(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let current = env.state.as_deref().unwrap_or("");

    // If already destroying, the agent is confirming deletion — remove from DB
    if current == "destroying" {
        crate::delete_environment_cascade(&state.db, &env_id).await;
        return Ok(StatusCode::NO_CONTENT);
    }

    sqlx::query("UPDATE environments SET state = 'destroying' WHERE id = ?")
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    events::emit(
        &state.db,
        &env_id,
        "state_change",
        Some(current),
        Some("destroying"),
        Some(&user.0.id),
        None,
    )
    .await;

    let provider = env.provider.as_deref().unwrap_or("libvirt");

    if !is_agent_managed(provider) {
        let task_id = tasks::create_task_for_env(&state.db, &format!("destroy_env:{env_id}"), Some(&env_id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;

        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let libvirt_uri = state.libvirt_uri.clone();
        let env_id_clone = env_id.clone();
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;

            // Detach any USB devices before destroying the VM
            if let Some(ref uri) = libvirt_uri {
                let usb_devices: Vec<UsbAttachment> = sqlx::query_as(
                    "SELECT * FROM usb_attachments WHERE env_id = ?",
                )
                .bind(&env_id_clone)
                .fetch_all(&db)
                .await
                .unwrap_or_default();

                if !usb_devices.is_empty() {
                    let domain = crate::vm::libvirt::env_domain_name(&env_id_clone);
                    let devs: Vec<(String, String)> = usb_devices
                        .iter()
                        .map(|a| (a.vendor_id.clone(), a.product_id.clone()))
                        .collect();
                    crate::vm::libvirt::detach_all_usb(uri, &domain, &devs).await;
                }
            }

            let provider_id = format!("libvirt-{env_id_clone}");
            match vm_provider.destroy_vm(&provider_id).await {
                Ok(()) => {
                    crate::delete_environment_cascade(&db, &env_id_clone).await;
                    tracing::info!(env_id = %env_id_clone, "environment destroyed");
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(
                        &db,
                        &env_id_clone,
                        "state_change",
                        Some("destroying"),
                        Some("failed"),
                        Some("system"),
                        Some(&e),
                    )
                    .await;
                    tracing::error!(env_id = %env_id_clone, error = %e, "failed to destroy VM");
                }
            }
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn ssh_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let env = fetch_env(&state, &env_id).await?;

    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if is_agent_managed(provider) {
        // If port forwarding endpoints are set, return those
        if let (Some(host), Some(port)) = (&env.ssh_host, env.ssh_port) {
            return Ok(Json(serde_json::json!({
                "env_id": env.id,
                "host": host,
                "port": port,
            })));
        }
        // Fallback: return node hostname with port 22 (local-only)
        let node = sqlx::query_as::<_, crate::models::Node>(
            "SELECT * FROM nodes WHERE id = ?",
        )
        .bind(env.node_id.as_deref().unwrap_or(""))
        .fetch_optional(&state.db)
        .await?;

        let host = node
            .and_then(|n| n.name)
            .unwrap_or_else(|| "unknown".into());

        return Ok(Json(serde_json::json!({
            "env_id": env.id,
            "host": host,
            "port": 22,
        })));
    }

    let provider_id = format!("libvirt-{}", env.id);
    let info = state
        .vm_provider
        .get_vm_info(&provider_id)
        .await
        .map_err(AppError::Internal)?;

    // Default credentials depend on guest OS — cloud-init Linux uses ubuntu/orion,
    // Windows VMs use whatever was configured in win_install_options (no default).
    let guest_os = env.guest_os.as_deref().unwrap_or("linux");
    let mut resp = serde_json::json!({
        "env_id": env.id,
        "host": info.ssh_host,
        "port": info.ssh_port,
    });
    if guest_os != "windows" {
        resp["username"] = serde_json::json!("ubuntu");
        resp["password"] = serde_json::json!("orion");
    }

    Ok(Json(resp))
}

async fn vnc_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let env = fetch_env(&state, &env_id).await?;

    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if is_agent_managed(provider) {
        // If port forwarding endpoints are set, return those
        if let (Some(host), Some(port)) = (&env.vnc_host, env.vnc_port) {
            return Ok(Json(serde_json::json!({
                "env_id": env.id,
                "host": host,
                "port": port,
            })));
        }
        return Err(AppError::BadRequest(
            "VNC endpoint not available — enable port forwarding first".into(),
        ));
    }

    let provider_id = format!("libvirt-{}", env.id);
    let info = state
        .vm_provider
        .get_vm_info(&provider_id)
        .await
        .map_err(AppError::Internal)?;

    Ok(Json(serde_json::json!({
        "env_id": env.id,
        "host": info.vnc_host,
        "port": info.vnc_port,
    })))
}

// ── Port forwarding ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TogglePortForwardingRequest {
    enabled: bool,
}

async fn toggle_port_forwarding(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<TogglePortForwardingRequest>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let val: i64 = if req.enabled { 1 } else { 0 };

    // If disabling, also clear the endpoint fields
    if !req.enabled {
        sqlx::query(
            "UPDATE environments SET port_forwarding = 0, ssh_host = NULL, ssh_port = NULL, vnc_host = NULL, vnc_port = NULL WHERE id = ?",
        )
        .bind(&env_id)
        .execute(&state.db)
        .await?;
    } else {
        sqlx::query("UPDATE environments SET port_forwarding = ? WHERE id = ?")
            .bind(val)
            .bind(&env_id)
            .execute(&state.db)
            .await?;
    }

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

/// Called by the node agent to report forwarded SSH/VNC endpoints.
#[derive(Debug, Deserialize)]
struct UpdateEndpointsRequest {
    ssh_host: Option<String>,
    ssh_port: Option<i64>,
    vnc_host: Option<String>,
    vnc_port: Option<i64>,
}

async fn update_endpoints(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<UpdateEndpointsRequest>,
) -> Result<Json<Environment>, AppError> {
    sqlx::query(
        "UPDATE environments SET ssh_host = ?, ssh_port = ?, vnc_host = ?, vnc_port = ? WHERE id = ?",
    )
    .bind(&req.ssh_host)
    .bind(req.ssh_port)
    .bind(&req.vnc_host)
    .bind(req.vnc_port)
    .bind(&env_id)
    .execute(&state.db)
    .await?;

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

async fn capture_image(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<CaptureImageRequest>,
) -> Result<(StatusCode, Json<crate::models::Image>), AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let current = env.state.as_deref().unwrap_or("");
    if !matches!(current, "running" | "suspended") {
        return Err(AppError::BadRequest(format!(
            "cannot capture image from environment in state '{current}'"
        )));
    }

    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if !is_agent_managed(provider) {
        return Err(AppError::BadRequest(
            "image capture is only supported for agent-managed environments".into(),
        ));
    }

    // Create the image record
    let image_id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    sqlx::query(
        "INSERT INTO images (id, name, provider, guest_os, guest_arch, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&image_id)
    .bind(&req.name)
    .bind(&env.provider)
    .bind(&env.guest_os)
    .bind(&env.guest_arch)
    .bind(now)
    .execute(&state.db)
    .await?;

    // Set the capture target on the environment and transition to capturing
    let old_state = env.state.clone();
    sqlx::query("UPDATE environments SET state = 'capturing', capture_image_id = ? WHERE id = ?")
        .bind(&image_id)
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    events::emit(
        &state.db,
        &env_id,
        "state_change",
        old_state.as_deref(),
        Some("capturing"),
        Some(&user.0.id),
        Some(&format!("capturing as image '{}'", req.name)),
    )
    .await;

    let image = sqlx::query_as::<_, crate::models::Image>("SELECT * FROM images WHERE id = ?")
        .bind(&image_id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(image)))
}

#[derive(Debug, Deserialize)]
struct UpdateIsoUrlRequest {
    iso_url: String,
}

async fn update_iso_url(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<UpdateIsoUrlRequest>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    sqlx::query("UPDATE environments SET iso_url = ? WHERE id = ?")
        .bind(&req.iso_url)
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

async fn rename_environment(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<RenameEnvironmentRequest>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    sqlx::query("UPDATE environments SET name = ? WHERE id = ?")
        .bind(&req.name)
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

async fn transition_state(
    state: &AppState,
    env_id: &str,
    user: &crate::models::User,
    valid_from: &[&str],
    new_state: &str,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(state, env_id).await?;
    check_env_owner(user, &env)?;

    let current = env.state.as_deref().unwrap_or("");
    if !valid_from.contains(&current) {
        return Err(AppError::BadRequest(format!(
            "cannot transition from '{current}' to '{new_state}'"
        )));
    }

    // When transitioning to states where the VM is being (re)created or destroyed,
    // clear stale endpoint info — the VM's IP/ports will change.
    if matches!(new_state, "creating" | "destroying") {
        sqlx::query(
            "UPDATE environments SET state = ?, ssh_host = NULL, ssh_port = NULL, vnc_host = NULL, vnc_port = NULL WHERE id = ?"
        )
        .bind(new_state)
        .bind(env_id)
        .execute(&state.db)
        .await?;
    } else {
        sqlx::query("UPDATE environments SET state = ? WHERE id = ?")
            .bind(new_state)
            .bind(env_id)
            .execute(&state.db)
            .await?;
    }

    events::emit(
        &state.db,
        env_id,
        "state_change",
        Some(current),
        Some(new_state),
        Some(&user.id),
        None,
    )
    .await;

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

async fn suspend(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Environment>, AppError> {
    let env = transition_state(&state, &env_id, &user.0, &["running"], "suspending").await?;

    let is_agent = is_agent_managed(env.0.provider.as_deref().unwrap_or("libvirt"));
    if !is_agent {
        let task_id = tasks::create_task_for_env(&state.db, &format!("suspend_env:{env_id}"), Some(&env_id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;
        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let env_id_clone = env_id.clone();
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;
            let provider_id = format!("libvirt-{env_id_clone}");
            match vm_provider.suspend_vm(&provider_id).await {
                Ok(()) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'suspended' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::update_task_state(&db, &task_id, "completed").await;
                    events::emit(&db, &env_id_clone, "state_change", Some("suspending"), Some("suspended"), Some("system"), None).await;
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(&db, &env_id_clone, "state_change", Some("suspending"), Some("failed"), Some("system"), Some(&e)).await;
                }
            }
        });
    }

    Ok(env)
}

async fn resume(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Environment>, AppError> {
    let env = transition_state(&state, &env_id, &user.0, &["suspended"], "resuming").await?;

    let is_agent = is_agent_managed(env.0.provider.as_deref().unwrap_or("libvirt"));
    if !is_agent {
        let task_id = tasks::create_task_for_env(&state.db, &format!("resume_env:{env_id}"), Some(&env_id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;
        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let env_id_clone = env_id.clone();
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;
            let provider_id = format!("libvirt-{env_id_clone}");
            match vm_provider.resume_vm(&provider_id).await {
                Ok(()) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'running' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::update_task_state(&db, &task_id, "completed").await;
                    events::emit(&db, &env_id_clone, "state_change", Some("resuming"), Some("running"), Some("system"), None).await;
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(&db, &env_id_clone, "state_change", Some("resuming"), Some("failed"), Some("system"), Some(&e)).await;
                }
            }
        });
    }

    Ok(env)
}

async fn reboot(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Environment>, AppError> {
    let env = transition_state(&state, &env_id, &user.0, &["running"], "rebooting").await?;

    let is_agent = is_agent_managed(env.0.provider.as_deref().unwrap_or("libvirt"));
    if !is_agent {
        let task_id = tasks::create_task_for_env(&state.db, &format!("reboot_env:{env_id}"), Some(&env_id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;
        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let env_id_clone = env_id.clone();
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;
            let provider_id = format!("libvirt-{env_id_clone}");
            match vm_provider.reboot_vm(&provider_id, false).await {
                Ok(()) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'running' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::update_task_state(&db, &task_id, "completed").await;
                    events::emit(&db, &env_id_clone, "state_change", Some("rebooting"), Some("running"), Some("system"), None).await;
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(&db, &env_id_clone, "state_change", Some("rebooting"), Some("failed"), Some("system"), Some(&e)).await;
                }
            }
        });
    }

    Ok(env)
}

async fn force_reboot(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Environment>, AppError> {
    let env = transition_state(
        &state,
        &env_id,
        &user.0,
        &["running", "suspended", "suspending", "resuming"],
        "rebooting",
    )
    .await?;

    let is_agent = is_agent_managed(env.0.provider.as_deref().unwrap_or("libvirt"));
    if !is_agent {
        let task_id = tasks::create_task_for_env(&state.db, &format!("force_reboot_env:{env_id}"), Some(&env_id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;
        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let env_id_clone = env_id.clone();
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;
            let provider_id = format!("libvirt-{env_id_clone}");
            match vm_provider.reboot_vm(&provider_id, true).await {
                Ok(()) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'running' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::update_task_state(&db, &task_id, "completed").await;
                    events::emit(&db, &env_id_clone, "state_change", Some("rebooting"), Some("running"), Some("system"), None).await;
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(&db, &env_id_clone, "state_change", Some("rebooting"), Some("failed"), Some("system"), Some(&e)).await;
                }
            }
        });
    }

    Ok(env)
}

async fn restart(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Environment>, AppError> {
    // Transition failed → creating; the node agent (or libvirt task spawner in create_environment)
    // will pick up the "creating" state and re-attempt VM creation.
    let env = transition_state(&state, &env_id, &user.0, &["failed"], "creating").await?;
    Ok(env)
}

async fn migrate(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<MigrateRequest>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let current = env.state.as_deref().unwrap_or("");
    if current != "suspended" {
        return Err(AppError::BadRequest(format!(
            "cannot migrate from '{current}', must be 'suspended'"
        )));
    }

    // Validate target node
    let target = sqlx::query_as::<_, crate::models::Node>("SELECT * FROM nodes WHERE id = ?")
        .bind(&req.target_node_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!("target node {} not found", req.target_node_id))
        })?;

    if target.online != Some(1) {
        return Err(AppError::BadRequest(format!(
            "target node {} is offline",
            req.target_node_id
        )));
    }

    let env_provider = env.provider.as_deref().unwrap_or("libvirt");
    let required_os = if is_agent_managed(env_provider) {
        "macos"
    } else {
        "linux"
    };
    if target.host_os.as_deref() != Some(required_os) {
        return Err(AppError::BadRequest(format!(
            "target node has host_os '{}', need '{required_os}'",
            target.host_os.as_deref().unwrap_or("unknown")
        )));
    }

    if env.node_id.as_deref() == Some(&req.target_node_id) {
        return Err(AppError::BadRequest(
            "target node is the same as current node".into(),
        ));
    }

    sqlx::query("UPDATE environments SET state = 'migrating' WHERE id = ?")
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    events::emit(
        &state.db,
        &env_id,
        "state_change",
        Some("suspended"),
        Some("migrating"),
        Some(&user.0.id),
        Some(&format!("migrating to node {}", req.target_node_id)),
    )
    .await;

    if !is_agent_managed(env_provider) {
        let task_id = tasks::create_task_for_env(&state.db, &format!("migrate_env:{env_id}"), Some(&env_id))
            .await
            .map_err(|e| AppError::Internal(format!("failed to create task: {e}")))?;

        let vm_provider = state.vm_provider.clone();
        let db = state.db.clone();
        let env_id_clone = env_id.clone();
        let target_node_id = req.target_node_id.clone();
        let target_host = target.name.clone().unwrap_or_default();
        tokio::spawn(async move {
            let _ = tasks::update_task_state(&db, &task_id, "running").await;
            let provider_id = format!("libvirt-{env_id_clone}");
            match vm_provider.migrate_vm(&provider_id, &target_host).await {
                Ok(()) => {
                    let _ = sqlx::query(
                        "UPDATE environments SET state = 'suspended', node_id = ? WHERE id = ?",
                    )
                    .bind(&target_node_id)
                    .bind(&env_id_clone)
                    .execute(&db)
                    .await;
                    let _ = tasks::update_task_state(&db, &task_id, "completed").await;
                    events::emit(&db, &env_id_clone, "state_change", Some("migrating"), Some("suspended"), Some("system"), Some("migration complete")).await;
                    tracing::info!(env_id = %env_id_clone, "environment migrated");
                }
                Err(e) => {
                    let _ =
                        sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                            .bind(&env_id_clone)
                            .execute(&db)
                            .await;
                    let _ = tasks::fail_task(&db, &task_id, &e).await;
                    events::emit(&db, &env_id_clone, "state_change", Some("migrating"), Some("failed"), Some("system"), Some(&e)).await;
                    tracing::error!(env_id = %env_id_clone, error = %e, "failed to migrate VM");
                }
            }
        });
    } else {
        // For agent-managed: update node_id, agents handle the rest
        sqlx::query("UPDATE environments SET node_id = ? WHERE id = ?")
            .bind(&req.target_node_id)
            .bind(&env_id)
            .execute(&state.db)
            .await?;
    }

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}

async fn extend_ttl(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<ExtendTtlRequest>,
) -> Result<Json<Environment>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let current = env.state.as_deref().unwrap_or("");
    if matches!(current, "destroying" | "failed") {
        return Err(AppError::BadRequest(format!(
            "cannot extend TTL for environment in state '{current}'"
        )));
    }

    if req.ttl_seconds <= 0 {
        return Err(AppError::BadRequest(
            "ttl_seconds must be positive".into(),
        ));
    }

    let now = crate::unix_now();
    let new_expires_at = now + req.ttl_seconds;

    sqlx::query("UPDATE environments SET expires_at = ? WHERE id = ?")
        .bind(new_expires_at)
        .bind(&env_id)
        .execute(&state.db)
        .await?;

    events::emit(
        &state.db,
        &env_id,
        "ttl_extended",
        None,
        None,
        Some(&user.0.id),
        Some(&format!(
            "TTL extended by {}s, new expiry: {new_expires_at}",
            req.ttl_seconds
        )),
    )
    .await;

    let env = sqlx::query_as::<_, Environment>("SELECT * FROM environments WHERE id = ?")
        .bind(&env_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(env))
}
