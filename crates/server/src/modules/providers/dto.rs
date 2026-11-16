use garde::Validate;
use ironic::prelude::OpenApiSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertProviderDto {
    #[garde(length(min = 1, max = 64))]
    pub id: String,
    #[garde(length(min = 1, max = 128))]
    pub name: String,
    #[garde(length(min = 1, max = 32))]
    pub kind: String,
    #[garde(length(min = 1, max = 512))]
    pub base_url: String,
    #[garde(length(min = 1, max = 256))]
    pub default_model: String,
    #[garde(skip)]
    #[serde(default)]
    pub fallback_provider_id: Option<String>,
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
