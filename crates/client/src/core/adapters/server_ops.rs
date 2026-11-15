use serde_json::{Value, json};

use crate::core::ClientError;
use crate::core::adapters::ServerModeProvider;
use crate::core::adapters::server_api::{ApiKeyRecord, AutocronScheduleRecord, WorkflowRecord};
use crate::core::backend::HttpMethod;

impl ServerModeProvider {
    /// Lists workflows via `GET /api/v1/workflows`.
    pub async fn list_workflows(&self, token: &str) -> Result<Vec<WorkflowRecord>, ClientError> {
        let val = self
            .request_json(
                HttpMethod::Get,
                "/api/v1/workflows",
                Some(token),
                None,
                None,
            )
            .await?;
        let items = val.get("workflows").cloned().unwrap_or(val);
        Ok(serde_json::from_value(items)?)
    }

    /// Creates a workflow via `POST /api/v1/workflows`.
    pub async fn create_workflow(&self, token: &str, payload: Value) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            "/api/v1/workflows",
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Approves a workflow via `POST /api/v1/workflows/:id/approve`.
    pub async fn approve_workflow(
        &self,
        token: &str,
        workflow_id: &str,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/workflows/{workflow_id}/approve"),
            Some(token),
            None,
            Some(json!({})),
        )
        .await
    }

    /// Triggers a workflow via `POST /api/v1/workflows/:id/trigger`.
    pub async fn trigger_workflow(
        &self,
        token: &str,
        workflow_id: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/workflows/{workflow_id}/trigger"),
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Lists autocron schedules via `GET /api/v1/autocron/schedules`.
    pub async fn list_autocron_schedules(
        &self,
        token: &str,
    ) -> Result<Vec<AutocronScheduleRecord>, ClientError> {
        let val = self
            .request_json(
                HttpMethod::Get,
                "/api/v1/autocron/schedules",
                Some(token),
                None,
                None,
            )
            .await?;
        let items = val.get("schedules").cloned().unwrap_or(val);
        Ok(serde_json::from_value(items)?)
    }

    /// Creates an autocron schedule via `POST /api/v1/autocron/schedules`.
    pub async fn create_autocron_schedule(
        &self,
        token: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            "/api/v1/autocron/schedules",
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Toggles an autocron schedule via `POST /api/v1/autocron/schedules/:id/toggle`.
    pub async fn toggle_autocron_schedule(
        &self,
        token: &str,
        schedule_id: &str,
        enabled: bool,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            &format!("/api/v1/autocron/schedules/{schedule_id}/toggle"),
            Some(token),
            None,
            Some(json!({ "enabled": enabled })),
        )
        .await
    }

    /// Lists API keys via `GET /api/v1/auth/api-keys`.
    pub async fn list_api_keys(&self, token: &str) -> Result<Vec<ApiKeyRecord>, ClientError> {
        let val = self
            .request_json(
                HttpMethod::Get,
                "/api/v1/auth/api-keys",
                Some(token),
                None,
                None,
            )
            .await?;
        let items = val.get("api_keys").cloned().unwrap_or(val);
        Ok(serde_json::from_value(items)?)
    }

    /// Creates an API key via `POST /api/v1/auth/api-keys`.
    pub async fn create_api_key(&self, token: &str, name: &str) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            "/api/v1/auth/api-keys",
            Some(token),
            None,
            Some(json!({ "name": name })),
        )
        .await
    }

    /// Revokes an API key via `DELETE /api/v1/auth/api-keys/:id`.
    pub async fn revoke_api_key(&self, token: &str, key_id: &str) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Delete,
            &format!("/api/v1/auth/api-keys/{key_id}"),
            Some(token),
            None,
            None,
        )
        .await
    }

    /// Fetches `GET /.well-known/agent-card.json`.
    pub async fn get_agent_card(&self) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Get,
            "/.well-known/agent-card.json",
            None,
            None,
            None,
        )
        .await
    }

    /// Sends an outbound A2A message via `POST /api/v1/a2a/send`.
    pub async fn send_a2a_message(
        &self,
        token: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Post,
            "/api/v1/a2a/send",
            Some(token),
            None,
            Some(payload),
        )
        .await
    }

    /// Lists interaction identity ledger entries via `GET /api/v1/identity/ledger`.
    pub async fn list_identity_ledger(&self, token: &str) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Get,
            "/api/v1/identity/ledger",
            Some(token),
            None,
            None,
        )
        .await
    }

    /// Fetches the MCP policy via `GET /api/v1/mcp/policy`.
    pub async fn get_mcp_policy(&self, token: &str) -> Result<Value, ClientError> {
        self.request_json(
            HttpMethod::Get,
            "/api/v1/mcp/policy",
            Some(token),
            None,
            None,
        )
        .await
    }

    /// Calls the MCP JSON-RPC endpoint via `POST /mcp`.
    pub async fn call_mcp_jsonrpc(
        &self,
        token: &str,
        payload: Value,
    ) -> Result<Value, ClientError> {
        self.request_json(HttpMethod::Post, "/mcp", Some(token), None, Some(payload))
            .await
    }
}
