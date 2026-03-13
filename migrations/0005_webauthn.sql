CREATE TABLE IF NOT EXISTS webauthn_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id TEXT NOT NULL UNIQUE,
    credential_json TEXT NOT NULL,
    name TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_webauthn_credentials_user_id ON webauthn_credentials(user_id);
CREATE INDEX idx_webauthn_credentials_credential_id ON webauthn_credentials(credential_id);

CREATE TABLE IF NOT EXISTS webauthn_challenges (
    id TEXT PRIMARY KEY,
    challenge_type TEXT NOT NULL,
    state_json TEXT NOT NULL,
    user_id TEXT,
    email TEXT,
    display_name TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
