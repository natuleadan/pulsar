//! Validated DTOs for multi-tenant workgroups, role hierarchy, quotas, and task pricing quotes.

use garde::Validate;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Role hierarchy inside a workgroup (OWASP Multi-Tenancy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkgroupRole {
    SuperAdmin,
    WorkgroupAdmin,
    HumanOperator,
    Viewer,
}

impl OpenApiSchema for WorkgroupRole {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "WorkgroupRole",
            "enum": ["super_admin", "workgroup_admin", "human_operator", "viewer"],
            "example": "human_operator"
        })
    }
}

impl WorkgroupRole {
    #[must_use]
    pub const fn can_manage_quotas(self) -> bool {
        matches!(self, Self::SuperAdmin | Self::WorkgroupAdmin)
    }

    #[must_use]
    pub const fn can_dispatch_tasks(self) -> bool {
        matches!(
            self,
            Self::SuperAdmin | Self::WorkgroupAdmin | Self::HumanOperator
        )
    }
}

/// Request to create a new multi-tenant workgroup.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkgroupDto {
    #[garde(length(min = 2, max = 160))]
    pub name: String,
    #[garde(range(min = 1, max = 128))]
    #[serde(default)]
    pub max_concurrent_boxes: Option<i32>,
    #[garde(range(min = 1, max = 3))]
    #[serde(default)]
    pub max_subagent_depth: Option<i16>,
    #[garde(range(min = 1, max = 16))]
    #[serde(default)]
    pub max_subagents_per_tree: Option<i16>,
    #[garde(range(min = 100_000, max = 1_000_000_000))]
    #[serde(default)]
    pub internal_tokens_quota: Option<i64>,
    #[garde(range(min = 0, max = 100_000_000))]
    #[serde(default)]
    pub dollar_budget_cents: Option<i64>,
    #[garde(skip)]
    #[serde(default)]
    pub fail_closed_on_budget: bool,
    #[garde(range(min = 128, max = 1_048_576))]
    #[serde(default)]
    pub allowed_storage_mb: Option<i64>,
}

/// Request to update workgroup compute limits and third-party financial budget.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkgroupLimitsDto {
    #[garde(range(min = 1, max = 128))]
    pub max_concurrent_boxes: Option<i32>,
    #[garde(range(min = 1, max = 3))]
    pub max_subagent_depth: Option<i16>,
    #[garde(range(min = 1, max = 16))]
    pub max_subagents_per_tree: Option<i16>,
    #[garde(range(min = 100_000, max = 1_000_000_000))]
    pub internal_tokens_quota: Option<i64>,
    #[garde(range(min = 0, max = 100_000_000))]
    pub dollar_budget_cents: Option<i64>,
    #[garde(skip)]
    pub fail_closed_on_budget: Option<bool>,
    #[garde(range(min = 128, max = 1_048_576))]
    pub allowed_storage_mb: Option<i64>,
}

/// Request to add or update a member in a workgroup.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddWorkgroupMemberDto {
    #[garde(length(min = 1, max = 128))]
    pub user_id: String,
    #[garde(skip)]
    pub role: WorkgroupRole,
}

/// Request to authorize cross-group agent sharing from pool.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateDelegationDto {
    #[garde(length(min = 1, max = 128))]
    pub target_workgroup_id: String,
    #[garde(length(min = 1, max = 64))]
    pub agent_role_slug: String,
    #[garde(range(min = 1, max = 16))]
    pub max_delegated_boxes: i32,
    #[garde(range(min = 60, max = 2_592_000))]
    pub duration_seconds: u64,
}

/// Multi-tenant workgroup entity.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkgroupEntity {
    pub id: String,
    pub name: String,
    pub max_concurrent_boxes: i32,
    pub max_subagent_depth: i16,
    pub max_subagents_per_tree: i16,
    pub internal_tokens_quota: i64,
    pub dollar_budget_cents: i64,
    pub fail_closed_on_budget: bool,
    pub allowed_storage_mb: i64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Real-time usage statistics for a workgroup.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkgroupUsageResponse {
    pub workgroup_id: String,
    pub active_boxes: i32,
    pub max_concurrent_boxes: i32,
    pub tokens_consumed: i64,
    pub tokens_quota: i64,
    pub spent_dollars_cents: i64,
    pub dollar_budget_cents: i64,
    pub budget_exhausted: bool,
    pub delegated_in_boxes: i32,
}

/// Coordinator pricing quote request for incoming task monetization.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestTaskQuoteDto {
    #[garde(length(min = 1, max = 128))]
    pub task_id: String,
    #[garde(length(min = 1, max = 32_768))]
    pub prompt: String,
    #[garde(range(min = 1, max = 100))]
    #[serde(default)]
    pub estimated_steps: Option<u32>,
    #[garde(skip)]
    #[serde(default)]
    pub sandbox_required: bool,
}

/// Coordinator task quote with cryptographic signature and expiration.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskQuoteResponse {
    pub quote_id: String,
    pub task_id: String,
    pub amount_credits: u64,
    pub amount_cents: u64,
    pub currency: String,
    pub expires_at: u64,
    pub signature: String,
}

/// Payment checkout verification or ledger debit.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckoutDto {
    #[garde(length(min = 1, max = 128))]
    pub quote_id: String,
    #[garde(length(min = 1, max = 128))]
    pub task_id: String,
    #[garde(length(min = 1, max = 256))]
    pub signature: String,
    #[garde(length(max = 128))]
    #[serde(default)]
    pub payment_proof: Option<String>,
}

/// Settlement confirmation response unblocking task to pending.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckoutResponse {
    pub task_id: String,
    pub settled: bool,
    pub state: String,
    pub receipt_id: String,
    pub settled_at: u64,
}
