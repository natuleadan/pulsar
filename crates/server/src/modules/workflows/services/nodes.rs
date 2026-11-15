//! Execution dispatcher for distinct workflow node kinds.

use agents_protocol::{ExecOptions, InteractionMode, TaskRequest, WorkflowNode};
use serde_json::{Value, json};

use crate::{
    infra::{error::ServerError, stream::BusClient},
    modules::workflows::entities::WorkflowRunEntity,
};

/// Dispatches execution of a single DAG node based on its kind.
///
/// # Errors
///
/// Returns [`ServerError`] if bus dispatch or underlying client setup fails.
pub async fn execute_single_node(
    bus: &BusClient,
    run: &WorkflowRunEntity,
    node: &WorkflowNode,
    outputs: &serde_json::Map<String, Value>,
) -> Result<Value, ServerError> {
    match node.kind.as_str() {
        "http_request" => {
            let url = node
                .config
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("https://natuleadan.com");
            let method = node
                .config
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| ServerError::Internal(format!("http client: {e}")))?;
            let install_id = std::env::var("AGENTS_INSTALLATION_ID")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "local-dev".to_owned());
            let mut req_builder = match method.to_uppercase().as_str() {
                "POST" => {
                    let body = node.config.get("body").unwrap_or(&json!({})).clone();
                    client.post(url).json(&body)
                }
                "PUT" => {
                    let body = node.config.get("body").unwrap_or(&json!({})).clone();
                    client.put(url).json(&body)
                }
                "DELETE" => client.delete(url),
                _ => client.get(url),
            };
            req_builder = req_builder
                .header("X-Agents-Installation-Id", install_id)
                .header("X-Agents-Workflow-Id", &run.workflow_id)
                .header("X-Agents-Workflow-Run-Id", &run.run_id);
            if let Ok(org) = std::env::var("AGENTS_ORG_NAME")
                && !org.trim().is_empty()
            {
                req_builder = req_builder.header("X-Agents-Org", org.trim());
            }
            let start = std::time::Instant::now();
            let resp = req_builder.send().await;
            let latency_ms = start.elapsed().as_millis() as u64;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let success = r.status().is_success();
                    let text = r.text().await.unwrap_or_default();
                    Ok(json!({
                        "status": status,
                        "latencyMs": latency_ms,
                        "success": success,
                        "body": text,
                    }))
                }
                Err(e) => Ok(json!({
                    "status": 0,
                    "latencyMs": latency_ms,
                    "success": false,
                    "error": e.to_string(),
                })),
            }
        }
        "agent_step" => {
            let prompt = node
                .config
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("execute workflow step")
                .to_owned();
            let req = TaskRequest {
                task_id: format!("{}-{}", run.run_id, node.node_id),
                session_id: format!("wf-{}", run.workflow_id),
                prompt,
                timeout_seconds: Some(120),
                mode: InteractionMode::Act,
                fencing_token: None,
                idempotency_key: Some(format!("wf:{}:{}", run.run_id, node.node_id)),
                sandbox: None,
                exec: ExecOptions::default(),
                inference: None,
                proxy_credentials: Vec::new(),
            };
            bus.0.publish_task_request(&req).await?;
            Ok(json!({ "dispatched": true, "taskId": req.task_id }))
        }
        "render_html" | "html_output" => {
            let html = node
                .config
                .get("html")
                .and_then(Value::as_str)
                .unwrap_or("<p>Default render output</p>");
            Ok(json!({
                "html": html,
                "contentType": "text/html; charset=utf-8",
            }))
        }
        "condition" => {
            let result = evaluate_condition(&node.config, outputs);
            Ok(json!({ "branch": result }))
        }
        _ => Ok(json!({ "ok": true, "kind": node.kind })),
    }
}

fn lookup_output_path<'a>(
    outputs: &'a serde_json::Map<String, Value>,
    path: &str,
) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = outputs.get(first)?;
    for segment in parts {
        current = current.get(segment)?;
    }
    Some(current)
}

fn value_is_truthy(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty() && s != "false" && s != "0",
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn evaluate_condition(config: &Value, outputs: &serde_json::Map<String, Value>) -> bool {
    if let Some(source_node) = config.get("sourceNode").and_then(Value::as_str) {
        let field = config.get("field").and_then(Value::as_str).unwrap_or("");
        let full_path = if field.is_empty() {
            source_node.to_owned()
        } else {
            format!("{source_node}.{field}")
        };
        let actual = lookup_output_path(outputs, &full_path);
        let op = config
            .get("operator")
            .and_then(Value::as_str)
            .unwrap_or("eq");
        let expected = config.get("expected");
        return match (op, actual, expected) {
            ("truthy", Some(v), _) => value_is_truthy(v),
            ("eq", Some(a), Some(e)) => a == e,
            ("ne", Some(a), Some(e)) => a != e,
            ("contains", Some(Value::String(hay)), Some(Value::String(needle))) => {
                hay.contains(needle.as_str())
            }
            ("gt", Some(a), Some(e)) => a.as_f64().zip(e.as_f64()).is_some_and(|(x, y)| x > y),
            ("lt", Some(a), Some(e)) => a.as_f64().zip(e.as_f64()).is_some_and(|(x, y)| x < y),
            _ => false,
        };
    }

    if let Some(expr) = config.get("expression").and_then(Value::as_str) {
        if let Some((lhs, rhs)) = expr.split_once("==") {
            let actual = lookup_output_path(outputs, lhs.trim());
            let expected_raw = rhs.trim().trim_matches('"').trim_matches('\'');
            return actual.is_some_and(|v| match v {
                Value::String(s) => s == expected_raw,
                Value::Bool(b) => b.to_string() == expected_raw,
                Value::Number(n) => n.to_string() == expected_raw,
                _ => serde_json::to_string(v).ok().as_deref() == Some(expected_raw),
            });
        }
        return lookup_output_path(outputs, expr.trim()).is_some_and(value_is_truthy);
    }

    config
        .get("branch")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_evaluates_source_node_and_expression() {
        let mut outputs = serde_json::Map::new();
        outputs.insert(
            "fetch".into(),
            json!({ "status": 200, "success": true, "body": { "state": "ok" } }),
        );
        assert!(evaluate_condition(
            &json!({ "sourceNode": "fetch", "field": "status", "operator": "eq", "expected": 200 }),
            &outputs
        ));
        assert!(!evaluate_condition(
            &json!({ "sourceNode": "fetch", "field": "status", "operator": "eq", "expected": 500 }),
            &outputs
        ));
        assert!(evaluate_condition(
            &json!({ "expression": "fetch.body.state == ok" }),
            &outputs
        ));
    }
}
