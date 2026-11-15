//! Follow-up and cancellation operations for [`TaskService`].

use agents_protocol::{FollowUpMessage, StreamPayload, WorkerStatus};

use crate::{
    infra::{error::ServerError, state::now_ms, stream::emit_stream_event},
    modules::{
        auth::entities::AuthenticatedActor,
        tasks::{dto::FollowUpDto, entities::TaskEntity},
    },
};

use super::TaskService;

impl TaskService {
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
}
