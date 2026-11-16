//! Plugin Manifest and Capability Specification
//!
//! Provides portable manifest types, permission masks, and A2A Agent Card definitions.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Permission mask defining what an extension or plugin is permitted to access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityMask {
    pub allow_network: bool,
    pub allowed_hosts: Vec<String>,
    pub allow_filesystem: bool,
    pub allowed_paths: Vec<String>,
    pub allow_env_vars: Vec<String>,
    pub max_execution_seconds: u32,
    pub max_memory_mb: u32,
}

impl Default for PluginCapabilityMask {
    fn default() -> Self {
        Self {
            allow_network: false,
            allowed_hosts: Vec::new(),
            allow_filesystem: false,
            allowed_paths: Vec::new(),
            allow_env_vars: Vec::new(),
            max_execution_seconds: 30,
            max_memory_mb: 256,
        }
    }
}

impl PluginCapabilityMask {
    #[must_use]
    pub fn is_host_allowed(&self, host: &str) -> bool {
        if !self.allow_network {
            return false;
        }
        let lower = host.to_ascii_lowercase();
        self.allowed_hosts.iter().any(|allowed| {
            let allowed_lower = allowed.to_ascii_lowercase();
            if allowed_lower == "*" || allowed_lower == lower {
                return true;
            }
            if let Some(suffix) = allowed_lower.strip_prefix("*.") {
                return lower == suffix || lower.ends_with(&format!(".{suffix}"));
            }
            lower.ends_with(&format!(".{allowed_lower}"))
        })
    }

    #[must_use]
    pub fn is_path_allowed(&self, path: &str) -> bool {
        if !self.allow_filesystem {
            return false;
        }
        self.allowed_paths
            .iter()
            .any(|allowed| path.starts_with(allowed))
    }
}

/// Dynamic MCP tool metadata exposed by a plugin or openjson specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub method: String,
}

const fn default_true() -> bool {
    true
}

/// Self-contained plugin manifest bundling metadata, tools, and permissions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    #[serde(default)]
    pub permissions: PluginCapabilityMask,
    #[serde(default)]
    pub tools: Vec<PluginToolDefinition>,
    #[serde(default)]
    pub settings_schema: Option<Value>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl PluginManifest {
    #[must_use]
    pub fn enabled_tools(&self) -> Vec<&PluginToolDefinition> {
        self.tools.iter().filter(|t| t.enabled).collect()
    }

    #[must_use]
    pub fn get_tool(&self, name: &str) -> Option<&PluginToolDefinition> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn set_tool_enabled(&mut self, name: &str, enabled: bool) -> bool {
        if let Some(tool) = self.tools.iter_mut().find(|t| t.name == name) {
            tool.enabled = enabled;
            true
        } else {
            false
        }
    }
}

/// A2A Agent Card for federated discovery (`/.well-known/agent-card.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub protocol_version: String,
    pub public_key: String,
    pub capabilities: BTreeSet<String>,
    pub endpoints: AgentCardEndpoints,
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCardEndpoints {
    pub api_base: String,
    pub mcp_endpoint: String,
    pub sse_stream: String,
    pub a2a_gateway: String,
}
