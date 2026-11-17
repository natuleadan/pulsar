//! OpenJSON / OpenAPI to MCP Transpiler
//!
//! Ingests arbitrary OpenAPI 3.0/3.1 or Swagger specifications and dynamically
//! synthesizes valid Model Context Protocol tool manifests.

use pulsar_protocol::PluginToolDefinition;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum TranspileError {
    #[error("invalid json/yaml openapi specification: {0}")]
    InvalidSpec(String),
    #[error("missing required openapi paths object")]
    MissingPaths,
    #[error("no valid endpoints could be extracted from specification")]
    NoEndpointsFound,
}

pub struct OpenJsonTranspiler;

impl OpenJsonTranspiler {
    /// Ingests an OpenAPI JSON or YAML string and returns a vector of compliant MCP tools.
    ///
    /// # Errors
    ///
    /// Returns [`TranspileError`] if spec is unparsable or contains no operations.
    pub fn transpile(raw_spec: &str) -> Result<Vec<PluginToolDefinition>, TranspileError> {
        let value: Value = serde_json::from_str(raw_spec)
            .map_err(|e| TranspileError::InvalidSpec(e.to_string()))?;

        let paths = value
            .get("paths")
            .and_then(Value::as_object)
            .ok_or(TranspileError::MissingPaths)?;

        let mut tools = Vec::new();

        for (path_str, methods_val) in paths {
            let Some(methods_obj) = methods_val.as_object() else {
                continue;
            };

            for (method_str, op_val) in methods_obj {
                let method_lower = method_str.to_ascii_lowercase();
                if !["get", "post", "put", "patch", "delete"].contains(&method_lower.as_str()) {
                    continue;
                }

                let op_obj = match op_val.as_object() {
                    Some(o) => o,
                    None => continue,
                };

                let tool_name = op_obj
                    .get("operationId")
                    .and_then(Value::as_str)
                    .map_or_else(
                        || Self::generate_tool_name(&method_lower, path_str),
                        ToOwned::to_owned,
                    );

                let description = op_obj
                    .get("description")
                    .or_else(|| op_obj.get("summary"))
                    .and_then(Value::as_str)
                    .unwrap_or("Generated MCP tool for OpenAPI endpoint")
                    .to_owned();

                let parameters = Self::extract_parameters(op_obj, &value);

                tools.push(PluginToolDefinition {
                    name: tool_name,
                    description,
                    parameters,
                    enabled: true,
                    path: path_str.clone(),
                    method: method_lower.to_ascii_uppercase(),
                });
            }
        }

        if tools.is_empty() {
            return Err(TranspileError::NoEndpointsFound);
        }

        Ok(tools)
    }

    fn generate_tool_name(method: &str, path: &str) -> String {
        let clean = path
            .replace(['/', '{', '}', '-', '.'], "_")
            .trim_matches('_')
            .to_string();
        format!("{method}_{clean}")
    }

    fn extract_parameters(op_obj: &serde_json::Map<String, Value>, root: &Value) -> Value {
        let mut properties = BTreeMap::new();
        let mut required = Vec::new();

        if let Some(params) = op_obj.get("parameters").and_then(Value::as_array) {
            for param in params {
                let param_obj = match param.as_object() {
                    Some(p) => p,
                    None => continue,
                };

                let name = match param_obj.get("name").and_then(Value::as_str) {
                    Some(n) => n,
                    None => continue,
                };

                let is_req = param_obj
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_req {
                    required.push(Value::String(name.to_owned()));
                }

                let schema = param_obj
                    .get("schema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "string"}));

                let resolved = Self::resolve_schema_ref(&schema, root);
                properties.insert(name.to_owned(), resolved);
            }
        }

        if let Some(req_body) = op_obj.get("requestBody").and_then(Value::as_object)
            && let Some(content) = req_body.get("content").and_then(Value::as_object)
            && let Some(app_json) = content.get("application/json").and_then(Value::as_object)
            && let Some(schema) = app_json.get("schema")
        {
            let resolved = Self::resolve_schema_ref(schema, root);
            if let Some(body_props) = resolved.get("properties").and_then(Value::as_object) {
                for (k, v) in body_props {
                    properties.insert(k.clone(), v.clone());
                }
            }
            if let Some(body_req) = resolved.get("required").and_then(Value::as_array) {
                for req_item in body_req {
                    if !required.contains(req_item) {
                        required.push(req_item.clone());
                    }
                }
            }
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    fn resolve_schema_ref(schema: &Value, root: &Value) -> Value {
        if let Some(ref_str) = schema.get("$ref").and_then(Value::as_str)
            && let Some(stripped) = ref_str.strip_prefix("#/")
        {
            let parts: Vec<&str> = stripped.split('/').collect();
            let mut current = root;
            for part in parts {
                if let Some(next) = current.get(part) {
                    current = next;
                } else {
                    return schema.clone();
                }
            }
            return current.clone();
        }
        schema.clone()
    }
}
