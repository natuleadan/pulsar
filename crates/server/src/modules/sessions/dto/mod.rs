//! Validated DTOs for session lifecycle, tri-state filtering, and local draft recovery.

use agents_protocol::{InteractionMode, TriStateFilter};
use garde::Validate;
use ironic::OpenApiSchema;
use serde::Deserialize;

use crate::infra::error::ServerError;

/// Request payload to create a new session.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionDto {
    #[garde(length(min = 1, max = 160))]
    pub title: String,
    #[garde(skip)]
    pub mode: Option<InteractionMode>,
}

/// Request payload to update session title, interaction mode, or archive state.
#[derive(Debug, Clone, Default, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionDto {
    #[garde(length(min = 1, max = 160))]
    pub title: Option<String>,
    #[garde(skip)]
    pub mode: Option<InteractionMode>,
    #[garde(skip)]
    pub archived: Option<bool>,
    #[garde(range(min = 1))]
    pub expected_version: Option<u64>,
}

/// Query parameters for cursor-paginated session listing with tri-state `archived` filter.
#[derive(Debug, Clone, Default, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsQuery {
    #[garde(length(max = 128))]
    pub cursor: Option<String>,
    #[garde(range(min = 1, max = 100))]
    pub limit: Option<u32>,
    /// Supports omitted (`Any`), `"null"` (`None`), `"true"`/`"false"` (`Exact`).
    #[garde(length(max = 16))]
    pub archived: Option<String>,
}

impl ListSessionsQuery {
    /// Parses `archived` string into a protocol [`TriStateFilter<bool>`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] for unrecognized filter tokens.
    pub fn archived_filter(&self) -> Result<TriStateFilter<bool>, ServerError> {
        match self.archived.as_deref().map(str::trim) {
            None | Some("") => Ok(TriStateFilter::Any),
            Some("null") => Ok(TriStateFilter::None),
            Some("true") => Ok(TriStateFilter::Exact(true)),
            Some("false") => Ok(TriStateFilter::Exact(false)),
            Some(other) => Err(ServerError::Validation(format!(
                "invalid archived filter `{other}`; expected null, true, or false"
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveDraftDto {
    #[garde(length(max = 32_768))]
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApproveSessionPlanDto {
    #[garde(skip)]
    pub approved: bool,
    #[garde(length(max = 2048))]
    pub feedback: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanApprovalResponse {
    pub session_id: String,
    pub mode: InteractionMode,
    pub approved: bool,
    pub approved_by: String,
    pub approved_at: u64,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContinueTurnDto {
    #[garde(range(min = 1, max = 100))]
    pub additional_steps: Option<u32>,
    #[garde(length(max = 4096))]
    pub directive: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContinueTurnResponse {
    pub session_id: String,
    pub resumed: bool,
    pub granted_steps: u32,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppendSessionEventDto {
    #[garde(range(min = 1))]
    pub turn_seq: u32,
    #[garde(length(min = 1, max = 64))]
    pub event_kind: String,
    #[garde(length(min = 1, max = 32_768))]
    pub content: String,
    #[garde(length(max = 128))]
    #[serde(default)]
    pub tool_name: Option<String>,
    #[garde(length(max = 128))]
    #[serde(default)]
    pub vfs_commit_sha: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventLogEntry {
    pub session_id: String,
    pub sequence_number: u64,
    pub turn_seq: u32,
    pub event_kind: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub vfs_commit_sha: Option<String>,
    pub recorded_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedConversationSnapshot {
    pub session_id: String,
    pub latest_sequence_number: u64,
    pub active_turn_seq: u32,
    pub latest_vfs_commit_sha: Option<String>,
    pub events: Vec<SessionEventLogEntry>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevertSessionDto {
    #[garde(range(min = 0))]
    pub target_turn_seq: u32,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevertSessionResponse {
    pub session_id: String,
    pub restored_turn_seq: u32,
    pub pruned_events: usize,
    pub restored_vfs_commit_sha: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionExportBundle {
    pub session_id: String,
    pub title: String,
    pub mode: InteractionMode,
    pub jsonl: String,
    pub event_count: usize,
    pub exported_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSessionDto {
    #[garde(length(max = 128))]
    #[serde(default)]
    pub preferred_session_id: Option<String>,
    #[garde(length(min = 1, max = 160))]
    pub title: String,
    #[garde(skip)]
    #[serde(default)]
    pub mode: Option<InteractionMode>,
    #[garde(length(min = 2, max = 524_288))]
    pub jsonl: String,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSessionResponse {
    pub session_id: String,
    pub remapped_from_id: Option<String>,
    pub imported_events: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchSessionsQuery {
    #[garde(length(min = 1, max = 256))]
    pub q: String,
    #[garde(length(max = 64))]
    #[serde(default)]
    pub tool: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchHit {
    pub session_id: String,
    pub title: String,
    pub matched_excerpt: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionArchiveResponse {
    pub session_id: String,
    pub archived: bool,
    pub archive_key: String,
    pub raw_bytes: usize,
    pub compressed_bytes: usize,
    pub restored_events: usize,
}
