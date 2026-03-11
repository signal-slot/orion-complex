-- Per-environment resource allocation
ALTER TABLE environments ADD COLUMN vcpus INTEGER;
ALTER TABLE environments ADD COLUMN memory_bytes INTEGER;
ALTER TABLE environments ADD COLUMN disk_bytes INTEGER;

-- Task error tracking
ALTER TABLE tasks ADD COLUMN error_message TEXT;
ALTER TABLE tasks ADD COLUMN completed_at INTEGER;

-- Environment event log / audit trail
CREATE TABLE IF NOT EXISTS environment_events (
 id TEXT PRIMARY KEY,
 env_id TEXT NOT NULL,
 event_type TEXT NOT NULL,
 from_state TEXT,
 to_state TEXT,
 triggered_by TEXT,
 message TEXT,
 created_at INTEGER NOT NULL
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_environments_owner ON environments(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_environments_node ON environments(node_id);
CREATE INDEX IF NOT EXISTS idx_environments_state ON environments(state);
CREATE INDEX IF NOT EXISTS idx_env_events_env_id ON environment_events(env_id);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state);
CREATE INDEX IF NOT EXISTS idx_user_ssh_keys_user ON user_ssh_keys(user_id);
