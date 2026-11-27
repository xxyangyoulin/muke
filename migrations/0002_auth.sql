CREATE TABLE sessions (
 id TEXT PRIMARY KEY, csrf TEXT NOT NULL, user_id INTEGER,
 expires_at INTEGER NOT NULL
);
CREATE INDEX idx_sessions_expiry ON sessions(expires_at);
CREATE TABLE rate_limits (
 scope TEXT NOT NULL, identity TEXT NOT NULL, started_at INTEGER NOT NULL,
 attempts INTEGER NOT NULL, PRIMARY KEY(scope,identity)
);
