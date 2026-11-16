//! DTOs for task artifact uploads, listing, and temporary presigned URLs.

use garde::Validate;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Request payload to upload an agent-produced artifact for a task.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadArtifactDto {
    #[garde(length(min = 1, max = 160))]
    pub filename: String,
    #[serde(default)]
    #[garde(skip)]
    pub content_type: Option<String>,
    #[garde(length(min = 1))]
    pub data_base64: String,
    #[serde(default)]
    #[garde(skip)]
    pub ttl_secs: Option<u64>,
}

/// Optional query parameters controlling temporary presigned URL expiration.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PresignQueryDto {
    #[garde(skip)]
    pub ttl: Option<u64>,
}

/// Stored artifact metadata paired with a time-bounded presigned URL.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: String,
    pub task_id: String,
    pub session_id: String,
    pub key: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub presigned_url: String,
    pub expires_in_secs: u64,
    pub created_at_ms: u64,
}

/// Response listing all artifacts produced by a task.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactListResponse {
    pub task_id: String,
    pub items: Vec<ArtifactRecord>,
}

/// Response returned when generating a temporary presigned download URL.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresignedArtifactResponse {
    pub key: String,
    pub url: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeliverVerifiedArtifactDto {
    #[garde(length(min = 1, max = 160))]
    pub filename: String,
    #[garde(skip)]
    #[serde(default)]
    pub content_type: Option<String>,
    #[garde(length(min = 1))]
    pub data_base64: String,
    #[garde(length(min = 64, max = 64))]
    pub expected_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedDeliveryReceipt {
    pub receipt_id: String,
    pub task_id: String,
    pub session_id: String,
    pub filename: String,
    pub sha256_digest: String,
    pub ref_count: u64,
    pub delivery_signature: String,
    pub presigned_url: String,
    pub delivered_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaBlobRefRecord {
    pub content_sha256: String,
    pub key: String,
    pub size_bytes: u64,
    pub ref_count: u64,
    pub marked_orphan_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordTaskPhaseDto {
    #[garde(length(min = 1, max = 64))]
    pub phase_name: String,
    #[garde(length(min = 1, max = 128))]
    pub output_sha256: String,
    #[garde(range(min = 0))]
    pub fencing_token: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskPhaseCheckpoint {
    pub task_id: String,
    pub phase_name: String,
    pub output_sha256: String,
    pub fencing_token: u64,
    pub recorded_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskCrashResumePlan {
    pub task_id: String,
    pub active_fencing_token: u64,
    pub completed_phases: Vec<TaskPhaseCheckpoint>,
    pub next_pending_phase: Option<String>,
}
