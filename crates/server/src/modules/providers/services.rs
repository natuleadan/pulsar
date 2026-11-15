use std::sync::Arc;

use agents_protocol::InferenceEndpointSpec;
use ironic::prelude::*;

use crate::{
    infra::{config::ServerConfig, error::ServerError},
    modules::providers::{
        crypto::{decrypt_api_key, encrypt_api_key, key_fingerprint},
        dto::UpsertProviderDto,
        entities::InferenceProviderEntity,
        repositories::{ProviderStore, StoredProvider},
    },
};

#[derive(Injectable)]
pub struct ProviderService {
    store: Arc<ProviderStore>,
    config: Arc<ServerConfig>,
}

impl ProviderService {
    fn master_key(&self) -> String {
        std::env::var("AGENTS_CREDENTIAL_MASTER_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.config.jwt_secret.clone())
    }

    fn to_entity(row: StoredProvider) -> InferenceProviderEntity {
        InferenceProviderEntity {
            id: row.id,
            name: row.name,
            kind: row.kind,
            base_url: row.base_url,
            default_model: row.default_model,
            has_api_key: row.encrypted_api_key.is_some(),
            key_fingerprint: row.key_fingerprint,
            is_default: row.is_default,
            enabled: row.enabled,
            updated_at: row.updated_at,
        }
    }

    /// Lists all configured inference providers with redacted key metadata.
    pub async fn list(&self) -> Result<Vec<InferenceProviderEntity>, ServerError> {
        let rows = self.store.0.list().await?;
        Ok(rows.into_iter().map(Self::to_entity).collect())
    }

    /// Creates or updates a provider, encrypting any supplied API key with Argon2id + AES-256-GCM.
    pub async fn upsert(
        &self,
        dto: &UpsertProviderDto,
    ) -> Result<InferenceProviderEntity, ServerError> {
        let master = self.master_key();
        let (encrypted_api_key, fingerprint) = match dto.api_key.as_deref().map(str::trim) {
            Some(raw) if !raw.is_empty() => (
                Some(encrypt_api_key(&master, raw)?),
                Some(key_fingerprint(raw)),
            ),
            _ => (None, None),
        };
        let row = self
            .store
            .0
            .upsert(StoredProvider {
                id: dto.id.trim().to_owned(),
                name: dto.name.trim().to_owned(),
                kind: dto.kind.trim().to_ascii_lowercase(),
                base_url: dto.base_url.trim().trim_end_matches('/').to_owned(),
                default_model: dto.default_model.trim().to_owned(),
                encrypted_api_key,
                key_fingerprint: fingerprint,
                is_default: dto.is_default,
                enabled: dto.enabled.unwrap_or(true),
                updated_at: String::new(),
            })
            .await?;
        Ok(Self::to_entity(row))
    }

    pub async fn set_default(&self, id: &str) -> Result<(), ServerError> {
        if !self.store.0.set_default(id).await? {
            return Err(ServerError::NotFound(format!("provider `{id}` not found")));
        }
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), ServerError> {
        if !self.store.0.delete(id).await? {
            return Err(ServerError::NotFound(format!("provider `{id}` not found")));
        }
        Ok(())
    }

    /// Seeds the initial default inference provider if the table is empty.
    /// Defaults to Ollama (`http://127.0.0.1:11434/v1`) when no env override is set.
    pub async fn seed_default_if_empty(&self) -> Result<(), ServerError> {
        if !self.store.0.list().await?.is_empty() {
            return Ok(());
        }
        let kind = std::env::var("AGENTS_LLM_PROVIDER")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "ollama".to_owned());
        let base_url = std::env::var("AGENTS_LLM_ENDPOINT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:11434/v1".to_owned());
        let default_model = std::env::var("AGENTS_LLM_MODEL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "llama3.2".to_owned());
        let api_key = std::env::var("AGENTS_LLM_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.config.deepinfra_token.clone())
            .or_else(|| self.config.deepseek_api_key.clone());

        let dto = UpsertProviderDto {
            id: "default".to_owned(),
            name: format!("Default ({kind})"),
            kind,
            base_url,
            default_model,
            api_key,
            is_default: true,
            enabled: Some(true),
        };
        let _ = self.upsert(&dto).await?;
        Ok(())
    }

    /// Resolves the active default inference endpoint + decrypted API key in memory.
    pub async fn resolve_active_endpoint(
        &self,
    ) -> Result<Option<InferenceEndpointSpec>, ServerError> {
        let Some(row) = self.store.0.get_default().await? else {
            return Ok(None);
        };
        let master = self.master_key();
        let api_key = match row.encrypted_api_key.as_deref() {
            Some(enc) => Some(decrypt_api_key(&master, enc)?),
            None => None,
        };
        Ok(Some(InferenceEndpointSpec {
            kind: row.kind,
            base_url: row.base_url,
            model: row.default_model,
            api_key,
        }))
    }
}
