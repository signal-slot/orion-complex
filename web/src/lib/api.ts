import { getToken, clearToken } from "./auth";
import type {
  User,
  Node,
  NodeUsage,
  Image,
  Environment,
  UserSshKey,
  Snapshot,
  Task,
  EnvironmentEvent,
  SshEndpoint,
  VncEndpoint,
  LoginRequest,
  LoginResponse,
  AuthProvidersResponse,
  CreateEnvironmentRequest,
  MigrateRequest,
  ExtendTtlRequest,
  CreateSnapshotRequest,
  RegisterNodeRequest,
  UpdateNodeRequest,
  RegisterImageRequest,
  UpdateUserRoleRequest,
  SetUserDisabledRequest,
  AddSshKeyRequest,
  UpdateEnvironmentStateRequest,
  PaginationParams,
} from "./types";

const BASE_URL =
  process.env.NEXT_PUBLIC_API_URL?.replace(/\/+$/, "") || "";

// ── Internal helpers ───────────────────────────────────────────────

class ApiError extends Error {
  constructor(
    public status: number,
    public body: string,
  ) {
    let message: string;
    try {
      const parsed = JSON.parse(body);
      message = parsed.error || body;
    } catch {
      message = body;
    }
    super(message);
    this.name = "ApiError";
  }
}

function authHeaders(): HeadersInit {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  const token = getToken();
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  return headers;
}

function queryString(params?: PaginationParams): string {
  if (!params) return "";
  const parts: string[] = [];
  if (params.limit !== undefined) parts.push(`limit=${params.limit}`);
  if (params.offset !== undefined) parts.push(`offset=${params.offset}`);
  return parts.length > 0 ? `?${parts.join("&")}` : "";
}

async function request<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const res = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers: {
      ...authHeaders(),
      ...(options.headers || {}),
    },
  });

  if (res.status === 401) {
    clearToken();
    if (typeof window !== "undefined") {
      window.location.href = "/login";
    }
    throw new ApiError(401, "Unauthorized");
  }

  if (!res.ok) {
    const text = await res.text();
    throw new ApiError(res.status, text);
  }

  if (res.status === 204) {
    return undefined as T;
  }

  return res.json();
}

// ── Auth ───────────────────────────────────────────────────────────

export async function getAuthProviders(): Promise<AuthProvidersResponse> {
  return request<AuthProvidersResponse>("/v1/auth/providers", {
    headers: { "Content-Type": "application/json" },
  });
}

export async function login(body: LoginRequest): Promise<LoginResponse> {
  return request<LoginResponse>("/v1/auth/login", {
    method: "POST",
    body: JSON.stringify(body),
    headers: { "Content-Type": "application/json" },
  });
}

export async function getMe(): Promise<User> {
  return request<User>("/v1/auth/me");
}

// ── WebAuthn ──────────────────────────────────────────────────────

export async function webauthnRegisterBegin(body: {
  email: string;
  display_name?: string;
}): Promise<{ challenge_id: string; publicKey: unknown }> {
  return request("/v1/auth/webauthn/register/begin", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function webauthnRegisterComplete(body: {
  challenge_id: string;
  credential: unknown;
}): Promise<LoginResponse> {
  return request("/v1/auth/webauthn/register/complete", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function webauthnLoginBegin(body: {
  email: string;
}): Promise<{ challenge_id: string; publicKey: unknown }> {
  return request("/v1/auth/webauthn/login/begin", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function webauthnLoginComplete(body: {
  challenge_id: string;
  credential: unknown;
}): Promise<LoginResponse> {
  return request("/v1/auth/webauthn/login/complete", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

// ── TOTP (fallback) ──────────────────────────────────────────────

export async function totpRegister(body: {
  username: string;
  display_name?: string;
}): Promise<{ challenge_id: string; otpauth_url: string; secret: string }> {
  return request("/v1/auth/totp/register", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function totpVerify(body: {
  challenge_id: string;
  code: string;
}): Promise<LoginResponse> {
  return request("/v1/auth/totp/verify", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function totpLogin(body: {
  code: string;
}): Promise<LoginResponse> {
  return request("/v1/auth/totp/login", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

// ── Users (admin) ──────────────────────────────────────────────────

export async function listUsers(params?: PaginationParams): Promise<User[]> {
  return request<User[]>(`/v1/users${queryString(params)}`);
}

export async function getUser(userId: string): Promise<User> {
  return request<User>(`/v1/users/${userId}`);
}

export async function updateUserRole(
  userId: string,
  body: UpdateUserRoleRequest,
): Promise<User> {
  return request<User>(`/v1/users/${userId}/role`, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export async function setUserDisabled(
  userId: string,
  body: SetUserDisabledRequest,
): Promise<User> {
  return request<User>(`/v1/users/${userId}/disabled`, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export async function listUserSshKeys(userId: string): Promise<UserSshKey[]> {
  return request<UserSshKey[]>(`/v1/users/${userId}/ssh-keys`);
}

// ── SSH Keys ───────────────────────────────────────────────────────

export async function listSshKeys(): Promise<UserSshKey[]> {
  return request<UserSshKey[]>("/v1/ssh-keys");
}

export async function addSshKey(body: AddSshKeyRequest): Promise<UserSshKey> {
  return request<UserSshKey>("/v1/ssh-keys", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function deleteSshKey(keyId: string): Promise<void> {
  return request<void>(`/v1/ssh-keys/${keyId}`, { method: "DELETE" });
}

// ── Nodes ──────────────────────────────────────────────────────────

export async function listNodes(params?: PaginationParams): Promise<Node[]> {
  return request<Node[]>(`/v1/nodes${queryString(params)}`);
}

export async function getNode(nodeId: string): Promise<Node> {
  return request<Node>(`/v1/nodes/${nodeId}`);
}

export async function registerNode(body: RegisterNodeRequest): Promise<Node> {
  return request<Node>("/v1/nodes", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function updateNode(
  nodeId: string,
  body: UpdateNodeRequest,
): Promise<Node> {
  return request<Node>(`/v1/nodes/${nodeId}`, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export async function deleteNode(nodeId: string): Promise<void> {
  return request<void>(`/v1/nodes/${nodeId}`, { method: "DELETE" });
}

export async function nodeHeartbeat(nodeId: string): Promise<void> {
  return request<void>(`/v1/nodes/${nodeId}/heartbeat`, { method: "POST" });
}

export async function getNodeUsage(nodeId: string): Promise<NodeUsage> {
  return request<NodeUsage>(`/v1/nodes/${nodeId}/usage`);
}

// ── Images ─────────────────────────────────────────────────────────

export async function listImages(params?: PaginationParams): Promise<Image[]> {
  return request<Image[]>(`/v1/images${queryString(params)}`);
}

export async function getImage(imageId: string): Promise<Image> {
  return request<Image>(`/v1/images/${imageId}`);
}

export async function registerImage(
  body: RegisterImageRequest,
): Promise<Image> {
  return request<Image>("/v1/images", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function deleteImage(imageId: string): Promise<void> {
  return request<void>(`/v1/images/${imageId}`, { method: "DELETE" });
}

// ── Environments ───────────────────────────────────────────────────

export async function listEnvironments(
  params?: PaginationParams,
): Promise<Environment[]> {
  return request<Environment[]>(`/v1/environments${queryString(params)}`);
}

export async function getEnvironment(envId: string): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}`);
}

export async function createEnvironment(
  body: CreateEnvironmentRequest,
): Promise<Environment> {
  return request<Environment>("/v1/environments", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function deleteEnvironment(envId: string): Promise<void> {
  return request<void>(`/v1/environments/${envId}`, { method: "DELETE" });
}

export async function updateEnvironmentState(
  envId: string,
  body: UpdateEnvironmentStateRequest,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/state`, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export async function getSshEndpoint(envId: string): Promise<SshEndpoint> {
  return request<SshEndpoint>(`/v1/environments/${envId}/ssh-endpoint`);
}

export async function getVncEndpoint(envId: string): Promise<VncEndpoint> {
  return request<VncEndpoint>(`/v1/environments/${envId}/vnc-endpoint`);
}

export async function suspendEnvironment(
  envId: string,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/suspend`, {
    method: "POST",
  });
}

export async function resumeEnvironment(envId: string): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/resume`, {
    method: "POST",
  });
}

export async function rebootEnvironment(envId: string): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/reboot`, {
    method: "POST",
  });
}

export async function forceRebootEnvironment(
  envId: string,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/force-reboot`, {
    method: "POST",
  });
}

export async function restartEnvironment(envId: string): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/restart`, {
    method: "POST",
  });
}

export async function migrateEnvironment(
  envId: string,
  body: MigrateRequest,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/migrate`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function togglePortForwarding(
  envId: string,
  enabled: boolean,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/port-forwarding`, {
    method: "POST",
    body: JSON.stringify({ enabled }),
  });
}

export async function renameEnvironment(
  envId: string,
  name: string,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/name`, {
    method: "PUT",
    body: JSON.stringify({ name }),
  });
}

export async function extendEnvironmentTtl(
  envId: string,
  body: ExtendTtlRequest,
): Promise<Environment> {
  return request<Environment>(`/v1/environments/${envId}/extend-ttl`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

// ── Snapshots ──────────────────────────────────────────────────────

export async function listSnapshots(envId: string): Promise<Snapshot[]> {
  return request<Snapshot[]>(`/v1/environments/${envId}/snapshots`);
}

export async function createSnapshot(
  envId: string,
  body?: CreateSnapshotRequest,
): Promise<Snapshot> {
  return request<Snapshot>(`/v1/environments/${envId}/snapshots`, {
    method: "POST",
    body: JSON.stringify(body || {}),
  });
}

export async function deleteSnapshot(
  envId: string,
  snapshotId: string,
): Promise<void> {
  return request<void>(
    `/v1/environments/${envId}/snapshots/${snapshotId}`,
    { method: "DELETE" },
  );
}

export async function restoreSnapshot(
  envId: string,
  snapshotId: string,
): Promise<Environment> {
  return request<Environment>(
    `/v1/environments/${envId}/snapshots/${snapshotId}/restore`,
    { method: "POST" },
  );
}

// ── Tasks ──────────────────────────────────────────────────────────

export async function listTasks(params?: PaginationParams): Promise<Task[]> {
  return request<Task[]>(`/v1/tasks${queryString(params)}`);
}

export async function getTask(taskId: string): Promise<Task> {
  return request<Task>(`/v1/tasks/${taskId}`);
}

export async function listEnvironmentTasks(
  envId: string,
  params?: PaginationParams,
): Promise<Task[]> {
  return request<Task[]>(
    `/v1/environments/${envId}/tasks${queryString(params)}`,
  );
}

// ── Events ─────────────────────────────────────────────────────────

export async function listEnvironmentEvents(
  envId: string,
  params?: PaginationParams,
): Promise<EnvironmentEvent[]> {
  return request<EnvironmentEvent[]>(
    `/v1/environments/${envId}/events${queryString(params)}`,
  );
}

// ── Health ─────────────────────────────────────────────────────────

export async function healthCheck(): Promise<void> {
  return request<void>("/v1/healthz", {
    headers: { "Content-Type": "application/json" },
  });
}

// Namespace object for convenience imports: `import { api } from "@/lib/api"`
export const api = {
  getAuthProviders,
  login,
  getMe,
  webauthnRegisterBegin,
  webauthnRegisterComplete,
  webauthnLoginBegin,
  webauthnLoginComplete,
  totpRegister,
  totpVerify,
  totpLogin,
  listUsers,
  getUser,
  updateUserRole,
  setUserDisabled,
  listUserSshKeys,
  listSshKeys,
  addSshKey,
  deleteSshKey,
  listNodes,
  getNode,
  registerNode,
  updateNode,
  deleteNode,
  nodeHeartbeat,
  getNodeUsage,
  listImages,
  getImage,
  registerImage,
  deleteImage,
  listEnvironments,
  getEnvironment,
  createEnvironment,
  deleteEnvironment,
  destroyEnvironment: deleteEnvironment,
  updateEnvironmentState,
  getSshEndpoint,
  getVncEndpoint,
  suspendEnvironment,
  resumeEnvironment,
  rebootEnvironment,
  forceRebootEnvironment,
  restartEnvironment,
  migrateEnvironment,
  renameEnvironment,
  togglePortForwarding,
  extendEnvironmentTtl,
  listSnapshots,
  createSnapshot,
  deleteSnapshot,
  restoreSnapshot,
  listTasks,
  getTask,
  listEnvironmentTasks,
  listEnvironmentEvents,
  healthCheck,
};
