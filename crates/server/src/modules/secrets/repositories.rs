use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use chrono::Utc;
use sqlx::{FromRow, PgPool};
use tokio::sync::RwLock;

use crate::infra::error::ServerError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, FromRow)]
pub struct StoredSecret {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub target_host: String,
    pub header_name: String,
    pub header_prefix: String,
    pub encrypted_value: String,
    pub value_fingerprint: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredSecretGroup {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
}

pub trait SecretRepository: Send + Sync {
    fn list_by_owner(&self, owner: &str) -> BoxFuture<'_, Result<Vec<StoredSecret>, ServerError>>;
    fn upsert(&self, item: StoredSecret) -> BoxFuture<'_, Result<StoredSecret, ServerError>>;
    fn delete(&self, owner: &str, id: &str) -> BoxFuture<'_, Result<bool, ServerError>>;
    fn create_group(
        &self,
        group: StoredSecretGroup,
    ) -> BoxFuture<'_, Result<StoredSecretGroup, ServerError>>;
    fn add_group_member(
        &self,
        owner: &str,
        group_id: &str,
        member_kind: &str,
        member_id: &str,
    ) -> BoxFuture<'_, Result<bool, ServerError>>;
    fn list_member_groups(
        &self,
        owner: &str,
        member_id: &str,
    ) -> BoxFuture<'_, Result<Vec<String>, ServerError>>;
}

#[derive(Clone)]
pub struct SecretStore(pub Arc<dyn SecretRepository>);

impl Default for SecretStore {
    fn default() -> Self {
        Self(Arc::new(InMemorySecretRepository::default()))
    }
}

#[derive(Default)]
pub struct InMemorySecretRepository {
    secrets: RwLock<HashMap<String, StoredSecret>>,
    groups: RwLock<HashMap<String, StoredSecretGroup>>,
    members: RwLock<HashSet<(String, String, String)>>,
}

impl SecretRepository for InMemorySecretRepository {
    fn list_by_owner(&self, owner: &str) -> BoxFuture<'_, Result<Vec<StoredSecret>, ServerError>> {
        let owner = owner.to_owned();
        Box::pin(async move {
            let mut out: Vec<_> = self
                .secrets
                .read()
                .await
                .values()
                .filter(|s| s.owner_user_id == owner)
                .cloned()
                .collect();
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        })
    }

    fn upsert(&self, mut item: StoredSecret) -> BoxFuture<'_, Result<StoredSecret, ServerError>> {
        Box::pin(async move {
            item.updated_at = Utc::now().to_rfc3339();
            self.secrets
                .write()
                .await
                .insert(item.id.clone(), item.clone());
            Ok(item)
        })
    }

    fn delete(&self, owner: &str, id: &str) -> BoxFuture<'_, Result<bool, ServerError>> {
        let owner = owner.to_owned();
        let id = id.to_owned();
        Box::pin(async move {
            let mut guard = self.secrets.write().await;
            if guard.get(&id).is_some_and(|s| s.owner_user_id == owner) {
                return Ok(guard.remove(&id).is_some());
            }
            Ok(false)
        })
    }

    fn create_group(
        &self,
        mut group: StoredSecretGroup,
    ) -> BoxFuture<'_, Result<StoredSecretGroup, ServerError>> {
        Box::pin(async move {
            group.created_at = Utc::now().to_rfc3339();
            self.groups
                .write()
                .await
                .insert(group.id.clone(), group.clone());
            Ok(group)
        })
    }

    fn add_group_member(
        &self,
        owner: &str,
        group_id: &str,
        member_kind: &str,
        member_id: &str,
    ) -> BoxFuture<'_, Result<bool, ServerError>> {
        let owner = owner.to_owned();
        let group_id = group_id.to_owned();
        let member_kind = member_kind.to_owned();
        let member_id = member_id.to_owned();
        Box::pin(async move {
            let groups = self.groups.read().await;
            if !groups
                .get(&group_id)
                .is_some_and(|g| g.owner_user_id == owner)
            {
                return Ok(false);
            }
            drop(groups);
            self.members
                .write()
                .await
                .insert((group_id, member_kind, member_id));
            Ok(true)
        })
    }

    fn list_member_groups(
        &self,
        owner: &str,
        member_id: &str,
    ) -> BoxFuture<'_, Result<Vec<String>, ServerError>> {
        let owner = owner.to_owned();
        let member_id = member_id.to_owned();
        Box::pin(async move {
            let groups = self.groups.read().await;
            let members = self.members.read().await;
            Ok(members
                .iter()
                .filter(|(gid, _, mid)| {
                    mid == &member_id && groups.get(gid).is_some_and(|g| g.owner_user_id == owner)
                })
                .map(|(gid, _, _)| gid.clone())
                .collect())
        })
    }
}

pub struct PgSecretRepository {
    pool: PgPool,
}

impl PgSecretRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SecretRepository for PgSecretRepository {
    fn list_by_owner(&self, owner: &str) -> BoxFuture<'_, Result<Vec<StoredSecret>, ServerError>> {
        let owner = owner.to_owned();
        Box::pin(async move {
            sqlx::query_as::<_, StoredSecret>(
                "SELECT id, owner_user_id, name, scope_type, scope_id, target_host, header_name, header_prefix, encrypted_value, value_fingerprint, updated_at::text AS updated_at FROM secrets WHERE owner_user_id = $1 ORDER BY name ASC",
            )
            .bind(&owner)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list secrets failed: {e}")))
        })
    }

    fn upsert(&self, item: StoredSecret) -> BoxFuture<'_, Result<StoredSecret, ServerError>> {
        Box::pin(async move {
            sqlx::query_as::<_, StoredSecret>(
                r"
                INSERT INTO secrets (id, owner_user_id, name, scope_type, scope_id, target_host, header_name, header_prefix, encrypted_value, value_fingerprint, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    scope_type = EXCLUDED.scope_type,
                    scope_id = EXCLUDED.scope_id,
                    target_host = EXCLUDED.target_host,
                    header_name = EXCLUDED.header_name,
                    header_prefix = EXCLUDED.header_prefix,
                    encrypted_value = EXCLUDED.encrypted_value,
                    value_fingerprint = EXCLUDED.value_fingerprint,
                    updated_at = NOW()
                WHERE secrets.owner_user_id = EXCLUDED.owner_user_id
                RETURNING id, owner_user_id, name, scope_type, scope_id, target_host, header_name, header_prefix, encrypted_value, value_fingerprint, updated_at::text AS updated_at
                ",
            )
            .bind(&item.id)
            .bind(&item.owner_user_id)
            .bind(&item.name)
            .bind(&item.scope_type)
            .bind(&item.scope_id)
            .bind(&item.target_host)
            .bind(&item.header_name)
            .bind(&item.header_prefix)
            .bind(&item.encrypted_value)
            .bind(&item.value_fingerprint)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("upsert secret failed: {e}")))
        })
    }

    fn delete(&self, owner: &str, id: &str) -> BoxFuture<'_, Result<bool, ServerError>> {
        let owner = owner.to_owned();
        let id = id.to_owned();
        Box::pin(async move {
            let res = sqlx::query("DELETE FROM secrets WHERE id = $1 AND owner_user_id = $2")
                .bind(&id)
                .bind(&owner)
                .execute(&self.pool)
                .await
                .map_err(|e| ServerError::Internal(format!("delete secret failed: {e}")))?;
            Ok(res.rows_affected() > 0)
        })
    }

    fn create_group(
        &self,
        group: StoredSecretGroup,
    ) -> BoxFuture<'_, Result<StoredSecretGroup, ServerError>> {
        Box::pin(async move {
            sqlx::query_as::<_, StoredSecretGroup>(
                r"
                INSERT INTO secret_groups (id, owner_user_id, name, description)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description
                RETURNING id, owner_user_id, name, description, created_at::text AS created_at
                ",
            )
            .bind(&group.id)
            .bind(&group.owner_user_id)
            .bind(&group.name)
            .bind(&group.description)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("create secret group failed: {e}")))
        })
    }

    fn add_group_member(
        &self,
        owner: &str,
        group_id: &str,
        member_kind: &str,
        member_id: &str,
    ) -> BoxFuture<'_, Result<bool, ServerError>> {
        let owner = owner.to_owned();
        let group_id = group_id.to_owned();
        let member_kind = member_kind.to_owned();
        let member_id = member_id.to_owned();
        Box::pin(async move {
            let owned: Option<String> = sqlx::query_scalar(
                "SELECT id FROM secret_groups WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(&group_id)
            .bind(&owner)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("check secret group failed: {e}")))?;
            if owned.is_none() {
                return Ok(false);
            }
            sqlx::query(
                "INSERT INTO secret_group_members (group_id, member_kind, member_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(&group_id)
            .bind(&member_kind)
            .bind(&member_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("add group member failed: {e}")))?;
            Ok(true)
        })
    }

    fn list_member_groups(
        &self,
        owner: &str,
        member_id: &str,
    ) -> BoxFuture<'_, Result<Vec<String>, ServerError>> {
        let owner = owner.to_owned();
        let member_id = member_id.to_owned();
        Box::pin(async move {
            sqlx::query_scalar::<_, String>(
                "SELECT m.group_id FROM secret_group_members m JOIN secret_groups g ON g.id = m.group_id WHERE g.owner_user_id = $1 AND m.member_id = $2",
            )
            .bind(&owner)
            .bind(&member_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list member groups failed: {e}")))
        })
    }
}
