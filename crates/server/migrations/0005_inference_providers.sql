-- Dynamic inference providers with Argon2id + AES-256-GCM encrypted API keys.

CREATE TABLE IF NOT EXISTS inference_providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    base_url TEXT NOT NULL,
    default_model TEXT NOT NULL,
    encrypted_api_key TEXT,
    key_fingerprint TEXT,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_inference_providers_default
    ON inference_providers (is_default, enabled);
