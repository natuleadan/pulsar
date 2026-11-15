use ironic::prelude::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Non-readable secret metadata returned to users and agents (never exposes plaintext or ciphertext).
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretMetadataEntity {
    pub id: String,
    pub name: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub target_host: String,
    pub header_name: String,
    pub has_value: bool,
    pub value_fingerprint: String,
    pub updated_at: String,
}

/// Secret group metadata.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretGroupEntity {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
}

/// Confirmation response for secret mutations.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretActionResponse {
    pub ok: bool,
    pub id: String,
}
