//! Provider Driver vs Instance Management
//!
//! Enables multiple accounts, OAuth subscriptions, and workgroup-specific provider binding.

use crate::infra::error::ServerError;
use pulsar_protocol::ProviderProtocol;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthCredential {
    ApiKey(String),
    OAuthToken {
        access_token: String,
        refresh_token: String,
        expires_at_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstance {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub credential: AuthCredential,
    pub workgroup_id: Option<String>,
    pub max_monthly_budget_usd: Option<f64>,
    pub is_active: bool,
}

#[derive(Default)]
pub struct ProviderInstanceManager {
    instances: RwLock<HashMap<String, ProviderInstance>>,
}

impl ProviderInstanceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register_instance(&self, instance: ProviderInstance) -> Result<(), ServerError> {
        if instance.id.trim().is_empty() {
            return Err(ServerError::Validation(
                "instance id cannot be empty".into(),
            ));
        }
        let mut lock = self.instances.write().await;
        lock.insert(instance.id.clone(), instance);
        Ok(())
    }

    pub async fn get_instance(&self, id: &str) -> Option<ProviderInstance> {
        let lock = self.instances.read().await;
        lock.get(id).cloned()
    }

    pub async fn resolve_for_workgroup(
        &self,
        workgroup_id: &str,
        protocol: &ProviderProtocol,
    ) -> Option<ProviderInstance> {
        let lock = self.instances.read().await;
        for instance in lock.values() {
            if instance.is_active
                && &instance.protocol == protocol
                && instance.workgroup_id.as_deref() == Some(workgroup_id)
            {
                return Some(instance.clone());
            }
        }
        // Fallback to active global instance
        for instance in lock.values() {
            if instance.is_active
                && &instance.protocol == protocol
                && instance.workgroup_id.is_none()
            {
                return Some(instance.clone());
            }
        }
        None
    }
}
