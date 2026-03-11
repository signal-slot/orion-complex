-- Reference schema for orion-complex.
-- The canonical source of truth is migrations/ — this file is for documentation only.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  provider TEXT,
  provider_subject TEXT,
  email TEXT,
  email_domain TEXT,
  display_name TEXT,
  role TEXT,
  disabled INTEGER DEFAULT 0,
  created_at INTEGER
);

CREATE TABLE IF NOT EXISTS user_ssh_keys (
  id TEXT PRIMARY KEY,
  user_id TEXT REFERENCES users(id),
  public_key TEXT,
  fingerprint TEXT,
  created_at INTEGER
);

CREATE TABLE IF NOT EXISTS nodes (
  id TEXT PRIMARY KEY,
  name TEXT,
  host_os TEXT,
  host_arch TEXT,
  cpu_cores INTEGER,
  memory_bytes INTEGER,
  disk_bytes_total INTEGER,
  max_cpu_utilization_ratio REAL,
  max_memory_utilization_ratio REAL,
  max_disk_utilization_ratio REAL,
  max_running_envs INTEGER,
  online INTEGER,
  last_heartbeat_at INTEGER
);

CREATE TABLE IF NOT EXISTS images (
  id TEXT PRIMARY KEY,
  name TEXT,
  provider TEXT,
  guest_os TEXT,
  guest_arch TEXT,
  base_image_id TEXT REFERENCES images(id),
  created_at INTEGER
);

CREATE TABLE IF NOT EXISTS environments (
  id TEXT PRIMARY KEY,
  image_id TEXT REFERENCES images(id),
  owner_user_id TEXT REFERENCES users(id),
  node_id TEXT REFERENCES nodes(id),
  provider TEXT,
  guest_os TEXT,
  guest_arch TEXT,
  state TEXT,
  created_at INTEGER,
  expires_at INTEGER,
  vcpus INTEGER,
  memory_bytes INTEGER,
  disk_bytes INTEGER
);

CREATE TABLE IF NOT EXISTS snapshots (
  id TEXT PRIMARY KEY,
  env_id TEXT REFERENCES environments(id),
  name TEXT,
  created_at INTEGER
);

CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  kind TEXT,
  state TEXT,
  created_at INTEGER,
  error_message TEXT,
  completed_at INTEGER,
  env_id TEXT REFERENCES environments(id)
);

CREATE TABLE IF NOT EXISTS environment_events (
  id TEXT PRIMARY KEY,
  env_id TEXT NOT NULL REFERENCES environments(id),
  event_type TEXT NOT NULL,
  from_state TEXT,
  to_state TEXT,
  triggered_by TEXT,
  message TEXT,
  created_at INTEGER NOT NULL
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_environments_node_id ON environments(node_id);
CREATE INDEX IF NOT EXISTS idx_environments_owner ON environments(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_environments_state ON environments(state);
CREATE INDEX IF NOT EXISTS idx_environment_events_env_id ON environment_events(env_id);
CREATE INDEX IF NOT EXISTS idx_tasks_env_id ON tasks(env_id);
