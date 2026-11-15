use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use chrono::Utc;
use sqlx::{FromRow, PgPool};
use tokio::sync::RwLock;

use crate::infra::error::ServerError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, FromRow)]
pub struct StoredProvider {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub default_model: String,
    pub encrypted_api_key: Option<String>,
    pub key_fingerprint: Option<String>,
    pub is_default: bool,
    pub enabled: bool,
    pub updated_at: String,
}

pub trait ProviderRepository: Send + Sync {
    fn list(&self) -> BoxFuture<'_, Result<Vec<StoredProvider>, ServerError>>;
    fn get_default(&self) -> BoxFuture<'_, Result<Option<StoredProvider>, ServerError>>;
    fn upsert(&self, item: StoredProvider) -> BoxFuture<'_, Result<StoredProvider, ServerError>>;
    fn set_default(&self, id: &str) -> BoxFuture<'_, Result<bool, ServerError>>;
    fn delete(&self, id: &str) -> BoxFuture<'_, Result<bool, ServerError>>;
}

#[derive(Clone)]
pub struct ProviderStore(pub Arc<dyn ProviderRepository>);

impl Default for ProviderStore {
    fn default() -> Self {
        Self(Arc::new(InMemoryProviderRepository::default()))
    }
}

#[derive(Default)]
pub struct InMemoryProviderRepository {
    items: RwLock<HashMap<String, StoredProvider>>,
}

impl ProviderRepository for InMemoryProviderRepository {
    fn list(&self) -> BoxFuture<'_, Result<Vec<StoredProvider>, ServerError>> {
        Box::pin(async move {
            let mut out: Vec<_> = self.items.read().await.values().cloned().collect();
            out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.id.cmp(&b.id)));
            Ok(out)
        })
    }

    fn get_default(&self) -> BoxFuture<'_, Result<Option<StoredProvider>, ServerError>> {
        Box::pin(async move {
            let guard = self.items.read().await;
            Ok(guard
                .values()
                .find(|p| p.enabled && p.is_default)
                .or_else(|| guard.values().find(|p| p.enabled))
                .cloned())
        })
    }

    fn upsert(
        &self,
        mut item: StoredProvider,
    ) -> BoxFuture<'_, Result<StoredProvider, ServerError>> {
        Box::pin(async move {
            let mut guard = self.items.write().await;
            if item.is_default {
                for existing in guard.values_mut() {
                    existing.is_default = false;
                }
            }
            item.updated_at = Utc::now().to_rfc3339();
            guard.insert(item.id.clone(), item.clone());
            Ok(item)
        })
    }

    fn set_default(&self, id: &str) -> BoxFuture<'_, Result<bool, ServerError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let mut guard = self.items.write().await;
            if !guard.contains_key(&id) {
                return Ok(false);
            }
            for (k, v) in guard.iter_mut() {
                v.is_default = k == &id;
            }
            Ok(true)
        })
    }

    fn delete(&self, id: &str) -> BoxFuture<'_, Result<bool, ServerError>> {
        let id = id.to_owned();
        Box::pin(async move { Ok(self.items.write().await.remove(&id).is_some()) })
    }
}

pub struct PgProviderRepository {
    pool: PgPool,
}

impl PgProviderRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ProviderRepository for PgProviderRepository {
    fn list(&self) -> BoxFuture<'_, Result<Vec<StoredProvider>, ServerError>> {
        Box::pin(async move {
            sqlx::query_as::<_, StoredProvider>(
                "SELECT id, name, kind, base_url, default_model, encrypted_api_key, key_fingerprint, is_default, enabled, updated_at::text AS updated_at FROM inference_providers ORDER BY is_default DESC, id ASC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list providers failed: {e}")))
        })
    }

    fn get_default(&self) -> BoxFuture<'_, Result<Option<StoredProvider>, ServerError>> {
        Box::pin(async move {
            sqlx::query_as::<_, StoredProvider>(
                "SELECT id, name, kind, base_url, default_model, encrypted_api_key, key_fingerprint, is_default, enabled, updated_at::text AS updated_at FROM inference_providers WHERE enabled = TRUE ORDER BY is_default DESC, updated_at DESC LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("get default provider failed: {e}")))
        })
    }

    fn upsert(&self, item: StoredProvider) -> BoxFuture<'_, Result<StoredProvider, ServerError>> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| ServerError::Internal(format!("tx begin failed: {e}")))?;
            if item.is_default {
                sqlx::query("UPDATE inference_providers SET is_default = FALSE")
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| ServerError::Internal(format!("clear default failed: {e}")))?;
            }
            let row = sqlx::query_as::<_, StoredProvider>(
                r"
                INSERT INTO inference_providers (id, name, kind, base_url, default_model, encrypted_api_key, key_fingerprint, is_default, enabled, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    kind = EXCLUDED.kind,
                    base_url = EXCLUDED.base_url,
                    default_model = EXCLUDED.default_model,
                    encrypted_api_key = COALESCE(EXCLUDED.encrypted_api_key, inference_providers.encrypted_api_key),
                    key_fingerprint = COALESCE(EXCLUDED.key_fingerprint, inference_providers.key_fingerprint),
                    is_default = EXCLUDED.is_default,
                    enabled = EXCLUDED.enabled,
                    updated_at = NOW()
                RETURNING id, name, kind, base_url, default_model, encrypted_api_key, key_fingerprint, is_default, enabled, updated_at::text AS updated_at
                ",
            )
            .bind(&item.id)
            .bind(&item.name)
            .bind(&item.kind)
            .bind(&item.base_url)
            .bind(&item.default_model)
            .bind(&item.encrypted_api_key)
            .bind(&item.key_fingerprint)
            .bind(item.is_default)
            .bind(item.enabled)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ServerError::Internal(format!("upsert provider failed: {e}")))?;
            tx.commit()
                .await
                .map_err(|e| ServerError::Internal(format!("tx commit failed: {e}")))?;
            Ok(row)
        })
    }

    fn set_default(&self, id: &str) -> BoxFuture<'_, Result<bool, ServerError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| ServerError::Internal(format!("tx begin failed: {e}")))?;
            sqlx::query("UPDATE inference_providers SET is_default = FALSE")
                .execute(&mut *tx)
                .await
                .map_err(|e| ServerError::Internal(format!("clear default failed: {e}")))?;
            let res = sqlx::query(
                "UPDATE inference_providers SET is_default = TRUE, updated_at = NOW() WHERE id = $1",
            )
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ServerError::Internal(format!("set default failed: {e}")))?;
            tx.commit()
                .await
                .map_err(|e| ServerError::Internal(format!("tx commit failed: {e}")))?;
            Ok(res.rows_affected() > 0)
        })
    }

    fn delete(&self, id: &str) -> BoxFuture<'_, Result<bool, ServerError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let res = sqlx::query("DELETE FROM inference_providers WHERE id = $1")
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(|e| ServerError::Internal(format!("delete provider failed: {e}")))?;
            Ok(res.rows_affected() > 0)
        })
    }
}
