-- Multi-level isolated user/agent/worker secrets and shared secret groups.
-- System orchestrator secrets (.env) are strictly excluded from this table.

CREATE TABLE IF NOT EXISTS secret_groups (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (owner_user_id, name)
);

CREATE TABLE IF NOT EXISTS secret_group_members (
    group_id TEXT NOT NULL REFERENCES secret_groups(id) ON DELETE CASCADE,
    member_kind TEXT NOT NULL CHECK (member_kind IN ('agent', 'worker', 'coordinator')),
    member_id TEXT NOT NULL,
    PRIMARY KEY (group_id, member_kind, member_id)
);

CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    scope_type TEXT NOT NULL CHECK (scope_type IN ('owner_shared', 'group', 'agent', 'worker')),
    scope_id TEXT,
    target_host TEXT NOT NULL,
    header_name TEXT NOT NULL DEFAULT 'authorization',
    header_prefix TEXT NOT NULL DEFAULT 'Bearer ',
    encrypted_value TEXT NOT NULL,
    value_fingerprint TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_secrets_owner_scope
    ON secrets (owner_user_id, scope_type, scope_id);
