//! Task frontier queue service: idempotency, rate limits, admission backpressure,
//! NATS JetStream dispatch, fencing leases, and live steering.

mod ops;

use std::{sync::Arc, time::Duration};

use agents_protocol::{
    CursorPage, ExecutionScope, FencingToken, SteeringInterventionKind, SteeringMessage,
    StreamPayload, TaskLease, TaskRequest, TaskState, WorkerStatus,
};
use ironic::prelude::*;
use serde_json::json;

use crate::{
    infra::{
        adapters::VectorStoreClient,
        config::ServerConfig,
        error::ServerError,
        events::{AppEventBus, TaskCompletedEvent},
        state::now_ms,
        stream::{AdmissionStore, BusClient, StreamStore, emit_stream_event},
    },
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, guards::ensure_owner, services::StateStore},
        providers::services::ProviderService,
        secrets::services::SecretService,
        sessions::services::SessionService,
        skills::services::SkillService,
        tasks::{
            dto::{
                CompleteTaskDto, LeaseTaskDto, ListTasksQuery, SteerTaskDto, SubmitTaskDto,
                UpdateTaskStateDto,
            },
            entities::TaskEntity,
            repositories::TaskStore,
        },
    },
};

/// Orchestrates durable tasks across Postgres, Valkey, NATS JetStream, and Boxer nodes.
#[derive(Injectable)]
pub struct TaskService {
    store: Arc<TaskStore>,
    sessions: Arc<SessionService>,
    state: Arc<StateStore>,
    bus: Arc<BusClient>,
    streams: Arc<StreamStore>,
    admission: Arc<AdmissionStore>,
    vectors: Arc<VectorStoreClient>,
    config: Arc<ServerConfig>,
    audit: Arc<AuditService>,
    providers: Arc<ProviderService>,
    secrets: Arc<SecretService>,
    skills: Arc<SkillService>,
    events: Arc<AppEventBus>,
}

impl TaskService {
    /// Enqueues a new task with Valkey rate-limiting, `Idempotency-Key` deduplication,
    /// admission backpressure, and NATS JetStream dispatch.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RateLimited`], [`ServerError::Overloaded`], [`ServerError::Forbidden`],
    /// or [`ServerError::Validation`].
    pub async fn submit(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        idempotency_key: Option<&str>,
        dto: &SubmitTaskDto,
    ) -> Result<TaskEntity, ServerError> {
        let rl = self
            .state
            .0
            .check_rate_limit(
                &format!("tasks:{}", actor.user_id),
                self.config.rate_limit_max_requests,
                Duration::from_secs(self.config.rate_limit_window_secs),
            )
            .await?;
        if !rl.allowed {
            return Err(ServerError::RateLimited(
                "task submission rate limit exceeded".into(),
            ));
        }

        let session = self.sessions.get_owned(actor, session_id).await?;
        if let Some(sandbox) = &dto.sandbox {
            sandbox
                .validate()
                .map_err(|err| ServerError::Validation(err.to_string()))?;
        }

        if let Some(idem_key) = idempotency_key {
            let cache_key = format!("task:{session_id}:{idem_key}");
            if let Some(existing_id) = self.state.0.get_idempotency(&cache_key).await?
                && let Some(task) = self.store.0.find_by_id(&existing_id).await?
            {
                return Ok(task);
            }
            if let Some(existing) = self
                .store
                .0
                .find_by_idempotency(session_id, idem_key)
                .await?
            {
                let _ = self
                    .state
                    .0
                    .put_idempotency(&cache_key, &existing.id, Duration::from_secs(3600))
                    .await;
                return Ok(existing);
            }
        }

        let _permit = self
            .admission
            .0
            .try_acquire(ExecutionScope::Session, Some(session_id))?;

        let now = now_ms();
        let task = TaskEntity {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session.id.clone(),
            owner_id: actor.user_id.clone(),
            prompt: dto.prompt.clone(),
            state: TaskState::Queued,
            mode: dto.mode.unwrap_or(session.mode),
            fencing_token: FencingToken(0),
            lease_holder: None,
            lease_expires_at: None,
            idempotency_key: idempotency_key.map(ToOwned::to_owned),
            version: 1,
            exit_code: None,
            stop_reason: None,
            output_summary: None,
            created_at: now,
            updated_at: now,
        };

        let created = self.store.0.create(task).await?;
        if let Some(idem_key) = idempotency_key {
            let cache_key = format!("task:{session_id}:{idem_key}");
            let _ = self
                .state
                .0
                .put_idempotency(&cache_key, &created.id, Duration::from_secs(3600))
                .await;
        }

        let inference = self.providers.resolve_active_endpoint().await?;
        let proxy_credentials = self
            .secrets
            .resolve_proxy_credentials(&actor.user_id, Some(&created.session_id))
            .await?;
        let skills_prompt = self
            .skills
            .catalog_for_prompt(4000)
            .await
            .unwrap_or_default();
        let prompt = if skills_prompt.is_empty() {
            created.prompt.clone()
        } else {
            format!(
                "{}\n\n[Active Skills System Catalog]\n{}",
                created.prompt, skills_prompt
            )
        };
        let nats_req = TaskRequest {
            task_id: created.id.clone(),
            session_id: created.session_id.clone(),
            prompt,
            timeout_seconds: dto.timeout_seconds,
            mode: created.mode,
            fencing_token: None,
            idempotency_key: created.idempotency_key.clone(),
            sandbox: dto.sandbox.clone(),
            exec: dto.exec.clone().unwrap_or_default(),
            inference,
            proxy_credentials,
            lifecycle: dto.lifecycle.unwrap_or_default(),
        };
        self.bus.0.publish_task_request(&nats_req).await?;

        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &created.session_id,
            Some(&created.id),
            StreamPayload::WorkerStateChanged {
                node_id: "frontier-queue".to_owned(),
                status: WorkerStatus::Idle,
                reason: Some("task queued".to_owned()),
            },
        )
        .await?;

        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "task.submit",
                "task",
                Some(&created.id),
                "success",
                json!({ "sessionId": session_id }),
            )
            .await;

        Ok(created)
    }

    /// Lists tasks in a session after BOLA ownership check.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn list_for_session(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        query: &ListTasksQuery,
    ) -> Result<CursorPage<TaskEntity>, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        self.store
            .0
            .list_for_session(
                session_id,
                query.cursor.as_deref(),
                query.limit.unwrap_or(25),
            )
            .await
    }

    /// Loads a single task after enforcing strict BOLA ownership.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] or [`ServerError::Forbidden`].
    pub async fn get_owned(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
    ) -> Result<TaskEntity, ServerError> {
        let task = self
            .store
            .0
            .find_by_id(task_id)
            .await?
            .ok_or_else(|| ServerError::NotFound(format!("task `{task_id}` not found")))?;
        if ensure_owner(actor, &task.owner_id).is_err() {
            let _ = self
                .audit
                .record(
                    Some(&actor.user_id),
                    "task.access",
                    "task",
                    Some(task_id),
                    "denied_bola",
                    json!({ "ownerId": task.owner_id }),
                )
                .await;
            return Err(ServerError::Forbidden(
                "access denied: caller does not own target task".into(),
            ));
        }
        Ok(task)
    }

    /// Grants an exclusive fenced lease to a Boxer node (`409` if already actively leased).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] or [`ServerError::NotFound`].
    pub async fn lease(&self, task_id: &str, dto: &LeaseTaskDto) -> Result<TaskLease, ServerError> {
        let now = now_ms();
        let ttl_ms = dto.lease_ttl_seconds.unwrap_or(120).saturating_mul(1000);
        let expires_at = now.saturating_add(ttl_ms);

        let updated = self
            .store
            .0
            .acquire_lease(task_id, &dto.boxer_node_id, now, expires_at)
            .await?;

        let _ = self
            .state
            .0
            .acquire_lock(
                &format!("task_lease:{task_id}"),
                &dto.boxer_node_id,
                Duration::from_millis(ttl_ms),
            )
            .await;

        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &updated.session_id,
            Some(&updated.id),
            StreamPayload::WorkerStateChanged {
                node_id: dto.boxer_node_id.clone(),
                status: WorkerStatus::Working,
                reason: Some(format!("leased token {}", updated.fencing_token.0)),
            },
        )
        .await;

        Ok(TaskLease {
            task_id: updated.id,
            session_id: updated.session_id,
            boxer_node_id: dto.boxer_node_id.clone(),
            fencing_token: updated.fencing_token,
            acquired_at_ms: now,
            expires_at_ms: expires_at,
        })
    }

    /// Advances task state (`Running` / `Blocked`) after validating the monotonic [`FencingToken`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] on stale token or invalid transition.
    pub async fn transition_fenced(
        &self,
        task_id: &str,
        dto: &UpdateTaskStateDto,
    ) -> Result<TaskEntity, ServerError> {
        let updated = self
            .store
            .0
            .transition_fenced(task_id, dto.fencing_token, dto.state.clone(), now_ms())
            .await?;

        let worker_status = match updated.state {
            TaskState::Blocked => WorkerStatus::Blocked,
            _ => WorkerStatus::Working,
        };
        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &updated.session_id,
            Some(&updated.id),
            StreamPayload::WorkerStateChanged {
                node_id: updated
                    .lease_holder
                    .clone()
                    .unwrap_or_else(|| "boxer".to_owned()),
                status: worker_status,
                reason: dto.reason.clone(),
            },
        )
        .await;
        Ok(updated)
    }

    /// Settles a task (`Done` / `Failed`) after verifying the active [`FencingToken`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] when stale token or terminal conflict occurs.
    pub async fn complete_fenced(
        &self,
        task_id: &str,
        dto: &CompleteTaskDto,
    ) -> Result<TaskEntity, ServerError> {
        let updated = self
            .store
            .0
            .complete_fenced(
                task_id,
                dto.fencing_token,
                dto.exit_code,
                dto.stop_reason.clone(),
                dto.output_summary.clone(),
                now_ms(),
            )
            .await?;

        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &updated.session_id,
            Some(&updated.id),
            StreamPayload::WorkerStateChanged {
                node_id: updated
                    .lease_holder
                    .clone()
                    .unwrap_or_else(|| "boxer".to_owned()),
                status: WorkerStatus::Done,
                reason: Some(format!("exit_code={}", dto.exit_code)),
            },
        )
        .await;

        let _ = self
            .events
            .publish(TaskCompletedEvent {
                task_id: updated.id.clone(),
                session_id: updated.session_id.clone(),
                status: format!("{:?}", updated.state),
                completed_at: now_ms(),
            })
            .await;

        let len_score = (updated.prompt.len() as f32).min(1000.0) / 1000.0;
        let _ = self
            .vectors
            .0
            .upsert(
                &format!("task:{}", updated.id),
                &[
                    len_score,
                    if dto.exit_code == 0 { 1.0 } else { 0.0 },
                    0.5,
                    0.25,
                ],
                Some(json!({ "sessionId": updated.session_id, "exitCode": dto.exit_code })),
            )
            .await;

        Ok(updated)
    }

    /// Injects live steering instructions into an active task loop via NATS JetStream.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`], [`ServerError::NotFound`], or [`ServerError::Conflict`].
    pub async fn steer(
        &self,
        actor: &AuthenticatedActor,
        task_id: &str,
        dto: &SteerTaskDto,
    ) -> Result<SteeringMessage, ServerError> {
        let task = self.get_owned(actor, task_id).await?;
        if task.is_terminal() {
            return Err(ServerError::Conflict(
                "cannot steer a task that has already terminated".into(),
            ));
        }
        let effective_token = if let Some(provided) = dto.fencing_token {
            if provided < task.fencing_token {
                return Err(ServerError::Conflict(format!(
                    "stale steering fencing token {} < active {}",
                    provided.0, task.fencing_token.0
                )));
            }
            provided
        } else {
            task.fencing_token
        };
        let kind = dto.kind.unwrap_or_default();
        if kind == SteeringInterventionKind::FollowUpPrompt {
            let _ = self
                .queue_follow_up(
                    actor,
                    &task.session_id,
                    &crate::modules::tasks::dto::FollowUpDto {
                        content: dto.content.clone(),
                    },
                )
                .await;
        }
        let msg = SteeringMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            session_id: task.session_id.clone(),
            task_id: task.id.clone(),
            content: dto.content.clone(),
            kind,
            fencing_token: Some(effective_token),
            emitted_at_ms: now_ms(),
        };
        self.bus.0.publish_steering(&msg).await?;
        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &task.session_id,
            Some(&task.id),
            StreamPayload::SteeringAck {
                message_id: msg.message_id.clone(),
                accepted: true,
            },
        )
        .await;
        Ok(msg)
    }
}
