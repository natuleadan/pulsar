//! Declarative, versioned model harness per provider family.
//!
//! Maximizes model accuracy by adapting prompt wrappers, tool declarations,
//! and parsing dialects (Claude XML, OpenAI JSON, DeepSeek R1 `<think>` reasoning)
//! without requiring server recompilation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelHarnessDialect {
    ClaudeXml,
    OpenAiJson,
    DeepSeekR1,
    GenericMarkdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HarnessOutput {
    pub reasoning: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Vec<HarnessToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHarness {
    pub id: String,
    pub family: String,
    pub dialect: ModelHarnessDialect,
    pub system_template: String,
    pub reasoning_open: Option<String>,
    pub reasoning_close: Option<String>,
    pub version: u32,
}

impl ModelHarness {
    pub fn default_anthropic() -> Self {
        Self {
            id: "anthropic-claude".to_string(),
            family: "anthropic".to_string(),
            dialect: ModelHarnessDialect::ClaudeXml,
            system_template: "{{base_prompt}}\n\nTools are provided in <tools> format. Use <invoke name=\"...\"><parameter>...</parameter></invoke> to call tools.".to_string(),
            reasoning_open: None,
            reasoning_close: None,
            version: 1,
        }
    }

    pub fn default_deepseek_r1() -> Self {
        Self {
            id: "deepseek-r1".to_string(),
            family: "deepseek".to_string(),
            dialect: ModelHarnessDialect::DeepSeekR1,
            system_template: "{{base_prompt}}\n\nReason within <think>...</think> before acting or generating output.".to_string(),
            reasoning_open: Some("<think>".to_string()),
            reasoning_close: Some("</think>".to_string()),
            version: 1,
        }
    }

    pub fn default_openai() -> Self {
        Self {
            id: "openai-gpt".to_string(),
            family: "openai".to_string(),
            dialect: ModelHarnessDialect::OpenAiJson,
            system_template: "{{base_prompt}}\n\nOperate with strict JSON Schema function calls."
                .to_string(),
            reasoning_open: None,
            reasoning_close: None,
            version: 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HarnessRegistry {
    harnesses: Arc<RwLock<HashMap<String, ModelHarness>>>,
}

impl HarnessRegistry {
    pub fn new() -> Self {
        let mut map = HashMap::new();
        let ant = ModelHarness::default_anthropic();
        let ds = ModelHarness::default_deepseek_r1();
        let oai = ModelHarness::default_openai();

        map.insert(ant.id.clone(), ant);
        map.insert(ds.id.clone(), ds);
        map.insert(oai.id.clone(), oai);

        Self {
            harnesses: Arc::new(RwLock::new(map)),
        }
    }

    pub async fn register(&self, harness: ModelHarness) {
        let mut lock = self.harnesses.write().await;
        lock.insert(harness.id.clone(), harness);
    }

    pub async fn get(&self, id: &str) -> Option<ModelHarness> {
        let lock = self.harnesses.read().await;
        lock.get(id).cloned()
    }

    pub async fn format_system_prompt(&self, harness_id: &str, base_prompt: &str) -> String {
        let lock = self.harnesses.read().await;
        if let Some(harness) = lock.get(harness_id) {
            harness
                .system_template
                .replace("{{base_prompt}}", base_prompt)
        } else {
            base_prompt.to_string()
        }
    }

    pub async fn parse_completion(&self, harness_id: &str, raw_output: &str) -> HarnessOutput {
        let lock = self.harnesses.read().await;
        let harness = match lock.get(harness_id) {
            Some(h) => h.clone(),
            None => {
                return HarnessOutput {
                    reasoning: None,
                    content: Some(raw_output.to_string()),
                    tool_calls: vec![],
                };
            }
        };

        match harness.dialect {
            ModelHarnessDialect::DeepSeekR1 => Self::parse_deepseek_r1(&harness, raw_output),
            ModelHarnessDialect::ClaudeXml => Self::parse_claude_xml(raw_output),
            ModelHarnessDialect::OpenAiJson => Self::parse_openai_json(raw_output),
            ModelHarnessDialect::GenericMarkdown => HarnessOutput {
                reasoning: None,
                content: Some(raw_output.to_string()),
                tool_calls: vec![],
            },
        }
    }

    fn parse_deepseek_r1(harness: &ModelHarness, raw_output: &str) -> HarnessOutput {
        let mut reasoning = None;
        let mut content_part = raw_output;

        if let (Some(open), Some(close)) = (&harness.reasoning_open, &harness.reasoning_close) {
            let start = raw_output.find(open);
            let end = raw_output.find(close);
            if let (Some(start_idx), Some(end_idx)) = (start, end)
                && start_idx < end_idx
            {
                let r = raw_output[start_idx + open.len()..end_idx].trim();
                reasoning = Some(r.to_string());
                content_part = raw_output[end_idx + close.len()..].trim();
            }
        }

        let mut output = HarnessOutput {
            reasoning,
            content: if content_part.is_empty() {
                None
            } else {
                Some(content_part.to_string())
            },
            tool_calls: vec![],
        };

        if let Some(json_start) = content_part.find("```json") {
            let after_fence = &content_part[json_start + 7..];
            if let Some(closing_fence) = after_fence.find("```") {
                let json_slice = after_fence[..closing_fence].trim();
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_slice) {
                    let tool_opt = val
                        .get("tool")
                        .or_else(|| val.get("name"))
                        .and_then(|v| v.as_str());
                    if let Some(tool_name) = tool_opt {
                        let args = val
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        output.tool_calls.push(HarnessToolCall {
                            name: tool_name.to_string(),
                            arguments: args,
                        });
                    }
                }
            }
        }

        output
    }

    fn parse_claude_xml(raw_output: &str) -> HarnessOutput {
        let mut tool_calls = vec![];
        let mut cursor = raw_output;

        while let Some(start_idx) = cursor.find("<invoke name=\"") {
            let after_name = &cursor[start_idx + 14..];
            if let Some(quote_idx) = after_name.find('"') {
                let tool_name = &after_name[..quote_idx];
                let after_tag = &after_name[quote_idx + 1..];
                if let Some(close_tag_idx) = after_tag.find("</invoke>") {
                    let inner_xml = &after_tag[..close_tag_idx];
                    let mut args_map = serde_json::Map::new();

                    let mut p_cursor = inner_xml;
                    while let Some(p_start) = p_cursor.find("<parameter name=\"") {
                        let p_after = &p_cursor[p_start + 17..];
                        if let Some(p_quote) = p_after.find('"') {
                            let p_name = &p_after[..p_quote];
                            if let Some(p_val_start) = p_after[p_quote..].find('>') {
                                let p_body = &p_after[p_quote + p_val_start + 1..];
                                if let Some(p_close) = p_body.find("</parameter>") {
                                    let p_val = &p_body[..p_close].trim();
                                    args_map.insert(
                                        p_name.to_string(),
                                        serde_json::Value::String(p_val.to_string()),
                                    );
                                    p_cursor = &p_body[p_close + 12..];
                                    continue;
                                }
                            }
                        }
                        break;
                    }

                    tool_calls.push(HarnessToolCall {
                        name: tool_name.to_string(),
                        arguments: serde_json::Value::Object(args_map),
                    });
                    cursor = &after_tag[close_tag_idx + 9..];
                    continue;
                }
            }
            break;
        }

        HarnessOutput {
            reasoning: None,
            content: Some(raw_output.to_string()),
            tool_calls,
        }
    }

    fn parse_openai_json(raw_output: &str) -> HarnessOutput {
        let mut tool_calls = vec![];
        let maybe_parsed: Result<serde_json::Value, _> = serde_json::from_str(raw_output);
        if let Ok(parsed) = maybe_parsed {
            let maybe_calls = parsed.get("tool_calls").and_then(|v| v.as_array());
            if let Some(calls) = maybe_calls {
                for call in calls {
                    let maybe_func = call.get("function");
                    let maybe_name = maybe_func
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str());
                    if let (Some(func), Some(name)) = (maybe_func, maybe_name) {
                        let arguments = if let Some(args_str) =
                            func.get("arguments").and_then(|a| a.as_str())
                        {
                            serde_json::from_str(args_str).unwrap_or(serde_json::json!({}))
                        } else {
                            func.get("arguments")
                                .cloned()
                                .unwrap_or(serde_json::json!({}))
                        };
                        tool_calls.push(HarnessToolCall {
                            name: name.to_string(),
                            arguments,
                        });
                    }
                }
            }
        }

        HarnessOutput {
            reasoning: None,
            content: Some(raw_output.to_string()),
            tool_calls,
        }
    }
}
