//! Multi-provider model endpoint registry supporting Ollama, LMStudio, OpenAI,
//! Anthropic, DeepInfra, and infinite custom `provider_endpoint:<id>` targets.

use std::{collections::HashMap, sync::Arc};

use pulsar_protocol::{
    ProviderErrorClass, StopReason, classify_provider_error, scrub_sensitive_tokens,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    error::BoxerError,
    leases::BoxFuture,
    runtime::{engine::ToolCallRequest, session_tree::SessionEntry},
};

/// Supported LLM inference backend adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Ollama,
    LmStudio,
    OpenAi,
    Anthropic,
    DeepInfra,
    Custom(String),
}

impl ProviderKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepInfra => "deepinfra",
            Self::Custom(id) => id.as_str(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpointConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub fallback_endpoint_id: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub default_headers: HashMap<String, String>,
}

impl ProviderEndpointConfig {
    /// Canonical config key used when referencing dynamic custom endpoints.
    #[must_use]
    pub fn registry_key(&self) -> String {
        if self.id.starts_with("provider_endpoint:") {
            self.id.clone()
        } else {
            format!("provider_endpoint:{}", self.id)
        }
    }
}

/// Registry holding infinite custom `provider_endpoint:<id>` and built-in endpoints.
#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    endpoints: HashMap<String, ProviderEndpointConfig>,
}

impl ProviderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cfg: ProviderEndpointConfig) {
        let key = cfg.registry_key();
        self.endpoints.insert(cfg.id.clone(), cfg.clone());
        self.endpoints.insert(key, cfg);
    }

    #[must_use]
    pub fn resolve(&self, id: &str) -> Option<&ProviderEndpointConfig> {
        self.endpoints
            .get(id)
            .or_else(|| self.endpoints.get(&format!("provider_endpoint:{id}")))
    }

    #[must_use]
    pub fn list(&self) -> Vec<&ProviderEndpointConfig> {
        self.endpoints
            .iter()
            .filter(|(k, _)| k.starts_with("provider_endpoint:"))
            .map(|(_, v)| v)
            .collect()
    }
}

/// Normalized request forwarded to a model provider for one inner-loop inference step.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTurnRequest {
    pub session_id: String,
    pub turn_index: u32,
    pub endpoint_id: String,
    pub entries: Vec<SessionEntry>,
    pub available_tools: Vec<String>,
    pub json_schema_mode: bool,
}

/// Normalized model inference output driving tool batches or loop termination.
#[derive(Debug, Clone)]
pub struct ModelTurnResponse {
    pub assistant_text: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub stop_reason: StopReason,
    pub usage_tokens: u64,
}

/// Model client abstraction decouples inference transport from loop logic.
pub trait ModelClient: Send + Sync + 'static {
    fn complete_turn<'a>(
        &'a self,
        req: ModelTurnRequest,
    ) -> BoxFuture<'a, Result<ModelTurnResponse, BoxerError>>;
}

/// HTTP model adapter dispatching normalized requests to configured provider endpoints.
#[derive(Clone)]
pub struct HttpProviderClient {
    registry: Arc<ProviderRegistry>,
    http: reqwest::Client,
}

impl HttpProviderClient {
    #[must_use]
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self { registry, http }
    }

    fn build_provider_payload(cfg: &ProviderEndpointConfig, req: &ModelTurnRequest) -> Value {
        let messages: Vec<Value> = req
            .entries
            .iter()
            .map(|entry| json!({ "role": entry.role_label(), "content": entry.summary_text() }))
            .collect();
        match cfg.kind {
            ProviderKind::Anthropic => json!({
                "model": cfg.model,
                "max_tokens": 2048,
                "messages": messages,
            }),
            ProviderKind::Ollama => json!({
                "model": cfg.model,
                "stream": false,
                "messages": messages,
            }),
            ProviderKind::LmStudio
            | ProviderKind::OpenAi
            | ProviderKind::DeepInfra
            | ProviderKind::Custom(_) => json!({
                "model": cfg.model,
                "messages": messages,
            }),
        }
    }

    fn target_url(cfg: &ProviderEndpointConfig) -> String {
        let base = cfg.base_url.trim_end_matches('/');
        match cfg.kind {
            ProviderKind::Ollama => format!("{base}/api/chat"),
            ProviderKind::Anthropic => format!("{base}/v1/messages"),
            ProviderKind::LmStudio
            | ProviderKind::OpenAi
            | ProviderKind::DeepInfra
            | ProviderKind::Custom(_) => {
                if base.ends_with("/v1") || base.ends_with("/openai") {
                    format!("{base}/chat/completions")
                } else {
                    format!("{base}/v1/chat/completions")
                }
            }
        }
    }

    async fn execute_once(
        &self,
        cfg: &ProviderEndpointConfig,
        req: &ModelTurnRequest,
    ) -> Result<ModelTurnResponse, (Option<u16>, String)> {
        let url = Self::target_url(cfg);
        let payload = Self::build_provider_payload(cfg, req);

        let mut req_builder = self.http.post(&url).json(&payload);
        for (k, v) in &cfg.default_headers {
            req_builder = req_builder.header(k.as_str(), v.as_str());
        }
        if let Some(env_name) = &cfg.api_key_env
            && let Ok(secret) = std::env::var(env_name)
            && !secret.trim().is_empty()
        {
            match cfg.kind {
                ProviderKind::Anthropic => {
                    req_builder = req_builder.header("x-api-key", secret);
                }
                _ => {
                    req_builder = req_builder.bearer_auth(secret);
                }
            }
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| (None, format!("provider request failed: {e}")))?;
        let status = resp.status();
        let raw = resp
            .bytes()
            .await
            .map_err(|e| (None, format!("provider read error: {e}")))?;

        if !status.is_success() {
            return Err((
                Some(status.as_u16()),
                String::from_utf8_lossy(&raw).into_owned(),
            ));
        }

        let parsed: Value = serde_json::from_slice(&raw)
            .map_err(|e| (Some(200), format!("invalid provider json: {e}")))?;

        let assistant_text = parsed
            .pointer("/choices/0/message/content")
            .or_else(|| parsed.pointer("/message/content"))
            .or_else(|| parsed.pointer("/content/0/text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let (tool_calls, stop_reason, usage_tokens) = parse_tool_calls_and_reason(&parsed);

        Ok(ModelTurnResponse {
            assistant_text,
            tool_calls,
            stop_reason,
            usage_tokens,
        })
    }
}

pub fn parse_tool_calls_and_reason(parsed: &Value) -> (Vec<ToolCallRequest>, StopReason, u64) {
    let mut tool_calls = Vec::new();

    if let Some(calls) = parsed
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    {
        for call in calls {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let tool_name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let input = if let Some(args_str) =
                call.pointer("/function/arguments").and_then(Value::as_str)
            {
                serde_json::from_str(args_str).unwrap_or_else(|_| json!({ "raw": args_str }))
            } else if let Some(args_val) = call.pointer("/function/arguments") {
                args_val.clone()
            } else {
                Value::Null
            };
            if !tool_name.is_empty() {
                tool_calls.push(ToolCallRequest {
                    call_id,
                    tool_name,
                    input,
                });
            }
        }
    }

    if let Some(content_items) = parsed.pointer("/content").and_then(Value::as_array) {
        for item in content_items {
            if item.get("type").and_then(Value::as_str) == Some("tool_use") {
                let call_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let tool_name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let input = item.get("input").cloned().unwrap_or(Value::Null);
                if !tool_name.is_empty() {
                    tool_calls.push(ToolCallRequest {
                        call_id,
                        tool_name,
                        input,
                    });
                }
            }
        }
    }

    let finish_reason = parsed
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str);
    let anthropic_stop = parsed.pointer("/stop_reason").and_then(Value::as_str);

    let stop_reason = if !tool_calls.is_empty() {
        StopReason::ToolUse
    } else if finish_reason == Some("length") || anthropic_stop == Some("max_tokens") {
        StopReason::Length
    } else {
        StopReason::EndTurn
    };

    let usage_tokens = parsed
        .pointer("/usage/total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            let inp = parsed
                .pointer("/usage/input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let out = parsed
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if inp + out > 0 { Some(inp + out) } else { None }
        })
        .unwrap_or(0);

    (tool_calls, stop_reason, usage_tokens)
}

impl ModelClient for HttpProviderClient {
    fn complete_turn<'a>(
        &'a self,
        req: ModelTurnRequest,
    ) -> BoxFuture<'a, Result<ModelTurnResponse, BoxerError>> {
        Box::pin(async move {
            let primary_cfg = self
                .registry
                .resolve(&req.endpoint_id)
                .ok_or_else(|| {
                    BoxerError::Config(format!(
                        "provider endpoint `{}` is not registered",
                        req.endpoint_id
                    ))
                })?
                .clone();

            let max_retries = 2_u32;
            let mut last_error_msg = String::new();
            let mut should_try_fallback = false;

            for attempt in 0..=max_retries {
                match self.execute_once(&primary_cfg, &req).await {
                    Ok(res) => return Ok(res),
                    Err((status_code, raw_err)) => {
                        let classified =
                            classify_provider_error(status_code, &raw_err, attempt, max_retries);
                        last_error_msg = match status_code {
                            Some(code) => {
                                format!("provider HTTP {code}: {}", classified.sanitized_message)
                            }
                            None => scrub_sensitive_tokens(&raw_err),
                        };
                        match classified.class {
                            ProviderErrorClass::NonRetryableClient => {
                                return Err(BoxerError::ExecutionFailed(last_error_msg));
                            }
                            ProviderErrorClass::TerminalQuotaExhausted => {
                                should_try_fallback = true;
                                break;
                            }
                            ProviderErrorClass::RetryableTransient => {
                                if let Some(backoff_ms) = classified.recommended_backoff_ms {
                                    tokio::time::sleep(std::time::Duration::from_millis(
                                        backoff_ms.min(50),
                                    ))
                                    .await;
                                } else {
                                    should_try_fallback = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if should_try_fallback
                && let Some(fb_id) = &primary_cfg.fallback_endpoint_id
                && let Some(fb_cfg) = self.registry.resolve(fb_id).cloned()
                && let Ok(res) = self.execute_once(&fb_cfg, &req).await
            {
                return Ok(res);
            }

            Err(BoxerError::ExecutionFailed(last_error_msg))
        })
    }
}

/// Deterministic scripted model client recording exact prompts received in tests.
#[derive(Default)]
pub struct ScriptedModelClient {
    responses: Mutex<Vec<Result<ModelTurnResponse, BoxerError>>>,
    seen_requests: Mutex<Vec<ModelTurnRequest>>,
}

impl ScriptedModelClient {
    #[must_use]
    pub fn new(mut responses: Vec<Result<ModelTurnResponse, BoxerError>>) -> Arc<Self> {
        responses.reverse();
        Arc::new(Self {
            responses: Mutex::new(responses),
            seen_requests: Mutex::new(Vec::new()),
        })
    }

    pub async fn seen_requests(&self) -> Vec<ModelTurnRequest> {
        self.seen_requests.lock().await.clone()
    }
}

impl ModelClient for ScriptedModelClient {
    fn complete_turn<'a>(
        &'a self,
        req: ModelTurnRequest,
    ) -> BoxFuture<'a, Result<ModelTurnResponse, BoxerError>> {
        Box::pin(async move {
            self.seen_requests.lock().await.push(req);
            self.responses.lock().await.pop().unwrap_or_else(|| {
                Ok(ModelTurnResponse {
                    assistant_text: "done".to_owned(),
                    tool_calls: Vec::new(),
                    stop_reason: StopReason::EndTurn,
                    usage_tokens: 1,
                })
            })
        })
    }
}
