use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProviderProtocol {
    #[default]
    OpenAiCompatible,
    AnthropicMessages,
    GeminiGenerateContent,
    Unknown(String),
}

impl ProviderProtocol {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::OpenAiCompatible => "openai_compatible",
            Self::AnthropicMessages => "anthropic_messages",
            Self::GeminiGenerateContent => "gemini_generate_content",
            Self::Unknown(raw) => raw.as_str(),
        }
    }

    #[must_use]
    pub fn from_kind(kind: &str) -> Self {
        match kind.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "anthropic_messages" | "claude" => Self::AnthropicMessages,
            "gemini" | "google" | "gemini_generate_content" => Self::GeminiGenerateContent,
            "openai" | "openai_compatible" | "deepseek" | "deepinfra" | "groq" | "together"
            | "mistral" | "ollama" | "vllm" | "openrouter" | "opencode" | "lmstudio" | "custom" => {
                Self::OpenAiCompatible
            }
            other => Self::Unknown(other.to_owned()),
        }
    }

    #[must_use]
    pub fn completion_url(&self, base_url: &str, model: &str) -> String {
        let base = base_url.trim().trim_end_matches('/');
        match self {
            Self::AnthropicMessages => {
                if base.ends_with("/v1") {
                    format!("{base}/messages")
                } else {
                    format!("{base}/v1/messages")
                }
            }
            Self::GeminiGenerateContent => {
                if base.contains("/models") {
                    format!("{base}/{model}:generateContent")
                } else {
                    format!("{base}/v1beta/models/{model}:generateContent")
                }
            }
            Self::OpenAiCompatible | Self::Unknown(_) => {
                if base.ends_with("/v1") {
                    format!("{base}/chat/completions")
                } else {
                    format!("{base}/v1/chat/completions")
                }
            }
        }
    }

    #[must_use]
    pub fn models_url(&self, base_url: &str) -> String {
        let base = base_url.trim().trim_end_matches('/');
        match self {
            Self::GeminiGenerateContent => {
                if base.contains("/v1beta") {
                    format!("{base}/models")
                } else {
                    format!("{base}/v1beta/models")
                }
            }
            Self::OpenAiCompatible | Self::AnthropicMessages | Self::Unknown(_) => {
                if base.ends_with("/openai") || base.ends_with("/v1") {
                    format!("{base}/models")
                } else {
                    format!("{base}/v1/models")
                }
            }
        }
    }
}

impl Serialize for ProviderProtocol {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderProtocol {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "openai_compatible" => Self::OpenAiCompatible,
            "anthropic_messages" => Self::AnthropicMessages,
            "gemini_generate_content" => Self::GeminiGenerateContent,
            _ => Self::from_kind(&raw),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCompletionRequest {
    pub model: String,
    pub messages: Vec<ConversationMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub stream: bool,
}

impl UniversalCompletionRequest {
    #[must_use]
    pub fn to_protocol_payload(&self, protocol: &ProviderProtocol) -> Value {
        match protocol {
            ProviderProtocol::AnthropicMessages => {
                let mut system_parts = Vec::new();
                let mut messages = Vec::new();
                for msg in &self.messages {
                    if msg.role == "system" {
                        system_parts.push(msg.content.as_str());
                    } else {
                        let role = if msg.role == "assistant" {
                            "assistant"
                        } else {
                            "user"
                        };
                        messages.push(json!({ "role": role, "content": msg.content }));
                    }
                }
                let mut body = json!({
                    "model": self.model,
                    "max_tokens": self.max_tokens.unwrap_or(4096),
                    "messages": messages,
                    "stream": self.stream,
                });
                if !system_parts.is_empty() {
                    body["system"] = Value::String(system_parts.join("\n\n"));
                }
                if let Some(temp) = self.temperature {
                    body["temperature"] = json!(temp);
                }
                if !self.tools.is_empty() {
                    body["tools"] = Value::Array(
                        self.tools
                            .iter()
                            .map(|t| {
                                json!({
                                    "name": t.name,
                                    "description": t.description,
                                    "input_schema": t.parameters,
                                })
                            })
                            .collect(),
                    );
                }
                body
            }
            ProviderProtocol::GeminiGenerateContent => {
                let contents: Vec<Value> = self
                    .messages
                    .iter()
                    .map(|m| {
                        let role = if m.role == "assistant" {
                            "model"
                        } else {
                            "user"
                        };
                        json!({
                            "role": role,
                            "parts": [{ "text": m.content }],
                        })
                    })
                    .collect();
                let mut body = json!({ "contents": contents });
                let mut gen_cfg = BTreeMap::new();
                if let Some(max_tok) = self.max_tokens {
                    gen_cfg.insert("maxOutputTokens", json!(max_tok));
                }
                if let Some(temp) = self.temperature {
                    gen_cfg.insert("temperature", json!(temp));
                }
                if !gen_cfg.is_empty() {
                    body["generationConfig"] = json!(gen_cfg);
                }
                body
            }
            ProviderProtocol::OpenAiCompatible | ProviderProtocol::Unknown(_) => {
                let messages: Vec<Value> = self
                    .messages
                    .iter()
                    .map(|m| json!({ "role": m.role, "content": m.content }))
                    .collect();
                let mut body = json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": self.stream,
                });
                if let Some(max_tok) = self.max_tokens {
                    body["max_tokens"] = json!(max_tok);
                }
                if let Some(temp) = self.temperature {
                    body["temperature"] = json!(temp);
                }
                if !self.tools.is_empty() {
                    body["tools"] = Value::Array(
                        self.tools
                            .iter()
                            .map(|t| {
                                json!({
                                    "type": "function",
                                    "function": {
                                        "name": t.name,
                                        "description": t.description,
                                        "parameters": t.parameters,
                                    }
                                })
                            })
                            .collect(),
                    );
                }
                body
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UniversalStreamChunk {
    TextDelta {
        delta: String,
    },
    ToolCallDelta {
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        arguments_delta: String,
    },
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
    },
    KeepAlive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredModel {
    pub id: String,
    pub owned_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

#[must_use]
pub fn parse_sse_line(protocol: &ProviderProtocol, line: &str) -> Option<UniversalStreamChunk> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') || trimmed.starts_with("event:") {
        return None;
    }
    let data = trimmed.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(UniversalStreamChunk::Done { stop_reason: None });
    }
    let parsed: Value = serde_json::from_str(data).ok()?;
    match protocol {
        ProviderProtocol::AnthropicMessages => {
            let ev_type = parsed.get("type").and_then(Value::as_str).unwrap_or("");
            match ev_type {
                "content_block_delta" => {
                    let delta_type = parsed
                        .pointer("/delta/type")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if delta_type == "text_delta" {
                        let text = parsed.pointer("/delta/text")?.as_str()?.to_owned();
                        Some(UniversalStreamChunk::TextDelta { delta: text })
                    } else if delta_type == "input_json_delta" {
                        let partial = parsed.pointer("/delta/partial_json")?.as_str()?.to_owned();
                        let index = parsed
                            .get("index")
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok())
                            .unwrap_or(0);
                        Some(UniversalStreamChunk::ToolCallDelta {
                            index,
                            id: None,
                            name: None,
                            arguments_delta: partial,
                        })
                    } else {
                        None
                    }
                }
                "message_stop" => Some(UniversalStreamChunk::Done {
                    stop_reason: Some("end_turn".to_owned()),
                }),
                "message_delta" => {
                    let stop = parsed
                        .pointer("/delta/stop_reason")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    if stop.is_some() {
                        Some(UniversalStreamChunk::Done { stop_reason: stop })
                    } else {
                        Some(UniversalStreamChunk::KeepAlive)
                    }
                }
                _ => Some(UniversalStreamChunk::KeepAlive),
            }
        }
        ProviderProtocol::GeminiGenerateContent => {
            if let Some(text) = parsed
                .pointer("/candidates/0/content/parts/0/text")
                .and_then(Value::as_str)
            {
                return Some(UniversalStreamChunk::TextDelta {
                    delta: text.to_owned(),
                });
            }
            None
        }
        ProviderProtocol::OpenAiCompatible | ProviderProtocol::Unknown(_) => {
            if let Some(content) = parsed
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                && !content.is_empty()
            {
                return Some(UniversalStreamChunk::TextDelta {
                    delta: content.to_owned(),
                });
            }
            if let Some(tc) = parsed.pointer("/choices/0/delta/tool_calls/0") {
                let index = tc
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0);
                let id = tc.get("id").and_then(Value::as_str).map(ToOwned::to_owned);
                let name = tc
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let args = tc
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                return Some(UniversalStreamChunk::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta: args,
                });
            }
            if let Some(finish) = parsed
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
            {
                return Some(UniversalStreamChunk::Done {
                    stop_reason: Some(finish.to_owned()),
                });
            }
            Some(UniversalStreamChunk::KeepAlive)
        }
    }
}

#[must_use]
pub fn parse_models_catalog_json(raw: &Value, default_owner: &str) -> Vec<DiscoveredModel> {
    let items = raw
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| raw.get("models").and_then(Value::as_array));
    let Some(arr) = items else {
        return Vec::new();
    };
    let mut models = Vec::with_capacity(arr.len());
    for entry in arr {
        let id = entry
            .get("id")
            .or_else(|| entry.get("name"))
            .and_then(Value::as_str)
            .map(|s| s.trim_start_matches("models/").to_owned());
        let Some(model_id) = id.filter(|s| !s.is_empty()) else {
            continue;
        };
        let owned_by = entry
            .get("owned_by")
            .and_then(Value::as_str)
            .unwrap_or(default_owner)
            .to_owned();
        let context_window = entry
            .get("context_window")
            .or_else(|| entry.get("context_length"))
            .or_else(|| entry.get("inputTokenLimit"))
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok());
        models.push(DiscoveredModel {
            id: model_id,
            owned_by,
            context_window,
        });
    }
    models
}
