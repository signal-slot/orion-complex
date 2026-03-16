use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub provider: Option<String>,
    pub provider_subject: Option<String>,
    pub email: Option<String>,
    pub email_domain: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub disabled: i64,
    #[serde(skip_serializing)]
    pub totp_secret: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserSshKey {
    pub id: String,
    pub user_id: Option<String>,
    pub public_key: Option<String>,
    pub fingerprint: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Node {
    pub id: String,
    pub name: Option<String>,
    pub host_os: Option<String>,
    pub host_arch: Option<String>,
    pub cpu_cores: Option<i64>,
    pub memory_bytes: Option<i64>,
    pub disk_bytes_total: Option<i64>,
    pub max_cpu_utilization_ratio: Option<f64>,
    pub max_memory_utilization_ratio: Option<f64>,
    pub max_disk_utilization_ratio: Option<f64>,
    pub max_running_envs: Option<i64>,
    pub online: Option<i64>,
    pub last_heartbeat_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Image {
    pub id: String,
    pub name: Option<String>,
    pub provider: Option<String>,
    pub guest_os: Option<String>,
    pub guest_arch: Option<String>,
    pub base_image_id: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Environment {
    pub id: String,
    pub name: Option<String>,
    pub image_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub node_id: Option<String>,
    pub provider: Option<String>,
    pub guest_os: Option<String>,
    pub guest_arch: Option<String>,
    pub state: Option<String>,
    pub created_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub vcpus: Option<i64>,
    pub memory_bytes: Option<i64>,
    pub disk_bytes: Option<i64>,
    pub port_forwarding: i64,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<i64>,
    pub vnc_host: Option<String>,
    pub vnc_port: Option<i64>,
    pub iso_url: Option<String>,
    pub capture_image_id: Option<String>,
    pub bypass_hw_check: i64,
    pub win_install_options: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Snapshot {
    pub id: String,
    pub env_id: Option<String>,
    pub name: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub created_at: Option<i64>,
    pub error_message: Option<String>,
    pub completed_at: Option<i64>,
    pub env_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EnvironmentEvent {
    pub id: String,
    pub env_id: String,
    pub event_type: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub triggered_by: Option<String>,
    pub message: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebauthnCredential {
    pub id: String,
    pub user_id: String,
    pub credential_id: String,
    pub credential_json: String,
    pub name: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WebauthnChallenge {
    pub id: String,
    pub challenge_type: String,
    pub state_json: String,
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct CreateEnvironmentRequest {
    pub image_id: Option<String>,
    pub iso_url: Option<String>,
    pub name: Option<String>,
    pub node_id: Option<String>,
    pub guest_os: Option<String>,
    pub guest_arch: Option<String>,
    pub ttl_seconds: Option<i64>,
    pub vcpus: Option<i64>,
    pub memory_bytes: Option<i64>,
    pub disk_bytes: Option<i64>,
    pub win_install_options: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CaptureImageRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameEnvironmentRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MigrateRequest {
    pub target_node_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ExtendTtlRequest {
    pub ttl_seconds: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateSnapshotRequest {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterNodeRequest {
    pub name: String,
    pub host_os: String,
    pub host_arch: String,
    pub cpu_cores: i64,
    pub memory_bytes: i64,
    pub disk_bytes_total: i64,
    pub max_cpu_utilization_ratio: Option<f64>,
    pub max_memory_utilization_ratio: Option<f64>,
    pub max_disk_utilization_ratio: Option<f64>,
    pub max_running_envs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNodeRequest {
    pub name: Option<String>,
    pub max_cpu_utilization_ratio: Option<f64>,
    pub max_memory_utilization_ratio: Option<f64>,
    pub max_disk_utilization_ratio: Option<f64>,
    pub max_running_envs: Option<i64>,
    pub online: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterImageRequest {
    pub name: String,
    pub provider: String,
    pub guest_os: String,
    pub guest_arch: String,
    pub base_image_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct SetUserDisabledRequest {
    pub disabled: bool,
}
