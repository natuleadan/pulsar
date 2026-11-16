//! Multi-agent swarm coordination contracts and OWASP ASI-07 permission narrowing.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Errors raised when a delegated child agent requests broader permissions than its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionError {
    PrivilegeEscalation { capability: &'static str },
    DisallowedTool { tool: String },
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrivilegeEscalation { capability } => {
                write!(
                    f,
                    "child agent cannot escalate '{capability}' beyond parent"
                )
            }
            Self::DisallowedTool { tool } => {
                write!(f, "child agent requested tool '{tool}' outside parent mask")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

/// Capability mask enforced across parent-to-child swarm delegations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionMask {
    pub can_read_fs: bool,
    pub can_write_fs: bool,
    pub can_exec_shell: bool,
    pub can_network: bool,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

impl PermissionMask {
    /// Verifies `requested` is strictly a subset of `self`, failing closed on escalation.
    pub fn narrow(&self, requested: &Self) -> Result<Self, PermissionError> {
        if requested.can_read_fs && !self.can_read_fs {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_read_fs",
            });
        }
        if requested.can_write_fs && !self.can_write_fs {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_write_fs",
            });
        }
        if requested.can_exec_shell && !self.can_exec_shell {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_exec_shell",
            });
        }
        if requested.can_network && !self.can_network {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_network",
            });
        }

        let parent_tools: BTreeSet<&str> = self.allowed_tools.iter().map(String::as_str).collect();
        for tool in &requested.allowed_tools {
            if !parent_tools.contains(tool.as_str()) {
                return Err(PermissionError::DisallowedTool { tool: tool.clone() });
            }
        }

        Ok(requested.clone())
    }
}

/// Target routing destination on the NATS swarm mesh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "agentId", rename_all = "camelCase")]
pub enum SwarmTopic {
    Broadcast,
    DirectAgent(String),
}

/// Signed swarm coordination message exchanged via NATS JetStream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmMessage {
    pub message_id: String,
    pub session_id: String,
    pub from_agent_id: String,
    pub to: SwarmTopic,
    pub payload: Value,
    pub permission_mask: PermissionMask,
    pub signature: String,
    pub emitted_at_ms: u64,
}

/// Bounded subagent delegation descriptor with optional isolated git worktree branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoff {
    pub handoff_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    pub delegated_mask: PermissionMask,
    pub goal: String,
}

pub const MAX_SUBAGENT_DEPTH: u8 = 2;
pub const MAX_CONCURRENT_SUBAGENTS_PER_TREE: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentLineage {
    pub root_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    pub current_agent_id: String,
    pub depth: u8,
    #[serde(default)]
    pub call_path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubagentInceptionError {
    DepthExceeded {
        requested_depth: u8,
        max_depth: u8,
    },
    TreeCapacityExceeded {
        active_count: usize,
        max_capacity: usize,
    },
    CycleDetected {
        target_agent_id: String,
    },
    TokenBudgetExhausted {
        remaining_tokens: u64,
    },
}

impl fmt::Display for SubagentInceptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DepthExceeded {
                requested_depth,
                max_depth,
            } => write!(
                f,
                "SUBAGENT_DEPTH_EXCEEDED: requested depth {requested_depth} exceeds max {max_depth}"
            ),
            Self::TreeCapacityExceeded {
                active_count,
                max_capacity,
            } => write!(
                f,
                "SUBAGENT_TREE_CAPACITY_EXCEEDED: active {active_count} reaches max {max_capacity}"
            ),
            Self::CycleDetected { target_agent_id } => write!(
                f,
                "SUBAGENT_CYCLE_DETECTED: target `{target_agent_id}` is already in ancestor call path"
            ),
            Self::TokenBudgetExhausted { remaining_tokens } => write!(
                f,
                "SUBAGENT_BUDGET_EXHAUSTED: remaining session tokens ({remaining_tokens}) insufficient"
            ),
        }
    }
}

impl std::error::Error for SubagentInceptionError {}

pub fn validate_subagent_spawn(
    parent_lineage: &SubagentLineage,
    target_agent_id: &str,
    active_subagents_in_tree: usize,
    remaining_session_tokens: u64,
) -> Result<SubagentLineage, SubagentInceptionError> {
    let next_depth = parent_lineage.depth.saturating_add(1);
    if next_depth > MAX_SUBAGENT_DEPTH {
        return Err(SubagentInceptionError::DepthExceeded {
            requested_depth: next_depth,
            max_depth: MAX_SUBAGENT_DEPTH,
        });
    }
    if parent_lineage
        .call_path
        .iter()
        .any(|id| id == target_agent_id)
        || parent_lineage.current_agent_id == target_agent_id
    {
        return Err(SubagentInceptionError::CycleDetected {
            target_agent_id: target_agent_id.to_owned(),
        });
    }
    if active_subagents_in_tree >= MAX_CONCURRENT_SUBAGENTS_PER_TREE {
        return Err(SubagentInceptionError::TreeCapacityExceeded {
            active_count: active_subagents_in_tree,
            max_capacity: MAX_CONCURRENT_SUBAGENTS_PER_TREE,
        });
    }
    if remaining_session_tokens == 0 {
        return Err(SubagentInceptionError::TokenBudgetExhausted {
            remaining_tokens: 0,
        });
    }
    let mut call_path = parent_lineage.call_path.clone();
    call_path.push(parent_lineage.current_agent_id.clone());
    Ok(SubagentLineage {
        root_session_id: parent_lineage.root_session_id.clone(),
        parent_agent_id: Some(parent_lineage.current_agent_id.clone()),
        current_agent_id: target_agent_id.to_owned(),
        depth: next_depth,
        call_path,
    })
}
