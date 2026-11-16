use agents_protocol::{ClassifiedProviderError, DiscoveredModel, ProviderProtocol};
use ironic::prelude::OpenApiSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct InferenceProviderEntity {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub fallback_provider_id: Option<String>,
    pub has_api_key: bool,
    pub key_fingerprint: Option<String>,
    pub is_default: bool,
    pub enabled: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderActionResponse {
    pub ok: bool,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsCatalogResponse {
    pub provider_id: String,
    pub protocol: ProviderProtocol,
    pub cached: bool,
    pub allow_freeform_override: bool,
    pub models: Vec<DiscoveredModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHandshakeResponse {
    pub provider_id: String,
    pub reachable: bool,
    pub latency_ms: u64,
    #[serde(default)]
    pub error_classification: Option<ClassifiedProviderError>,
}
