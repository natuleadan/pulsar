//! Artifact service managing S3/RustFS uploads, Valkey metadata index, and temporary presigned URLs.

use std::sync::Arc;

use base64::Engine;
use ironic::prelude::*;
use subtle::ConstantTimeEq;

use crate::{
    infra::{adapters::BlobStoreClient, config::ServerConfig, error::ServerError, state::now_ms},
    modules::{
        artifacts::dto::{
            ArtifactListResponse, ArtifactRecord, PresignedArtifactResponse, UploadArtifactDto,
        },
        auth::{entities::AuthenticatedActor, services::StateStore},
        tasks::services::TaskService,
    },
};

const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Injectable)]
pub struct ArtifactService {
    blob: Arc<BlobStoreClient>,
    state: Arc<StateStore>,
    tasks: Arc<TaskService>,
    config: Arc<ServerConfig>,
}

impl ArtifactService {
    fn validate_filename(raw: &str) -> Result<String, ServerError> {
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.len() > 160
            || trimmed.contains('/')
            || trimmed.contains('\\')
            || trimmed.contains("..")
            || trimmed.starts_with('.')
        {
            return Err(ServerError::Validation("invalid artifact filename".into()));
        }
        Ok(trimmed.to_owned())
    }

    fn effective_ttl(&self, requested: Option<u64>) -> u64 {
        requested
            .unwrap_or(self.config.s3_presign_ttl_secs)
            .clamp(10, 86_400)
    }

    fn index_key(task_id: &str) -> String {
        format!("artifacts:task:{task_id}")
    }

    async fn load_index(&self, task_id: &str) -> Result<Vec<ArtifactRecord>, ServerError> {
        let Some(raw) = self.state.0.load_draft(&Self::index_key(task_id)).await? else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    async fn save_index(&self, task_id: &str, items: &[ArtifactRecord]) -> Result<(), ServerError> {
        let json = serde_json::to_string(items)
            .map_err(|e| ServerError::Internal(format!("serialize artifact index: {e}")))?;
        self.state
            .0
            .save_draft(&Self::index_key(task_id), &json)
            .await
    }

    async fn store_for_task(
        &self,
        task_id: &str,
        session_id: &str,
        dto: &UploadArtifactDto,
    ) -> Result<ArtifactRecord, ServerError> {
        let filename = Self::validate_filename(&dto.filename)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(dto.data_base64.trim())
            .map_err(|_| ServerError::Validation("invalid base64 artifact payload".into()))?;
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(ServerError::Validation(
                "artifact exceeds 16 MiB upload limit".into(),
            ));
        }
        let content_type = dto
            .content_type
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        let key = format!("tasks/{session_id}/{task_id}/{filename}");
        let size_bytes = bytes.len() as u64;
        let _ = self.blob.0.put_blob(&key, bytes, &content_type).await?;
        let ttl = self.effective_ttl(dto.ttl_secs);
        let presigned_url = self.blob.0.presign_url(&key, ttl).await?;

        let record = ArtifactRecord {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_owned(),
            session_id: session_id.to_owned(),
            key,
            filename,
            content_type,
            size_bytes,
            presigned_url,
            expires_in_secs: ttl,
            created_at_ms: now_ms(),
        };

        let mut index = self.load_index(task_id).await?;
        index.retain(|item| item.filename != record.filename);
        index.push(record.clone());
        self.save_index(task_id, &index).await?;
        Ok(record)
    }

    /// Uploads an artifact on behalf of an authenticated user owning `task_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`], [`ServerError::NotFound`], or [`ServerError::Validation`].
    pub async fn upload_as_actor(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        dto: &UploadArtifactDto,
    ) -> Result<ArtifactRecord, ServerError> {
        let task = self.tasks.get_owned(actor, task_id).await?;
        self.store_for_task(&task.id, &task.session_id, dto).await
    }

    /// Uploads raw bytes (e.g., from `multipart/form-data`) on behalf of an authenticated user.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`], [`ServerError::NotFound`], or [`ServerError::Validation`].
    pub async fn upload_raw_as_actor(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        raw_filename: &str,
        content_type: Option<String>,
        bytes: Vec<u8>,
        ttl_secs: Option<u64>,
    ) -> Result<ArtifactRecord, ServerError> {
        let task = self.tasks.get_owned(actor, task_id).await?;
        let filename = Self::validate_filename(raw_filename)?;
        if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(ServerError::Validation(format!(
                "artifact size must be between 1 byte and {} MiB",
                MAX_ARTIFACT_BYTES / (1024 * 1024)
            )));
        }
        let ct = content_type
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        let key = format!("tasks/{}/{}/{filename}", task.session_id, task.id);
        let size_bytes = bytes.len() as u64;
        let _ = self.blob.0.put_blob(&key, bytes, &ct).await?;
        let ttl = self.effective_ttl(ttl_secs);
        let presigned_url = self.blob.0.presign_url(&key, ttl).await?;
        let record = ArtifactRecord {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task.id.clone(),
            session_id: task.session_id.clone(),
            key,
            filename,
            content_type: ct,
            size_bytes,
            presigned_url,
            expires_in_secs: ttl,
            created_at_ms: now_ms(),
        };
        let mut index = self.load_index(&task.id).await?;
        index.retain(|item| item.filename != record.filename);
        index.push(record.clone());
        self.save_index(&task.id, &index).await?;
        Ok(record)
    }

    /// Uploads an artifact on behalf of a Boxer worker authenticated via cluster secret.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`], [`ServerError::NotFound`], or [`ServerError::Validation`].
    pub async fn upload_as_worker(
        &self,
        worker_token: &str,
        task_id: &str,
        dto: &UploadArtifactDto,
    ) -> Result<ArtifactRecord, ServerError> {
        let expected = self.config.cluster_secret.as_bytes();
        let provided = worker_token.as_bytes();
        let matches_cluster =
            expected.len() == provided.len() && expected.ct_eq(provided).unwrap_u8() == 1;
        let matches_worker_env = std::env::var("BOXER_WORKER_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .is_some_and(|tok| {
                let b = tok.as_bytes();
                b.len() == provided.len() && b.ct_eq(provided).unwrap_u8() == 1
            });
        if !matches_cluster && !matches_worker_env {
            return Err(ServerError::Unauthorized(
                "invalid boxer worker token".into(),
            ));
        }
        let task = self.tasks.get_unscoped(task_id).await?;
        self.store_for_task(&task.id, &task.session_id, dto).await
    }

    /// Lists artifacts for a task with freshly signed temporary URLs.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn list_for_task(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        ttl_secs: Option<u64>,
    ) -> Result<ArtifactListResponse, ServerError> {
        let task = self.tasks.get_owned(actor, task_id).await?;
        let ttl = self.effective_ttl(ttl_secs);
        let mut items = self.load_index(&task.id).await?;
        for item in &mut items {
            item.presigned_url = self.blob.0.presign_url(&item.key, ttl).await?;
            item.expires_in_secs = ttl;
        }
        Ok(ArtifactListResponse {
            task_id: task.id,
            items,
        })
    }

    /// Generates a temporary presigned URL for a specific task artifact.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`], [`ServerError::NotFound`], or [`ServerError::Validation`].
    pub async fn presign_artifact(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        filename: &str,
        ttl_secs: Option<u64>,
    ) -> Result<PresignedArtifactResponse, ServerError> {
        let task = self.tasks.get_owned(actor, task_id).await?;
        let safe_name = Self::validate_filename(filename)?;
        let key = format!("tasks/{}/{}/{}", task.session_id, task.id, safe_name);
        let ttl = self.effective_ttl(ttl_secs);
        let url = self.blob.0.presign_url(&key, ttl).await?;
        Ok(PresignedArtifactResponse {
            key,
            url,
            expires_in_secs: ttl,
        })
    }

    /// Reads raw artifact bytes for direct download fallback when using local storage.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn download_bytes(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        filename: &str,
    ) -> Result<(String, Vec<u8>), ServerError> {
        let task = self.tasks.get_owned(actor, task_id).await?;
        let safe_name = Self::validate_filename(filename)?;
        let key = format!("tasks/{}/{}/{}", task.session_id, task.id, safe_name);
        let index = self.load_index(&task.id).await?;
        let content_type = index
            .into_iter()
            .find(|item| item.filename == safe_name)
            .map_or_else(|| "application/octet-stream".to_owned(), |r| r.content_type);
        let bytes = self.blob.0.get_blob(&key).await?;
        Ok((content_type, bytes))
    }
}
