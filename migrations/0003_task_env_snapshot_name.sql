-- Add env_id to tasks for environment-scoped queries
ALTER TABLE tasks ADD COLUMN env_id TEXT REFERENCES environments(id);
CREATE INDEX IF NOT EXISTS idx_tasks_env_id ON tasks(env_id);

-- Add name to snapshots for user-friendly naming
ALTER TABLE snapshots ADD COLUMN name TEXT;
