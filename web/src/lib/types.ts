// Types matching the Rust backend models (src/models.rs) and OpenAPI spec

// ── Enums ──────────────────────────────────────────────────────────

export type UserRole = "user" | "admin";

export type EnvironmentState =
  | "creating"
  | "running"
  | "suspending"
  | "suspended"
  | "resuming"
  | "rebooting"
  | "migrating"
  | "destroying"
  | "capturing"
  | "failed";

export type TaskState = "pending" | "running" | "completed" | "failed";

export type AuthProvider = "google" | "microsoft";

export type ImageProvider = "libvirt" | "macos" | "virtualization";

export type HostOs = "linux" | "macos";

// ── Resource Models ────────────────────────────────────────────────

export interface User {
  id: string;
  provider: string | null;
  provider_subject: string | null;
  email: string | null;
  email_domain: string | null;
  display_name: string | null;
  role: UserRole | null;
  disabled: number;
  created_at: number | null;
}

export interface Node {
  id: string;
  name: string | null;
  host_os: HostOs | null;
  host_arch: string | null;
  cpu_cores: number | null;
  memory_bytes: number | null;
  disk_bytes_total: number | null;
  max_cpu_utilization_ratio: number | null;
  max_memory_utilization_ratio: number | null;
  max_disk_utilization_ratio: number | null;
  max_running_envs: number | null;
  online: number | null;
  last_heartbeat_at: number | null;
}

export interface NodeUsage {
  node_id: string;
  environment_count: number;
  max_environments: number;
  vcpus_used: number;
  vcpus_capacity: number;
  memory_bytes_used: number;
  memory_bytes_capacity: number;
  disk_bytes_used: number;
  disk_bytes_capacity: number;
}

export interface Image {
  id: string;
  name: string | null;
  provider: ImageProvider | null;
  guest_os: string | null;
  guest_arch: string | null;
  base_image_id: string | null;
  created_at: number | null;
}

export interface Environment {
  id: string;
  name: string | null;
  image_id: string | null;
  owner_user_id: string | null;
  node_id: string | null;
  provider: string | null;
  guest_os: string | null;
  guest_arch: string | null;
  state: EnvironmentState | null;
  created_at: number | null;
  expires_at: number | null;
  vcpus: number | null;
  memory_bytes: number | null;
  disk_bytes: number | null;
  port_forwarding: number;
  ssh_host: string | null;
  ssh_port: number | null;
  vnc_host: string | null;
  vnc_port: number | null;
  iso_url: string | null;
  capture_image_id: string | null;
}

export interface Snapshot {
  id: string;
  env_id: string | null;
  name: string | null;
  created_at: number | null;
}

export interface Task {
  id: string;
  kind: string | null;
  state: TaskState | null;
  created_at: number | null;
  error_message: string | null;
  completed_at: number | null;
  env_id: string | null;
}

export interface EnvironmentEvent {
  id: string;
  env_id: string;
  event_type: string;
  from_state: string | null;
  to_state: string | null;
  triggered_by: string | null;
  message: string | null;
  created_at: number;
}

export interface UserSshKey {
  id: string;
  user_id: string | null;
  public_key: string | null;
  fingerprint: string | null;
  created_at: number | null;
}

// ── Endpoint-specific response types ───────────────────────────────

export interface SshEndpoint {
  env_id: string;
  host: string;
  port: number;
}

export interface VncEndpoint {
  env_id: string;
  host: string;
  port: number;
}

export interface LoginResponse {
  token: string;
  user: User;
}

export interface AuthProvidersResponse {
  providers: string[];
}

export interface ApiError {
  error: string;
}

// ── Pagination ─────────────────────────────────────────────────────

export interface PaginationParams {
  limit?: number;
  offset?: number;
}

export interface PaginatedResponse<T> {
  data: T[];
  total?: number;
  offset: number;
  limit: number;
}

// ── Request types ──────────────────────────────────────────────────

export interface LoginRequest {
  provider: AuthProvider;
  access_token: string;
}

export interface CreateEnvironmentRequest {
  image_id?: string;
  iso_url?: string;
  name?: string;
  node_id?: string;
  guest_os?: string;
  guest_arch?: string;
  ttl_seconds?: number;
  vcpus?: number;
  memory_bytes?: number;
  disk_bytes?: number;
}

export interface MigrateRequest {
  target_node_id: string;
}

export interface ExtendTtlRequest {
  ttl_seconds: number;
}

export interface CreateSnapshotRequest {
  name?: string;
}

export interface RegisterNodeRequest {
  name: string;
  host_os: string;
  host_arch: string;
  cpu_cores: number;
  memory_bytes: number;
  disk_bytes_total: number;
  max_cpu_utilization_ratio?: number;
  max_memory_utilization_ratio?: number;
  max_disk_utilization_ratio?: number;
  max_running_envs?: number;
}

export interface UpdateNodeRequest {
  name?: string;
  max_cpu_utilization_ratio?: number;
  max_memory_utilization_ratio?: number;
  max_disk_utilization_ratio?: number;
  max_running_envs?: number;
  online?: number;
}

export interface RegisterImageRequest {
  name: string;
  provider: string;
  guest_os: string;
  guest_arch: string;
  base_image_id?: string;
}

export interface UpdateUserRoleRequest {
  role: UserRole;
}

export interface SetUserDisabledRequest {
  disabled: boolean;
}

export interface AddSshKeyRequest {
  public_key: string;
}

export interface UpdateEnvironmentStateRequest {
  state: string;
}
