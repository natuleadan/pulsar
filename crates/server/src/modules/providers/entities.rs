use ironic::prelude::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Public provider metadata returned by API (never includes plaintext or encrypted API keys).
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct InferenceProviderEntity {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub default_model: String,
    pub has_api_key: bool,
    pub key_fingerprint: Option<String>,
    pub is_default: bool,
    pub enabled: bool,
    pub updated_at: String,
}

/// Confirmation response for provider mutation actions.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderActionResponse {
    pub ok: bool,
    pub id: String,
}
