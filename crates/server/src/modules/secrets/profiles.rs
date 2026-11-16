//! Named Environment Profiles for Dynamic Secret Injection
//!
//! Enables hot-switching between environments (Dev, Staging, Prod) without server or sandbox restarts.

use crate::infra::error::ServerError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub secrets: HashMap<String, String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Default)]
pub struct ProfileManager {
    profiles: RwLock<HashMap<String, EnvProfile>>,
    session_active_profile: RwLock<HashMap<String, String>>,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores or updates an environment profile.
    pub async fn save_profile(&self, profile: EnvProfile) -> Result<(), ServerError> {
        if profile.id.trim().is_empty() {
            return Err(ServerError::Validation("profile id cannot be empty".into()));
        }
        let mut lock = self.profiles.write().await;
        lock.insert(profile.id.clone(), profile);
        Ok(())
    }

    /// Retrieves an environment profile by ID.
    pub async fn get_profile(&self, id: &str) -> Option<EnvProfile> {
        let lock = self.profiles.read().await;
        lock.get(id).cloned()
    }

    /// Lists all available profiles.
    pub async fn list_profiles(&self) -> Vec<EnvProfile> {
        let lock = self.profiles.read().await;
        lock.values().cloned().collect()
    }

    /// Binds an environment profile to a session.
    pub async fn bind_to_session(
        &self,
        session_id: &str,
        profile_id: &str,
    ) -> Result<(), ServerError> {
        let lock = self.profiles.read().await;
        if !lock.contains_key(profile_id) {
            return Err(ServerError::NotFound(format!(
                "environment profile `{profile_id}` not found"
            )));
        }
        drop(lock);

        let mut session_lock = self.session_active_profile.write().await;
        session_lock.insert(session_id.to_owned(), profile_id.to_owned());
        Ok(())
    }

    /// Resolves the active environment variables and secrets for a session.
    pub async fn resolve_for_session(&self, session_id: &str) -> Option<EnvProfile> {
        let session_lock = self.session_active_profile.read().await;
        let profile_id = session_lock.get(session_id)?;
        let lock = self.profiles.read().await;
        lock.get(profile_id).cloned()
    }
}
