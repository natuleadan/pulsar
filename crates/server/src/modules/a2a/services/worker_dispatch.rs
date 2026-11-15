use std::sync::Arc;

use a2a::{
    Artifact, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse, Message, Part, Role,
    TaskState, TaskStatus,
};
use agents_protocol::{
    FencingToken, InteractionMode, IsolationKind, RuntimeKind, SandboxProfile, SetupMode,
    TaskRequest,
};
use axum::http::HeaderMap;
use chrono::Utc;
use reqwest::Url;
use serde_json::{Value, json};
use tracing::info;

use crate::{
    infra::{
        bus::MessageBus,
        error::ServerError,
        events::{A2aInboundTaskEvent, AppEventBus},
    },
    modules::{a2a::repositories::A2aTaskStore, identity::services::OutboundInteractionRecord},
};

use super::{A2aService, JSONRPC_INVALID_PARAMS, OutboundA2aResponse, OutboundA2aSendDto};

/// Spawns an async subscriber for [`A2aInboundTaskEvent`] dispatching tasks to workers.
pub fn spawn_inbound_dispatcher(
    events: Arc<AppEventBus>,
    store: Arc<A2aTaskStore>,
    bus: Option<Arc<dyn MessageBus>>,
) {
    tokio::spawn(async move {
        let mut sub = events.subscribe::<A2aInboundTaskEvent>(64).await;
        while let Some(ev) = sub.recv().await {
            let Some(mut task) = store.0.get_task(&ev.task_id).await.ok().flatten() else {
                continue;
            };
            let prompt = task
                .history
                .as_ref()
                .and_then(|h| h.last())
                .and_then(|m| m.text())
                .unwrap_or("")
                .to_owned();

            if let Some(ref message_bus) = bus {
                let req = TaskRequest {
                    task_id: ev.task_id.clone(),
                    session_id: ev.context_id.clone(),
                    prompt: prompt.clone(),
                    mode: InteractionMode::Act,
                    fencing_token: Some(FencingToken(1)),
                    idempotency_key: Some(format!("a2a-{}", ev.task_id)),
                    sandbox: Some(SandboxProfile {
                        mode: SetupMode::Local,
                        runtime: RuntimeKind::Docker,
                        isolation: IsolationKind::Openshell,
                        allow_host_native: false,
                        gateway_url: None,
                    }),
                    exec: Default::default(),
                    timeout_seconds: Some(120),
                    inference: None,
                };
                let _ = message_bus.publish_task_request(&req).await;
            }

            let completion_msg = Message {
                message_id: uuid::Uuid::new_v4().to_string(),
                context_id: Some(ev.context_id.clone()),
                task_id: Some(ev.task_id.clone()),
                role: Role::Agent,
                parts: vec![Part::text(format!(
                    "Dispatched by orchestrator worker gateway (task={}, context={}, caller_org={})",
                    ev.task_id,
                    ev.context_id,
                    ev.caller_org.as_deref().unwrap_or("anonymous")
                ))],
                metadata: None,
                extensions: None,
                reference_task_ids: None,
            };
            task.status = TaskStatus {
                state: TaskState::Completed,
                message: Some(completion_msg),
                timestamp: Some(Utc::now()),
            };
            if let Some(ref mut artifacts) = task.artifacts {
                artifacts.push(Artifact {
                    artifact_id: uuid::Uuid::new_v4().to_string(),
                    name: Some("worker-dispatch-receipt".to_owned()),
                    description: Some("Orchestrator worker execution receipt".to_owned()),
                    parts: vec![Part::data(json!({
                        "taskId": ev.task_id,
                        "dispatchedToBus": bus.is_some(),
                        "promptChars": prompt.len(),
                    }))],
                    metadata: None,
                    extensions: None,
                });
            }
            let _ = store
                .0
                .save_task(
                    task,
                    ev.caller_org.clone(),
                    ev.caller_installation_id.clone(),
                )
                .await;
            info!(task_id = %ev.task_id, "dispatched inbound A2A task to worker");
        }
    });
}

impl A2aService {
    /// Sends an outbound A2A JSON-RPC `message/send` call with mandatory installation identity headers.
    pub async fn send_outbound(
        &self,
        dto: &OutboundA2aSendDto,
    ) -> Result<OutboundA2aResponse, ServerError> {
        let parsed = Url::parse(&dto.target_url)
            .map_err(|e| ServerError::Validation(format!("invalid A2A targetUrl: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ServerError::Validation(
                "A2A targetUrl must use http or https".into(),
            ));
        }

        let mut headers = HeaderMap::new();
        let active_identity = self.identity.inject_outbound_headers(&mut headers).await?;

        let mut msg = Message::new(Role::User, vec![Part::text(dto.text.clone())]);
        msg.context_id = dto.context_id.clone();

        let rpc_req = JsonRpcRequest::new(
            JsonRpcId::String(uuid::Uuid::new_v4().to_string()),
            "message/send",
            Some(json!({ "message": msg })),
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ServerError::Internal(format!("a2a client build failed: {e}")))?;

        let resp = client
            .post(parsed.as_str())
            .headers(headers)
            .json(&rpc_req)
            .send()
            .await
            .map_err(|e| ServerError::Validation(format!("outbound A2A request failed: {e}")))?;

        let status = resp.status().as_u16();
        let body: Value = resp
            .json()
            .await
            .unwrap_or_else(|_| json!({ "rawStatus": status }));

        let _ = self
            .identity
            .record_outbound(OutboundInteractionRecord {
                protocol: "a2a",
                target_host: parsed.host_str(),
                endpoint_path: parsed.path(),
                method: "POST",
                status_code: Some(i32::from(status)),
                task_id: None,
                identity: Some(&active_identity),
            })
            .await;

        Ok(OutboundA2aResponse {
            target_url: dto.target_url.clone(),
            status_code: status,
            injected_org: active_identity.organization_name,
            injected_installation_id: active_identity.installation_id,
            response: body,
        })
    }
}

pub(super) fn extract_task_id(params: Option<&Value>) -> Option<String> {
    params?
        .get("id")
        .or_else(|| params?.get("taskId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(super) fn invalid_params(id: JsonRpcId, msg: &str) -> JsonRpcResponse {
    JsonRpcResponse::error(
        id,
        JsonRpcError {
            code: JSONRPC_INVALID_PARAMS,
            message: msg.to_owned(),
            data: None,
        },
    )
}
