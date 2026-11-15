use garde::Validate;
use ironic::prelude::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Request payload to create or update a dynamic inference provider.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertProviderDto {
    #[garde(length(min = 1, max = 64))]
    pub id: String,
    #[garde(length(min = 1, max = 128))]
    pub name: String,
    /// Provider protocol kind: `ollama`, `openai`, `anthropic`, `deepinfra`, `deepseek`, or `custom`.
    #[garde(length(min = 1, max = 32))]
    pub kind: String,
    #[garde(length(min = 1, max = 512))]
    pub base_url: String,
    #[garde(length(min = 1, max = 256))]
    pub default_model: String,
    /// Optional plaintext API key (encrypted immediately with Argon2id + AES-256-GCM; never returned).
    #[garde(skip)]
    #[serde(default)]
    pub api_key: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub is_default: bool,
    #[garde(skip)]
    #[serde(default)]
    pub enabled: Option<bool>,
}
