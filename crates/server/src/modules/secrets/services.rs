use std::sync::Arc;

use agents_protocol::ProxyCredentialSpec;
use ironic::prelude::*;

use crate::{
    infra::{config::ServerConfig, error::ServerError},
    modules::{
        providers::crypto::{decrypt_api_key, encrypt_api_key, key_fingerprint},
        secrets::{
            dto::{AddGroupMemberDto, CreateSecretGroupDto, UpsertSecretDto},
            entities::{SecretGroupEntity, SecretMetadataEntity},
            repositories::{SecretStore, StoredSecret, StoredSecretGroup},
        },
    },
};

const RESERVED_SYSTEM_NAMES: &[&str] = &[
    "AGENTS_JWT_SECRET",
    "AGENTS_CREDENTIAL_MASTER_KEY",
    "AGENTS_ADMIN_INITIAL_PASSWORD",
    "AGENTS_POSTGRES_PASSWORD",
    "DATABASE_URL",
    "VALKEY_URL",
    "VALKEY_PASSWORD",
    "NATS_NKEY_SEED",
    "S3_ACCESS_KEY",
    "S3_SECRET_KEY",
];

#[derive(Injectable)]
pub struct SecretService {
    store: Arc<SecretStore>,
    config: Arc<ServerConfig>,
}

impl SecretService {
    fn scoped_master_key(
        &self,
        owner_user_id: &str,
        scope_type: &str,
        scope_id: Option<&str>,
    ) -> String {
        let base = std::env::var("AGENTS_CREDENTIAL_MASTER_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.config.jwt_secret.clone());
        format!(
            "{base}:secret:{owner_user_id}:{scope_type}:{}",
            scope_id.unwrap_or("*")
        )
    }

    fn validate_scope(scope_type: &str, scope_id: Option<&str>) -> Result<(), ServerError> {
        match scope_type {
            "owner_shared" => Ok(()),
            "group" | "agent" | "worker" => {
                if scope_id.is_none_or(|s| s.trim().is_empty()) {
                    return Err(ServerError::Validation(format!(
                        "scopeId is required when scopeType is `{scope_type}`"
                    )));
                }
                Ok(())
            }
            _ => Err(ServerError::Validation(
                "scopeType must be one of: owner_shared, group, agent, worker".to_owned(),
            )),
        }
    }

    fn to_metadata(row: StoredSecret) -> SecretMetadataEntity {
        SecretMetadataEntity {
            id: row.id,
            name: row.name,
            scope_type: row.scope_type,
            scope_id: row.scope_id,
            target_host: row.target_host,
            header_name: row.header_name,
            has_value: true,
            value_fingerprint: row.value_fingerprint,
            updated_at: row.updated_at,
        }
    }

    /// Lists secret metadata only (names, scopes, target hosts, fingerprints — never values).
    pub async fn list_metadata(
        &self,
        owner_user_id: &str,
        scope_type: Option<&str>,
        scope_id: Option<&str>,
    ) -> Result<Vec<SecretMetadataEntity>, ServerError> {
        let rows = self.store.0.list_by_owner(owner_user_id).await?;
        Ok(rows
            .into_iter()
            .filter(|r| scope_type.is_none_or(|st| r.scope_type == st))
            .filter(|r| scope_id.is_none_or(|sid| r.scope_id.as_deref() == Some(sid)))
            .map(Self::to_metadata)
            .collect())
    }

    /// Encrypts and stores an isolated user/agent/worker/group secret.
    pub async fn upsert(
        &self,
        owner_user_id: &str,
        dto: &UpsertSecretDto,
    ) -> Result<SecretMetadataEntity, ServerError> {
        let normalized_name = dto.name.trim().to_ascii_uppercase();
        if RESERVED_SYSTEM_NAMES.contains(&normalized_name.as_str()) {
            return Err(ServerError::Forbidden(
                "system orchestrator secret names cannot be overridden in user secret vault"
                    .to_owned(),
            ));
        }
        let scope_type = dto.scope_type.trim().to_ascii_lowercase();
        let scope_id = dto
            .scope_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Self::validate_scope(&scope_type, scope_id.as_deref())?;

        let key = self.scoped_master_key(owner_user_id, &scope_type, scope_id.as_deref());
        let encrypted_value = encrypt_api_key(&key, &dto.value)?;
        let value_fingerprint = key_fingerprint(&dto.value);

        let stored = self
            .store
            .0
            .upsert(StoredSecret {
                id: dto.id.trim().to_owned(),
                owner_user_id: owner_user_id.to_owned(),
                name: dto.name.trim().to_owned(),
                scope_type,
                scope_id,
                target_host: dto.target_host.trim().to_ascii_lowercase(),
                header_name: dto
                    .header_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("authorization")
                    .to_ascii_lowercase(),
                header_prefix: dto
                    .header_prefix
                    .clone()
                    .unwrap_or_else(|| "Bearer ".to_owned()),
                encrypted_value,
                value_fingerprint,
                updated_at: String::new(),
            })
            .await?;
        Ok(Self::to_metadata(stored))
    }

    pub async fn delete(&self, owner_user_id: &str, id: &str) -> Result<(), ServerError> {
        if !self.store.0.delete(owner_user_id, id).await? {
            return Err(ServerError::NotFound(format!("secret `{id}` not found")));
        }
        Ok(())
    }

    pub async fn create_group(
        &self,
        owner_user_id: &str,
        dto: &CreateSecretGroupDto,
    ) -> Result<SecretGroupEntity, ServerError> {
        let g = self
            .store
            .0
            .create_group(StoredSecretGroup {
                id: dto.id.trim().to_owned(),
                owner_user_id: owner_user_id.to_owned(),
                name: dto.name.trim().to_owned(),
                description: dto.description.clone().unwrap_or_default(),
                created_at: String::new(),
            })
            .await?;
        Ok(SecretGroupEntity {
            id: g.id,
            name: g.name,
            description: g.description,
            created_at: g.created_at,
        })
    }

    pub async fn add_group_member(
        &self,
        owner_user_id: &str,
        group_id: &str,
        dto: &AddGroupMemberDto,
    ) -> Result<(), ServerError> {
        let kind = dto.member_kind.trim().to_ascii_lowercase();
        if !matches!(kind.as_str(), "agent" | "worker" | "coordinator") {
            return Err(ServerError::Validation(
                "memberKind must be `agent`, `worker`, or `coordinator`".to_owned(),
            ));
        }
        if !self
            .store
            .0
            .add_group_member(owner_user_id, group_id, &kind, dto.member_id.trim())
            .await?
        {
            return Err(ServerError::NotFound(format!(
                "secret group `{group_id}` not found"
            )));
        }
        Ok(())
    }

    /// Resolves ONLY the secrets authorized for `(owner_user_id, member_id)` for Boxer proxy injection.
    pub async fn resolve_proxy_credentials(
        &self,
        owner_user_id: &str,
        member_id: Option<&str>,
    ) -> Result<Vec<ProxyCredentialSpec>, ServerError> {
        let all = self.store.0.list_by_owner(owner_user_id).await?;
        let groups = match member_id {
            Some(mid) => self.store.0.list_member_groups(owner_user_id, mid).await?,
            None => Vec::new(),
        };

        let mut out = Vec::new();
        for row in all {
            let allowed = match row.scope_type.as_str() {
                "owner_shared" => true,
                "agent" | "worker" => {
                    member_id.is_some_and(|mid| row.scope_id.as_deref() == Some(mid))
                }
                "group" => row
                    .scope_id
                    .as_deref()
                    .is_some_and(|gid| groups.iter().any(|g| g == gid)),
                _ => false,
            };
            if !allowed {
                continue;
            }
            let key =
                self.scoped_master_key(owner_user_id, &row.scope_type, row.scope_id.as_deref());
            let plaintext = decrypt_api_key(&key, &row.encrypted_value)?;
            out.push(ProxyCredentialSpec {
                host: row.target_host,
                header_name: row.header_name,
                header_value: format!("{}{}", row.header_prefix, plaintext),
            });
        }
        Ok(out)
    }
}
