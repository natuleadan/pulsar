use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use agents_protocol::{
    DiscoveredModel, InferenceEndpointSpec, ProviderProtocol, classify_provider_error,
    parse_models_catalog_json,
};
use ironic::prelude::*;
use serde_json::Value;

use crate::{
    infra::{cache::AppCache, config::ServerConfig, error::ServerError},
    modules::providers::{
        crypto::{decrypt_api_key, encrypt_api_key, key_fingerprint},
        dto::UpsertProviderDto,
        entities::{
            InferenceProviderEntity, ProviderHandshakeResponse, ProviderModelsCatalogResponse,
        },
        repositories::{ProviderStore, StoredProvider},
    },
};

const MODELS_CACHE_TTL_SECS: u64 = 3600;

#[derive(Injectable)]
pub struct ProviderService {
    store: Arc<ProviderStore>,
    cache: Arc<AppCache>,
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
        let protocol = ProviderProtocol::from_kind(&row.kind);
        InferenceProviderEntity {
            id: row.id,
            name: row.name,
            kind: row.kind,
            protocol,
            base_url: row.base_url,
            default_model: row.default_model,
            fallback_provider_id: row.fallback_provider_id,
            has_api_key: row.encrypted_api_key.is_some(),
            key_fingerprint: row.key_fingerprint,
            is_default: row.is_default,
            enabled: row.enabled,
            updated_at: row.updated_at,
        }
    }

    pub async fn list(&self) -> Result<Vec<InferenceProviderEntity>, ServerError> {
        let rows = self.store.0.list().await?;
        Ok(rows.into_iter().map(Self::to_entity).collect())
    }

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
        let fallback_provider_id = dto
            .fallback_provider_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        let id = dto.id.trim().to_owned();
        let row = self
            .store
            .0
            .upsert(StoredProvider {
                id: id.clone(),
                name: dto.name.trim().to_owned(),
                kind: dto.kind.trim().to_ascii_lowercase(),
                base_url: dto.base_url.trim().trim_end_matches('/').to_owned(),
                default_model: dto.default_model.trim().to_owned(),
                fallback_provider_id,
                encrypted_api_key,
                key_fingerprint: fingerprint,
                is_default: dto.is_default,
                enabled: dto.enabled.unwrap_or(true),
                updated_at: String::new(),
            })
            .await?;
        let _ = self.cache.remove(&format!("provider_models:{id}")).await;
        Ok(Self::to_entity(row))
    }

    pub async fn set_default(&self, id: &str) -> Result<(), ServerError> {
        if !self.store.0.set_default(id).await? {
            return Err(ServerError::NotFound(format!("provider `{id}` not found")));
        }
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), ServerError> {
        let all = self.store.0.list().await?;
        let active_count = all.iter().filter(|p| p.enabled).count();
        if active_count <= 1 && all.iter().any(|p| p.id == id && p.enabled) {
            return Err(ServerError::Conflict(
                "cannot delete the last active inference provider".to_owned(),
            ));
        }
        if !self.store.0.delete(id).await? {
            return Err(ServerError::NotFound(format!("provider `{id}` not found")));
        }
        let _ = self.cache.remove(&format!("provider_models:{id}")).await;
        Ok(())
    }

    pub async fn discover_models(
        &self,
        id: &str,
    ) -> Result<ProviderModelsCatalogResponse, ServerError> {
        let Some(row) = self.store.0.get_by_id(id).await? else {
            return Err(ServerError::NotFound(format!("provider `{id}` not found")));
        };
        let protocol = ProviderProtocol::from_kind(&row.kind);
        let cache_key = format!("provider_models:{id}");
        if let Some(cached_models) = self
            .cache
            .get_json::<Vec<DiscoveredModel>>(&cache_key)
            .await
        {
            return Ok(ProviderModelsCatalogResponse {
                provider_id: row.id,
                protocol,
                cached: true,
                allow_freeform_override: true,
                models: cached_models,
            });
        }

        let master = self.master_key();
        let api_key = match row.encrypted_api_key.as_deref() {
            Some(enc) => decrypt_api_key(&master, enc).ok(),
            None => None,
        };
        let models_url = protocol.models_url(&row.base_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| ServerError::Internal(format!("http client build failed: {e}")))?;

        let mut req = client.get(&models_url);
        if let Some(secret) = api_key.as_deref().filter(|s| !s.is_empty()) {
            req = match protocol {
                ProviderProtocol::AnthropicMessages => req
                    .header("x-api-key", secret)
                    .header("anthropic-version", "2023-06-01"),
                _ => req.bearer_auth(secret),
            };
        }

        let mut discovered = match req.send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                Ok(json_val) => parse_models_catalog_json(&json_val, &row.kind),
                Err(_) => Vec::new(),
            },
            _ => Vec::new(),
        };

        if discovered.is_empty() && !row.default_model.trim().is_empty() {
            discovered.push(DiscoveredModel {
                id: row.default_model.clone(),
                owned_by: row.kind.clone(),
                context_window: None,
            });
        }

        self.cache
            .set_json(
                &cache_key,
                &discovered,
                Duration::from_secs(MODELS_CACHE_TTL_SECS),
            )
            .await;

        Ok(ProviderModelsCatalogResponse {
            provider_id: row.id,
            protocol,
            cached: false,
            allow_freeform_override: true,
            models: discovered,
        })
    }

    pub async fn test_handshake(&self, id: &str) -> Result<ProviderHandshakeResponse, ServerError> {
        let Some(row) = self.store.0.get_by_id(id).await? else {
            return Err(ServerError::NotFound(format!("provider `{id}` not found")));
        };
        let protocol = ProviderProtocol::from_kind(&row.kind);
        let master = self.master_key();
        let api_key = match row.encrypted_api_key.as_deref() {
            Some(enc) => decrypt_api_key(&master, enc).ok(),
            None => None,
        };
        let models_url = protocol.models_url(&row.base_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| ServerError::Internal(format!("http client build failed: {e}")))?;

        let mut req = client.get(&models_url);
        if let Some(secret) = api_key.as_deref().filter(|s| !s.is_empty()) {
            req = match protocol {
                ProviderProtocol::AnthropicMessages => req
                    .header("x-api-key", secret)
                    .header("anthropic-version", "2023-06-01"),
                _ => req.bearer_auth(secret),
            };
        }

        let start = Instant::now();
        match req.send().await {
            Ok(resp) => {
                let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(3000);
                let status = resp.status();
                if status.is_success() {
                    Ok(ProviderHandshakeResponse {
                        provider_id: row.id,
                        reachable: true,
                        latency_ms,
                        error_classification: None,
                    })
                } else {
                    let code = status.as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    let classified = classify_provider_error(Some(code), &body, 0, 3);
                    Ok(ProviderHandshakeResponse {
                        provider_id: row.id,
                        reachable: false,
                        latency_ms,
                        error_classification: Some(classified),
                    })
                }
            }
            Err(err) => {
                let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(3000);
                let classified = classify_provider_error(None, &err.to_string(), 0, 3);
                Ok(ProviderHandshakeResponse {
                    provider_id: row.id,
                    reachable: false,
                    latency_ms,
                    error_classification: Some(classified),
                })
            }
        }
    }

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
            fallback_provider_id: None,
            api_key,
            is_default: true,
            enabled: Some(true),
        };
        let _ = self.upsert(&dto).await?;
        Ok(())
    }

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
        let (fallback_base_url, fallback_model) = match row.fallback_provider_id.as_deref() {
            Some(fb_id) => match self.store.0.get_by_id(fb_id).await? {
                Some(fb_row) if fb_row.enabled => {
                    (Some(fb_row.base_url), Some(fb_row.default_model))
                }
                _ => (None, None),
            },
            None => (None, None),
        };
        Ok(Some(InferenceEndpointSpec {
            kind: row.kind,
            base_url: row.base_url,
            model: row.default_model,
            api_key,
            fallback_provider_id: row.fallback_provider_id,
            fallback_base_url,
            fallback_model,
        }))
    }
}
