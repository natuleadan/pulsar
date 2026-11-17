use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use pulsar_protocol::EventEnvelope;

use crate::core::ClientError;
use crate::core::adapters::ServerModeProvider;
use crate::core::backend::{HttpMethod, HttpRequest};

/// Summary of a connected boxer node returned by `GET /api/v1/nodes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRecord {
    #[serde(alias = "nodeId", alias = "id")]
    pub node_id: String,
    #[serde(default, alias = "vlanIp", alias = "hostname")]
    pub vlan_ip: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, alias = "activeBoxes", alias = "active_tasks")]
    pub active_boxes: u32,
    #[serde(default, alias = "maxConcurrentBoxes", alias = "max_tasks")]
    pub max_boxes: u32,
}

/// Summary of a task record returned by `GET /api/v1/sessions/:id/tasks`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: String,
    pub session_id: String,
    pub status: String,
    #[serde(default)]
    pub prompt: String,
}

/// API key metadata returned by `GET /api/v1/auth/api-keys`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub created_at: String,
}

/// Workflow summary returned by `GET /api/v1/workflows`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub approved: bool,
}

/// Autocron schedule record returned by `GET /api/v1/autocron/schedules`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutocronScheduleRecord {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub enabled: bool,
}

impl ServerModeProvider {
    pub(super) async fn request_json(
        &self,
        method: HttpMethod,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        body: Option<Value>,
    ) -> Result<Value, ClientError> {
        let req = HttpRequest {
            method,
            url: format!("{}{}", self.base_url, path),
            headers: Self::json_headers(token, idempotency_key),
            body: body.map(|v| v.to_string()),
        };
        let raw = Self::ensure_ok(self.transport.execute(req).await?)?;
        if raw.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&raw)?)
    }

    /// Returns the SSE stream URL for a session.
    #[must_use]
    pub fn session_sse_url(&self, session_id: &str) -> String {
        format!(
            "{}/api/v1/sessions/{session_id}/events/stream",
            self.base_url
        )
    }

    /// Returns the WebSocket stream URL for a session.
    #[must_use]
    pub fn session_ws_url(&self, session_id: &str) -> String {
        let ws_base = if let Some(rest) = self.base_url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base_url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            self.base_url.clone()
        };
        format!("{ws_base}/api/v1/sessions/{session_id}/ws")
    }

    /// Parses raw SSE text frames into `EventEnvelope` items.
    #[must_use]
    pub fn parse_sse_events(raw: &str) -> Vec<EventEnvelope> {
        let mut events = Vec::new();
        for block in raw.split("\n\n") {
            let mut data_lines = Vec::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim());
                }
            }
            if data_lines.is_empty() {
                continue;
            }
            let joined = data_lines.join("\n");
            if let Ok(env) = serde_json::from_str::<EventEnvelope>(&joined) {
                events.push(env);
            }
        }
        events
    }

    /// Lists boxer nodes via `GET /api/v1/nodes`.
    pub async fn list_nodes(&self, token: &str) -> Result<Vec<NodeRecord>, ClientError> {
        let val = self
            .request_json(HttpMethod::Get, "/api/v1/nodes", Some(token), None, None)
            .await?;
        let items = val.get("nodes").cloned().unwrap_or(val);
        Ok(serde_json::from_value(items)?)
    }

    /// Creates preview route allocations via `POST /api/v1/nodes/previews`.
    pub async fn create_node_previews(
        &self,
        token: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            "/api/v1/nodes/previews",
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Lists sessions via `GET /api/v1/sessions`.
    pub async fn list_sessions(&self, token: &str) -> Result<Vec<Value>, ClientError> {
        let val = self
            .request_json(HttpMethod::Get, "/api/v1/sessions", Some(token), None, None)
            .await?;
        let items = val.get("sessions").cloned().unwrap_or(val);
        Ok(serde_json::from_value(items)?)
    }

    /// Creates a new session via `POST /api/v1/sessions`.
    pub async fn create_session(
        &self,
        token: &str,
        title: &str,
        mode: &str,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            "/api/v1/sessions",
            Some(token),
            None,
            Some(json!({ "title": title, "mode": mode })),
        )
        .await
    }

    /// Fetches a single session via `GET /api/v1/sessions/:id`.
    pub async fn get_session(&self, token: &str, session_id: &str) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Get,
            &format!("/api/v1/sessions/{session_id}"),
            Some(token),
            None,
            None,
        )
        .await
    }

    /// Updates session metadata via `PATCH /api/v1/sessions/:id`.
    pub async fn update_session(
        &self,
        token: &str,
        session_id: &str,
        patch: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Patch,
            &format!("/api/v1/sessions/{session_id}"),
            Some(token),
            None,
            Some(patch),
        )
        .await
    }

    /// Deletes a session via `DELETE /api/v1/sessions/:id`.
    pub async fn delete_session(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Delete,
            &format!("/api/v1/sessions/{session_id}"),
            Some(token),
            None,
            None,
        )
        .await
    }

    /// Lists tasks for a session via `GET /api/v1/sessions/:id/tasks`.
    pub async fn list_tasks(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<Vec<TaskRecord>, ClientError> {
        let val = self
            .request_json(
                HttpMethod::Get,
                &format!("/api/v1/sessions/{session_id}/tasks"),
                Some(token),
                None,
                None,
            )
            .await?;
        let items = val.get("tasks").cloned().unwrap_or(val);
        Ok(serde_json::from_value(items)?)
    }

    /// Appends a follow-up prompt via `POST /api/v1/sessions/:id/follow-ups`.
    pub async fn send_follow_up(
        &self,
        token: &str,
        session_id: &str,
        prompt: &str,
        idempotency_key: &str,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/sessions/{session_id}/follow-ups"),
            Some(token),
            Some(idempotency_key),
            Some(json!({ "prompt": prompt })),
        )
        .await
    }

    /// Updates a task state via `POST /api/v1/tasks/:id/state`.
    pub async fn update_task_state(
        &self,
        token: &str,
        task_id: &str,
        state: &str,
        idempotency_key: &str,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/tasks/{task_id}/state"),
            Some(token),
            Some(idempotency_key),
            Some(json!({ "state": state })),
        )
        .await
    }

    /// Creates a swarm handoff via `POST /api/v1/sessions/:id/swarm/handoffs`.
    pub async fn create_swarm_handoff(
        &self,
        token: &str,
        session_id: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Posts a swarm message via `POST /api/v1/sessions/:id/swarm/messages`.
    pub async fn post_swarm_message(
        &self,
        token: &str,
        session_id: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/sessions/{session_id}/swarm/messages"),
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Lists swarm messages via `GET /api/v1/sessions/:id/swarm/messages`.
    pub async fn list_swarm_messages(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Get,
            &format!("/api/v1/sessions/{session_id}/swarm/messages"),
            Some(token),
            None,
            None,
        )
        .await
    }

    /// Updates a swarm blackboard key via `POST /api/v1/sessions/:id/swarm/blackboard`.
    pub async fn put_swarm_blackboard(
        &self,
        token: &str,
        session_id: &str,
        key: &str,
        value: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/sessions/{session_id}/swarm/blackboard"),
            Some(token),
            None,
            Some(json!({ "key": key, "value": value })),
        )
        .await
    }
}
