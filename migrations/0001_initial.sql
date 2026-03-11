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
 user_id TEXT,
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
 base_image_id TEXT,
 created_at INTEGER
);

CREATE TABLE IF NOT EXISTS environments (
 id TEXT PRIMARY KEY,
 image_id TEXT,
 owner_user_id TEXT,
 node_id TEXT,
 provider TEXT,
 guest_os TEXT,
 guest_arch TEXT,
 state TEXT,
 created_at INTEGER,
 expires_at INTEGER
);

CREATE TABLE IF NOT EXISTS snapshots (
 id TEXT PRIMARY KEY,
 env_id TEXT,
 created_at INTEGER
);

CREATE TABLE IF NOT EXISTS tasks (
 id TEXT PRIMARY KEY,
 kind TEXT,
 state TEXT,
 created_at INTEGER
);
