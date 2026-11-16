-- Multi-tenant workgroups, role hierarchy, cross-group delegations and three-tier quotas
CREATE TABLE IF NOT EXISTS workgroups (
    id VARCHAR(128) PRIMARY KEY,
    name VARCHAR(160) NOT NULL,
    max_concurrent_boxes INTEGER NOT NULL DEFAULT 5,
    max_subagent_depth SMALLINT NOT NULL DEFAULT 2,
    max_subagents_per_tree SMALLINT NOT NULL DEFAULT 5,
    internal_tokens_quota BIGINT NOT NULL DEFAULT 10000000,
    dollar_budget_cents BIGINT NOT NULL DEFAULT 0,
    fail_closed_on_budget BOOLEAN NOT NULL DEFAULT FALSE,
    allowed_storage_mb BIGINT NOT NULL DEFAULT 5120,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS workgroup_members (
    workgroup_id VARCHAR(128) NOT NULL REFERENCES workgroups(id) ON DELETE CASCADE,
    user_id VARCHAR(128) NOT NULL,
    role VARCHAR(32) NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (workgroup_id, user_id)
);

CREATE TABLE IF NOT EXISTS workgroup_delegations (
    delegation_id VARCHAR(128) PRIMARY KEY,
    source_workgroup_id VARCHAR(128) NOT NULL REFERENCES workgroups(id) ON DELETE CASCADE,
    target_workgroup_id VARCHAR(128) NOT NULL REFERENCES workgroups(id) ON DELETE CASCADE,
    agent_role_slug VARCHAR(64) NOT NULL,
    max_delegated_boxes INTEGER NOT NULL DEFAULT 1,
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workgroup_members_user ON workgroup_members(user_id);
CREATE INDEX IF NOT EXISTS idx_workgroup_delegations_target ON workgroup_delegations(target_workgroup_id);
