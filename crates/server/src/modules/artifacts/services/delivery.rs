//! Phase-based crash resumption checkpoints, verified SigV4 deliveries,
//! and reference-counted blob garbage collection tracking.

use base64::Engine;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    infra::{error::ServerError, state::now_ms},
    modules::{
        artifacts::{
            dto::{
                DeliverVerifiedArtifactDto, MediaBlobRefRecord, RecordTaskPhaseDto,
                TaskCrashResumePlan, TaskPhaseCheckpoint, UploadArtifactDto,
                VerifiedDeliveryReceipt,
            },
            services::ArtifactService,
        },
        auth::entities::AuthenticatedActor,
    },
};

impl ArtifactService {
    fn phase_key(task_id: &str) -> String {
        format!("task:phases:{task_id}")
    }

    fn blob_ref_key(sha256: &str) -> String {
        format!("blob:ref:{sha256}")
    }

    pub async fn record_phase_checkpoint(
        &self,
        task_id: &str,
        dto: &RecordTaskPhaseDto,
    ) -> Result<TaskPhaseCheckpoint, ServerError> {
        let task = self.tasks.get_unscoped(task_id).await?;
        if dto.fencing_token < task.fencing_token.0 {
            return Err(ServerError::Conflict(format!(
                "stale task phase fencing token {} < active {}",
                dto.fencing_token, task.fencing_token.0
            )));
        }
        let checkpoint = TaskPhaseCheckpoint {
            task_id: task_id.to_owned(),
            phase_name: dto.phase_name.clone(),
            output_sha256: dto.output_sha256.clone(),
            fencing_token: dto.fencing_token,
            recorded_at_ms: now_ms(),
        };
        let key = Self::phase_key(task_id);
        let mut list: Vec<TaskPhaseCheckpoint> = match self.state.0.load_draft(&key).await? {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            None => Vec::new(),
        };
        list.retain(|p| p.phase_name != checkpoint.phase_name);
        list.push(checkpoint.clone());
        let json = serde_json::to_string(&list)
            .map_err(|e| ServerError::Internal(format!("serialize phases: {e}")))?;
        self.state.0.save_draft(&key, &json).await?;
        Ok(checkpoint)
    }

    pub async fn get_crash_resume_plan(
        &self,
        task_id: &str,
    ) -> Result<TaskCrashResumePlan, ServerError> {
        let task = self.tasks.get_unscoped(task_id).await?;
        let key = Self::phase_key(task_id);
        let completed_phases: Vec<TaskPhaseCheckpoint> = match self.state.0.load_draft(&key).await?
        {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            None => Vec::new(),
        };
        let known_phases = ["inspect", "patch", "compile", "test", "deliver"];
        let next_pending_phase = known_phases
            .iter()
            .find(|expected| !completed_phases.iter().any(|c| c.phase_name == **expected))
            .map(|s| (*s).to_owned());

        Ok(TaskCrashResumePlan {
            task_id: task_id.to_owned(),
            active_fencing_token: task.fencing_token.0,
            completed_phases,
            next_pending_phase,
        })
    }

    pub async fn deliver_verified_artifact(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        dto: &DeliverVerifiedArtifactDto,
    ) -> Result<VerifiedDeliveryReceipt, ServerError> {
        let task = self.tasks.get_owned(actor, task_id).await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(dto.data_base64.trim())
            .map_err(|_| ServerError::Validation("invalid base64 payload".into()))?;
        let computed_digest = hex::encode(Sha256::digest(&bytes));
        if !bool::from(
            computed_digest
                .as_bytes()
                .ct_eq(dto.expected_sha256.to_lowercase().as_bytes()),
        ) {
            return Err(ServerError::Validation(format!(
                "artifact SHA-256 digest mismatch: expected {}, got {computed_digest}",
                dto.expected_sha256
            )));
        }

        let upload_dto = UploadArtifactDto {
            filename: dto.filename.clone(),
            content_type: dto.content_type.clone(),
            data_base64: dto.data_base64.clone(),
            ttl_secs: Some(3600),
        };
        let stored = self
            .store_for_task(&task.id, &task.session_id, &upload_dto)
            .await?;

        let ref_key = Self::blob_ref_key(&computed_digest);
        let mut blob_ref: MediaBlobRefRecord = match self.state.0.load_draft(&ref_key).await? {
            Some(raw) => serde_json::from_str(&raw).unwrap_or(MediaBlobRefRecord {
                content_sha256: computed_digest.clone(),
                key: stored.key.clone(),
                size_bytes: stored.size_bytes,
                ref_count: 0,
                marked_orphan_at_ms: None,
            }),
            None => MediaBlobRefRecord {
                content_sha256: computed_digest.clone(),
                key: stored.key.clone(),
                size_bytes: stored.size_bytes,
                ref_count: 0,
                marked_orphan_at_ms: None,
            },
        };
        blob_ref.ref_count = blob_ref.ref_count.saturating_add(1);
        blob_ref.marked_orphan_at_ms = None;
        let blob_json = serde_json::to_string(&blob_ref)
            .map_err(|e| ServerError::Internal(format!("serialize blob ref: {e}")))?;
        self.state.0.save_draft(&ref_key, &blob_json).await?;

        let sig_payload = format!(
            "{}:{}:{computed_digest}:{}",
            task.id, stored.filename, stored.created_at_ms
        );
        let signature = hex::encode(Sha256::digest(
            format!("{}:{sig_payload}", self.config.cluster_secret).as_bytes(),
        ));

        Ok(VerifiedDeliveryReceipt {
            receipt_id: uuid::Uuid::new_v4().to_string(),
            task_id: task.id,
            session_id: task.session_id,
            filename: stored.filename,
            sha256_digest: computed_digest,
            ref_count: blob_ref.ref_count,
            delivery_signature: signature,
            presigned_url: stored.presigned_url,
            delivered_at_ms: stored.created_at_ms,
        })
    }

    pub async fn get_blob_ref(
        &self,
        content_sha256: &str,
    ) -> Result<Option<MediaBlobRefRecord>, ServerError> {
        let key = Self::blob_ref_key(content_sha256);
        let Some(raw) = self.state.0.load_draft(&key).await? else {
            return Ok(None);
        };
        Ok(serde_json::from_str(&raw).ok())
    }
}
