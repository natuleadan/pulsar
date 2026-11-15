use garde::Validate;
use ironic::prelude::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Request payload to store an isolated user/agent/worker/group secret.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSecretDto {
    #[garde(length(min = 1, max = 64))]
    pub id: String,
    #[garde(length(min = 1, max = 128))]
    pub name: String,
    /// Isolation scope: `owner_shared`, `group`, `agent`, or `worker`.
    #[garde(length(min = 1, max = 32))]
    pub scope_type: String,
    #[garde(skip)]
    #[serde(default)]
    pub scope_id: Option<String>,
    #[garde(length(min = 1, max = 256))]
    pub target_host: String,
    #[garde(skip)]
    #[serde(default)]
    pub header_name: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub header_prefix: Option<String>,
    #[garde(length(min = 1, max = 4096))]
    pub value: String,
}

/// Request payload to create a shared secret group.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateSecretGroupDto {
    #[garde(length(min = 1, max = 64))]
    pub id: String,
    #[garde(length(min = 1, max = 128))]
    pub name: String,
    #[garde(skip)]
    #[serde(default)]
    pub description: Option<String>,
}

/// Request payload to bind an agent, worker, or coordinator to a secret group.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddGroupMemberDto {
    /// `agent`, `worker`, or `coordinator`.
    #[garde(length(min = 1, max = 32))]
    pub member_kind: String,
    #[garde(length(min = 1, max = 128))]
    pub member_id: String,
}
