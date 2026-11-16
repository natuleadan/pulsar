//! Shared installation-level agent identity headers and validation helpers.

use serde::{Deserialize, Serialize};

/// HTTP header carrying the organization name that owns the agent installation.
pub const HEADER_AGENT_ORGANIZATION: &str = "x-agent-organization";

/// HTTP header carrying the unique alphanumeric installation identifier.
pub const HEADER_AGENT_INSTALLATION_ID: &str = "x-agent-installation-id";

/// HTTP header carrying the optional signed identity certificate/code bound to the installation IP.
pub const HEADER_AGENT_IDENTITY_CODE: &str = "x-agent-identity-code";

/// Validates that an installation identifier is non-empty, 4..=64 ASCII alphanumeric/dash/underscore chars.
#[must_use]
pub fn is_valid_alphanumeric_id(value: &str) -> bool {
    let trimmed = value.trim();
    (4..=64).contains(&trimmed.len())
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Validates that an organization identifier is non-empty and within 2..=128 printable ASCII chars.
#[must_use]
pub fn is_valid_organization_name(value: &str) -> bool {
    let trimmed = value.trim();
    (2..=128).contains(&trimmed.len()) && trimmed.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
}

/// Installation-wide identity headers injected on outbound traffic and inspected on inbound traffic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationIdentityHeaders {
    pub organization: String,
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_code: Option<String>,
}

impl InstallationIdentityHeaders {
    /// Returns `true` when both organization name and alphanumeric installation ID are well-formed.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        is_valid_organization_name(&self.organization)
            && is_valid_alphanumeric_id(&self.installation_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkgroupId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BoxId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GenerationId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoTierSessionBinding {
    pub session_id: SessionId,
    pub workgroup_id: WorkgroupId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_box_id: Option<BoxId>,
    pub generation_id: GenerationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nats_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ws_connection_id: Option<String>,
}

impl TwoTierSessionBinding {
    #[must_use]
    pub fn new(session_id: impl Into<String>, workgroup_id: impl Into<String>) -> Self {
        Self {
            session_id: SessionId(session_id.into()),
            workgroup_id: WorkgroupId(workgroup_id.into()),
            active_box_id: None,
            generation_id: GenerationId(1),
            nats_subject: None,
            ws_connection_id: None,
        }
    }

    pub fn rebind_ephemeral(
        &mut self,
        box_id: impl Into<String>,
        nats_subject: impl Into<String>,
        ws_connection_id: Option<String>,
    ) {
        self.active_box_id = Some(BoxId(box_id.into()));
        self.generation_id = GenerationId(self.generation_id.0.saturating_add(1));
        self.nats_subject = Some(nats_subject.into());
        self.ws_connection_id = ws_connection_id;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateInvariantError {
    CompletionWithoutOutput { task_id: String },
    QuotaExhaustedDispatch { worker_id: String, reason: String },
    CrossTenantCollision { entity_id: String },
    StaleGeneration { expected: u64, received: u64 },
}

impl std::fmt::Display for StateInvariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CompletionWithoutOutput { task_id } => {
                write!(
                    f,
                    "task `{task_id}` cannot transition to completed without assistant message or tool result"
                )
            }
            Self::QuotaExhaustedDispatch { worker_id, reason } => {
                write!(
                    f,
                    "worker `{worker_id}` cannot accept task: quota exhausted ({reason})"
                )
            }
            Self::CrossTenantCollision { entity_id } => {
                write!(
                    f,
                    "public identifier `{entity_id}` collided across distinct tenant workgroups"
                )
            }
            Self::StaleGeneration { expected, received } => {
                write!(
                    f,
                    "stale generation `{received}` rejected (current generation is `{expected}`)"
                )
            }
        }
    }
}

impl std::error::Error for StateInvariantError {}

pub fn validate_turn_completion_invariant(
    task_id: &str,
    target_state: &crate::task::TaskState,
    emitted_messages: usize,
    emitted_tool_results: usize,
) -> Result<(), StateInvariantError> {
    if *target_state == crate::task::TaskState::Done
        && emitted_messages == 0
        && emitted_tool_results == 0
    {
        return Err(StateInvariantError::CompletionWithoutOutput {
            task_id: task_id.to_owned(),
        });
    }
    Ok(())
}

pub fn validate_worker_quota_invariant(
    worker_id: &str,
    remaining_tokens: u64,
    remaining_memory_mb: u64,
) -> Result<(), StateInvariantError> {
    if remaining_tokens == 0 {
        return Err(StateInvariantError::QuotaExhaustedDispatch {
            worker_id: worker_id.to_owned(),
            reason: "zero token balance".to_owned(),
        });
    }
    if remaining_memory_mb == 0 {
        return Err(StateInvariantError::QuotaExhaustedDispatch {
            worker_id: worker_id.to_owned(),
            reason: "zero memory budget".to_owned(),
        });
    }
    Ok(())
}

pub fn validate_tenant_isolation_invariant(
    entity_id: &str,
    owner_workgroup: &WorkgroupId,
    requesting_workgroup: &WorkgroupId,
) -> Result<(), StateInvariantError> {
    if owner_workgroup != requesting_workgroup {
        return Err(StateInvariantError::CrossTenantCollision {
            entity_id: entity_id.to_owned(),
        });
    }
    Ok(())
}
