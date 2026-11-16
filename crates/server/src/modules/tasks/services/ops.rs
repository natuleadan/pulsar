//! Follow-up and cancellation operations for [`TaskService`].

use agents_protocol::{FollowUpMessage, StreamPayload, WorkerStatus};

use crate::{
    infra::{error::ServerError, state::now_ms, stream::emit_stream_event},
    modules::{
        auth::entities::AuthenticatedActor,
        tasks::{
            dto::{FollowUpDto, HardKillBoxResponse},
            entities::TaskEntity,
        },
    },
};

use super::TaskService;

impl TaskService {
    /// Looks up a task by ID without actor ownership check (internal worker dispatch only).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the task does not exist.
    pub async fn get_unscoped(&self, task_id: &str) -> Result<TaskEntity, ServerError> {
        self.store
            .0
            .find_by_id(task_id)
            .await?
            .ok_or_else(|| ServerError::NotFound("task not found".into()))
    }

    /// Enqueues an outer-loop follow-up prompt after BOLA ownership check.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn queue_follow_up(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &FollowUpDto,
    ) -> Result<FollowUpMessage, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        let follow_up = FollowUpMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_owned(),
            content: dto.content.clone(),
            queued_at_ms: now_ms(),
        };
        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            session_id,
            None,
            StreamPayload::TaskChunk {
                chunk: format!("[follow-up queued: {}]", follow_up.message_id),
                is_stderr: false,
            },
        )
        .await;
        Ok(follow_up)
    }

    /// Cancels a non-terminal task and broadcasts cancellation with its current fencing token.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`], [`ServerError::NotFound`], or [`ServerError::Conflict`].
    pub async fn cancel(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
    ) -> Result<TaskEntity, ServerError> {
        let existing = self.get_owned(actor, task_id).await?;
        let cancelled = self.store.0.cancel(&existing.id, now_ms()).await?;
        self.bus
            .0
            .publish_task_cancel(
                &cancelled.id,
                &cancelled.session_id,
                cancelled.fencing_token,
            )
            .await?;
        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &cancelled.session_id,
            Some(&cancelled.id),
            StreamPayload::WorkerStateChanged {
                node_id: "control-plane".to_owned(),
                status: WorkerStatus::Done,
                reason: Some("cancelled by user".to_owned()),
            },
        )
        .await;
        Ok(cancelled)
    }

    pub async fn hard_kill_box(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        box_id: &str,
    ) -> Result<HardKillBoxResponse, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        let _ = self
            .bus
            .0
            .publish_task_cancel(box_id, session_id, agents_protocol::FencingToken(u64::MAX))
            .await;
        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            session_id,
            Some(box_id),
            StreamPayload::WorkerStateChanged {
                node_id: "control-plane".to_owned(),
                status: WorkerStatus::Done,
                reason: Some("CancelledByOperator (hard kill switch)".to_owned()),
            },
        )
        .await;
        Ok(HardKillBoxResponse {
            session_id: session_id.to_owned(),
            box_id: box_id.to_owned(),
            killed: true,
            state: "CancelledByOperator".to_owned(),
        })
    }

    /// Reaps expired task leases and re-enqueues them with incremented fencing tokens.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on persistence errors.
    pub async fn sweep_expired_leases(&self, now_ms: u64) -> Result<Vec<TaskEntity>, ServerError> {
        let reaped = self.store.0.reap_expired_leases(now_ms).await?;
        for task in &reaped {
            let _ = emit_stream_event(
                &self.streams,
                &self.bus,
                &task.session_id,
                Some(&task.id),
                StreamPayload::WorkerStateChanged {
                    node_id: "task-sweeper".to_owned(),
                    status: WorkerStatus::Working,
                    reason: Some(format!(
                        "reaped expired lease, bumped fencing token to {}",
                        task.fencing_token.0
                    )),
                },
            )
            .await;
        }
        Ok(reaped)
    }
}
