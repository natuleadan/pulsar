use std::{collections::HashMap, path::PathBuf, sync::Arc};

use agents_protocol::{
    BoxerNodeAnnouncement, CodecId, EventEnvelope, ExecOptions, FollowUpMessage, NodeStatus,
    PermissionMask, PreviewRouteSync, SandboxProfile, SteeringMessage, StreamPayload, TaskRequest,
    TaskStreamEvent, WorkerStatus,
};
use serde_json::Value;
use std::sync::atomic::Ordering;
use tokio::sync::{mpsc, watch};

use crate::{
    config::{BoxerConfig, now_ms},
    daemon::{BoxerBus, BoxerDaemon},
    error::BoxerError,
    proxy::CredentialRule,
    runtime::{
        DefaultLoopHooks, DurableReplayTracker, DurableStepLogger, HttpProviderClient,
        JsonlFileDurableLogger, NativeAgentRuntime, NativeRuntimeConfig, ProviderEndpointConfig,
        ProviderKind, ProviderRegistry, RetryPolicy, RuntimeTool, SessionEntry, SessionTree,
    },
    sandbox::{BoxFuture, SandboxDriver, SandboxExecOutput, SandboxExecRequest},
    swarm::{build_signed_swarm_message, verify_swarm_signature},
};

/// Builds upstream proxy credential injection rules from `BoxerConfig`.
#[must_use]
pub fn build_proxy_credentials(config: &BoxerConfig) -> Vec<CredentialRule> {
    let mut rules = Vec::new();
    for (host, raw_header) in &config.proxy_upstream_headers {
        if let Some((name, value)) = raw_header.split_once(':') {
            rules.push(CredentialRule {
                host: host.clone(),
                header_name: name.trim().to_owned(),
                header_value: value.trim().to_owned(),
            });
        } else {
            rules.push(CredentialRule {
                host: host.clone(),
                header_name: "authorization".to_owned(),
                header_value: raw_header.clone(),
            });
        }
    }
    if let Some(tok) = &config.upstash_box_token {
        rules.push(CredentialRule {
            host: "box.upstash.io".to_owned(),
            header_name: "authorization".to_owned(),
            header_value: format!("Bearer {tok}"),
        });
    }
    if let Some(tok) = &config.deepinfra_token {
        rules.push(CredentialRule {
            host: "api.deepinfra.com".to_owned(),
            header_name: "authorization".to_owned(),
            header_value: format!("Bearer {tok}"),
        });
    }
    rules
}

struct SandboxShellTool {
    driver: Arc<dyn SandboxDriver>,
    task_id: String,
    session_id: String,
    profile: SandboxProfile,
    exec: ExecOptions,
    proxy_url: Option<String>,
    stream_tx: mpsc::Sender<TaskStreamEvent>,
    cancel_rx: watch::Receiver<bool>,
}

impl RuntimeTool for SandboxShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn is_mutating(&self) -> bool {
        true
    }

    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>> {
        Box::pin(async move {
            let command = input
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| BoxerError::ExecutionFailed("shell tool requires 'command'".into()))?
                .to_owned();
            let out = self
                .driver
                .execute(
                    SandboxExecRequest {
                        task_id: self.task_id.clone(),
                        session_id: self.session_id.clone(),
                        command,
                        profile: self.profile.clone(),
                        exec: self.exec.clone(),
                        proxy_url: self.proxy_url.clone(),
                    },
                    Some(self.stream_tx.clone()),
                    self.cancel_rx.clone(),
                )
                .await?;
            Ok(format!(
                "exit_code={}\nstdout:\n{}\nstderr:\n{}",
                out.exit_code, out.stdout, out.stderr
            ))
        })
    }
}

fn detect_llm_endpoint() -> Option<ProviderEndpointConfig> {
    if let Ok(key) = std::env::var("OPENAI_API_KEY")
        && !key.trim().is_empty()
    {
        return Some(ProviderEndpointConfig {
            id: "default".to_owned(),
            kind: ProviderKind::OpenAi,
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned()),
            model: std::env::var("AGENTS_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned()),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            default_headers: HashMap::new(),
        });
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
        && !key.trim().is_empty()
    {
        return Some(ProviderEndpointConfig {
            id: "default".to_owned(),
            kind: ProviderKind::Anthropic,
            base_url: std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com".to_owned()),
            model: std::env::var("AGENTS_LLM_MODEL")
                .unwrap_or_else(|_| "claude-3-5-sonnet-latest".to_owned()),
            api_key_env: Some("ANTHROPIC_API_KEY".to_owned()),
            default_headers: HashMap::new(),
        });
    }
    if let Ok(base_url) = std::env::var("AGENTS_LLM_ENDPOINT")
        && !base_url.trim().is_empty()
    {
        return Some(ProviderEndpointConfig {
            id: "default".to_owned(),
            kind: ProviderKind::OpenAi,
            base_url,
            model: std::env::var("AGENTS_LLM_MODEL").unwrap_or_else(|_| "llama3.2".to_owned()),
            api_key_env: std::env::var("AGENTS_LLM_API_KEY_ENV").ok(),
            default_headers: HashMap::new(),
        });
    }
    None
}

/// Context passed to [`try_run_agent_engine`].
pub struct AgentEngineContext<'a> {
    pub req: &'a TaskRequest,
    pub profile: &'a SandboxProfile,
    pub exec_opts: &'a ExecOptions,
    pub proxy_url: Option<String>,
    pub driver: Arc<dyn SandboxDriver>,
    pub bus: Arc<dyn BoxerBus>,
    pub stream_tx: mpsc::Sender<TaskStreamEvent>,
    pub steering_rx: Option<mpsc::Receiver<SteeringMessage>>,
    pub cancel_rx: watch::Receiver<bool>,
    pub working_dir: PathBuf,
}

/// Runs `NativeAgentRuntime` when an LLM provider is configured and the prompt is not a raw `sh:` command.
pub async fn try_run_agent_engine(
    ctx: AgentEngineContext<'_>,
) -> Option<Result<SandboxExecOutput, BoxerError>> {
    let AgentEngineContext {
        req,
        profile,
        exec_opts,
        proxy_url,
        driver,
        bus,
        stream_tx,
        steering_rx,
        cancel_rx,
        working_dir,
    } = ctx;
    if req.prompt.starts_with("sh:") {
        return None;
    }
    let endpoint = if let Some(spec) = &req.inference {
        let kind = match spec.kind.trim().to_ascii_lowercase().as_str() {
            "anthropic" => ProviderKind::Anthropic,
            _ => ProviderKind::OpenAi,
        };
        let mut default_headers = HashMap::new();
        if let Some(key) = spec
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match kind {
                ProviderKind::Anthropic => {
                    default_headers.insert("x-api-key".to_owned(), key.to_owned());
                }
                _ => {
                    default_headers.insert("authorization".to_owned(), format!("Bearer {key}"));
                }
            }
        }
        ProviderEndpointConfig {
            id: "default".to_owned(),
            kind,
            base_url: spec.base_url.clone(),
            model: spec.model.clone(),
            api_key_env: None,
            default_headers,
        }
    } else {
        detect_llm_endpoint()?
    };

    let mut registry = ProviderRegistry::new();
    let endpoint_id = endpoint.id.clone();
    registry.register(endpoint);
    let model = Arc::new(HttpProviderClient::new(Arc::new(registry)));
    let mut runtime = NativeAgentRuntime::new(model, Arc::new(DefaultLoopHooks));
    runtime.register_tool(SandboxShellTool {
        driver,
        task_id: req.task_id.clone(),
        session_id: req.session_id.clone(),
        profile: profile.clone(),
        exec: exec_opts.clone(),
        proxy_url,
        stream_tx,
        cancel_rx: cancel_rx.clone(),
    });

    let mut tree = SessionTree::new(req.session_id.clone());
    tree.append(SessionEntry::UserMessage {
        text: req.prompt.clone(),
    });
    let (ev_tx, mut ev_rx) = mpsc::channel::<EventEnvelope>(128);
    let bus_clone = Arc::clone(&bus);
    let ev_forwarder = tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let _ = bus_clone.publish_stream_event(&ev).await;
        }
    });

    let (_unused_steer_tx, fallback_steer_rx) = mpsc::channel::<SteeringMessage>(8);
    let (_follow_tx, follow_rx) = mpsc::channel::<FollowUpMessage>(8);
    let steer_rx = steering_rx.unwrap_or(fallback_steer_rx);

    let rt_cfg = NativeRuntimeConfig {
        task_id: req.task_id.clone(),
        endpoint_id,
        mode: req.mode,
        permissions: PermissionMask {
            can_read_fs: true,
            can_write_fs: true,
            can_exec_shell: true,
            can_network: req.exec.network_egress,
            allowed_tools: Vec::new(),
        },
        max_turns: 16,
        compact_keep_tail: 8,
        retry_policy: RetryPolicy {
            max_retries: 2,
            base_delay_ms: 250,
            max_delay_ms: 2_000,
        },
        output_contract: None,
    };

    let durable_logger = JsonlFileDurableLogger::new(&working_dir);
    let mut replay_tracker = DurableReplayTracker::new(&req.task_id, &req.session_id);
    if let Ok(prior) = durable_logger.list_steps(&req.task_id).await {
        replay_tracker.recover_from_steps(&prior);
    }

    let outcome = runtime
        .run(&rt_cfg, &mut tree, ev_tx, steer_rx, follow_rx, cancel_rx)
        .await;
    let _ = ev_forwarder.await;

    match outcome {
        Ok(out) => {
            let step_rec = replay_tracker
                .build_checkpoint_step(serde_json::json!({ "final_text": out.final_text }));
            let logger_clone = durable_logger.clone();
            tokio::spawn(async move {
                let _ = logger_clone.append_step(step_rec).await;
            });
            let _ = bus
                .publish_stream_event(&EventEnvelope {
                    seq: 0,
                    emitted_at_ms: now_ms(),
                    codec: agents_protocol::CodecId::EventsV1,
                    session_id: req.session_id.clone(),
                    task_id: Some(req.task_id.clone()),
                    payload: StreamPayload::TaskChunk {
                        chunk: out.final_text.clone(),
                        is_stderr: false,
                    },
                })
                .await;
            Some(Ok(SandboxExecOutput {
                exit_code: 0,
                stdout: out.final_text,
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                working_dir,
            }))
        }
        Err(err) => Some(Err(err)),
    }
}

impl BoxerDaemon {
    pub(crate) async fn emit_worker_status(
        &self,
        session_id: &str,
        task_id: Option<&str>,
        status: WorkerStatus,
        reason: Option<&str>,
    ) {
        let env = EventEnvelope {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            emitted_at_ms: now_ms(),
            codec: CodecId::EventsV1,
            session_id: session_id.to_owned(),
            task_id: task_id.map(ToOwned::to_owned),
            payload: StreamPayload::WorkerStateChanged {
                node_id: self.config.node_id.clone(),
                status,
                reason: reason.map(ToOwned::to_owned),
            },
        };
        let _ = self.bus.publish_stream_event(&env).await;
    }

    /// Announces this Boxer worker onto the private VLAN discovery bus (`boxer.nodes.*`).
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Bus`] if the bus publication fails.
    pub async fn announce_node(
        &self,
        vlan_ip: &str,
        status: NodeStatus,
    ) -> Result<BoxerNodeAnnouncement, BoxerError> {
        let active_boxes = self
            .config
            .max_concurrent_boxes
            .saturating_sub(self.semaphore.available_permits());
        let announcement = BoxerNodeAnnouncement {
            node_id: self.config.node_id.clone(),
            vlan_ip: vlan_ip.to_owned(),
            runtime: self.config.default_profile.runtime.clone(),
            isolation: self.config.default_profile.isolation.clone(),
            max_concurrent_boxes: self.config.max_concurrent_boxes,
            active_boxes,
            status,
            last_heartbeat_ms: now_ms(),
            ttl_seconds: 60,
        };
        self.bus.publish_node_announcement(&announcement).await?;
        Ok(announcement)
    }

    /// Publishes or revokes a distributed preview route sync message for Traefik HTTP discovery.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Bus`] if synchronization fails.
    pub async fn sync_preview_route(&self, sync: &PreviewRouteSync) -> Result<(), BoxerError> {
        self.bus.publish_preview_sync(sync).await
    }

    /// Builds and verifies a signed P2P swarm message using the node cluster secret.
    pub fn build_verified_swarm_message(
        &self,
        session_id: &str,
        to: agents_protocol::SwarmTopic,
        payload: Value,
        permission_mask: PermissionMask,
    ) -> Result<agents_protocol::SwarmMessage, BoxerError> {
        let msg = build_signed_swarm_message(
            self.config.cluster_secret.as_bytes(),
            session_id,
            &self.config.node_id,
            to,
            payload,
            permission_mask,
        );
        verify_swarm_signature(self.config.cluster_secret.as_bytes(), &msg)?;
        Ok(msg)
    }
}

/// Uploads packaged task artifacts to the control-plane `/api/v1/tasks/:id/artifacts/worker` endpoint.
pub async fn upload_packaged_artifacts(
    config: &BoxerConfig,
    task_id: &str,
    working_dir: &std::path::Path,
    manifest: &agents_protocol::ArtifactManifest,
) -> usize {
    use base64::Engine;

    if manifest.files.is_empty() {
        return 0;
    }
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/tasks/{task_id}/artifacts/worker",
        config.server_internal_url.trim_end_matches('/')
    );
    let token = if !config.worker_token.is_empty() {
        config.worker_token.as_str()
    } else {
        config.cluster_secret.as_str()
    };
    let mut uploaded = 0_usize;
    for entry in &manifest.files {
        let path = working_dir.join(&entry.path);
        let Ok(bytes) = tokio::fs::read(&path).await else {
            continue;
        };
        let filename = std::path::Path::new(&entry.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("artifact.bin");
        let payload = serde_json::json!({
            "filename": filename,
            "contentType": entry.content_type,
            "dataBase64": base64::engine::general_purpose::STANDARD.encode(&bytes),
        });
        match client
            .post(&url)
            .header("x-boxer-worker-token", token)
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => uploaded += 1,
            Ok(resp) => tracing::warn!(
                task_id = %task_id,
                filename = %filename,
                status = %resp.status(),
                "artifact upload rejected by control plane"
            ),
            Err(err) => tracing::warn!(
                task_id = %task_id,
                filename = %filename,
                error = %err,
                "artifact upload request failed"
            ),
        }
    }
    uploaded
}
