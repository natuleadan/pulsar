//! Plugin Registry and Lifecycle Service
//!
//! Handles registration, verification, permissions enforcement, and discovery of plugins.

pub mod hooks;
pub mod instincts;
pub mod scanner;

use crate::infra::error::ServerError;
use agents_protocol::{AgentCard, AgentCardEndpoints, PluginManifest, PluginToolDefinition};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct PluginRegistry {
    plugins: RwLock<HashMap<String, PluginManifest>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or updates a plugin manifest in the runtime.
    pub async fn register(&self, manifest: PluginManifest) -> Result<(), ServerError> {
        if manifest.id.trim().is_empty() {
            return Err(ServerError::Validation("plugin id cannot be empty".into()));
        }
        let mut lock = self.plugins.write().await;
        lock.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// Fetches a registered plugin by ID.
    pub async fn get(&self, id: &str) -> Option<PluginManifest> {
        let lock = self.plugins.read().await;
        lock.get(id).cloned()
    }

    /// Lists all installed plugins.
    pub async fn list(&self) -> Vec<PluginManifest> {
        let lock = self.plugins.read().await;
        lock.values().cloned().collect()
    }

    /// Toggles an individual tool within a plugin.
    pub async fn set_tool_status(
        &self,
        plugin_id: &str,
        tool_name: &str,
        enabled: bool,
    ) -> Result<(), ServerError> {
        let mut lock = self.plugins.write().await;
        let plugin = lock
            .get_mut(plugin_id)
            .ok_or_else(|| ServerError::NotFound(format!("plugin `{plugin_id}` not found")))?;
        if !plugin.set_tool_enabled(tool_name, enabled) {
            return Err(ServerError::NotFound(format!(
                "tool `{tool_name}` not found in plugin `{plugin_id}`"
            )));
        }
        Ok(())
    }

    /// Exports all active tools across all installed plugins.
    pub async fn collect_active_tools(&self) -> Vec<PluginToolDefinition> {
        let lock = self.plugins.read().await;
        let mut tools = Vec::new();
        for plugin in lock.values() {
            for tool in plugin.enabled_tools() {
                tools.push(tool.clone());
            }
        }
        tools
    }

    /// Generates public AgentCard for federated discovery (`/.well-known/agent-card.json`).
    pub async fn generate_agent_card(&self, public_base_url: &str) -> AgentCard {
        let lock = self.plugins.read().await;
        let plugin_ids: Vec<String> = lock.keys().cloned().collect();
        let mut capabilities = std::collections::BTreeSet::new();
        capabilities.insert("mcp:tools".to_owned());
        capabilities.insert("stream:sse".to_owned());
        capabilities.insert("agent:a2a".to_owned());

        AgentCard {
            id: "agents-sovereign-node".to_owned(),
            name: "Agents Sovereign Node".to_owned(),
            description: "Sovereign AI Agent Orchestrator with Boxer Sandboxes and MCP Gateway"
                .to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: "1.0.0".to_owned(),
            public_key: "sovereign-ed25519-public-identity".to_owned(),
            capabilities,
            endpoints: AgentCardEndpoints {
                api_base: format!("{public_base_url}/api/v1"),
                mcp_endpoint: format!("{public_base_url}/mcp"),
                sse_stream: format!("{public_base_url}/api/v1/sessions"),
                a2a_gateway: format!("{public_base_url}/api/v1/a2a"),
            },
            plugins: plugin_ids,
        }
    }
}
