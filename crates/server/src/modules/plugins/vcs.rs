//! Secure VCS (Git / GitHub) proxy and PR inspection for autonomous agents.
//!
//! Enables agents to inspect pull requests, read diffs, and audit CI check runs
//! without leaking personal access tokens or write credentials into agent sandboxes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub head_sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRunStatus {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummaryReport {
    pub pr_number: u64,
    pub technical_notes: Vec<String>,
    pub suggested_verifications: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsError {
    RateLimited(u64),
    NotFound(String),
    NetworkError(String),
    Unauthorized,
}

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited(retry) => write!(f, "VCS rate limit reached. Retry after {retry}s"),
            Self::NotFound(msg) => write!(f, "VCS resource not found: {msg}"),
            Self::NetworkError(msg) => write!(f, "VCS network error: {msg}"),
            Self::Unauthorized => write!(f, "VCS unauthorized"),
        }
    }
}

impl std::error::Error for VcsError {}

#[derive(Debug, Clone, Default)]
pub struct MockVcsState {
    pub prs: Vec<PullRequestSummary>,
    pub diffs: HashMap<u64, String>,
    pub checks: HashMap<String, Vec<CheckRunStatus>>,
    pub is_rate_limited: bool,
    pub rate_limit_reset_secs: u64,
}

#[derive(Debug, Clone)]
pub struct VcsProxy {
    // Encrypted/secure token lives on the server orchestrator; never exposed to Boxer
    server_token: String,
    http: reqwest::Client,
    mock_state: Arc<RwLock<MockVcsState>>,
}

impl VcsProxy {
    #[must_use]
    pub fn new(server_token: impl Into<String>) -> Self {
        Self {
            server_token: server_token.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            mock_state: Arc::new(RwLock::new(MockVcsState::default())),
        }
    }

    pub async fn set_mock_state(&self, state: MockVcsState) {
        let mut lock = self.mock_state.write().await;
        *lock = state;
    }

    pub async fn list_pull_requests(
        &self,
        repo: &str,
    ) -> Result<Vec<PullRequestSummary>, VcsError> {
        let state = self.mock_state.read().await;
        if state.is_rate_limited {
            return Err(VcsError::RateLimited(state.rate_limit_reset_secs));
        }
        if !state.prs.is_empty() || self.server_token.is_empty() {
            return Ok(state.prs.clone());
        }
        drop(state);

        let url = format!("https://api.github.com/repos/{repo}/pulls");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.server_token))
            .header("User-Agent", "sovereign-agents-server")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| VcsError::NetworkError(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(VcsError::RateLimited(60));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(VcsError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(VcsError::NotFound(repo.to_owned()));
        }
        if !status.is_success() {
            return Err(VcsError::NetworkError(format!(
                "status {}",
                status.as_u16()
            )));
        }

        let items: Vec<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| VcsError::NetworkError(e.to_string()))?;

        let prs = items
            .into_iter()
            .filter_map(|item| {
                let number = item.get("number")?.as_u64()?;
                let title = item.get("title")?.as_str()?.to_owned();
                let author = item.pointer("/user/login")?.as_str()?.to_owned();
                let state = item.get("state")?.as_str()?.to_owned();
                let head_sha = item.pointer("/head/sha")?.as_str()?.to_owned();
                Some(PullRequestSummary {
                    number,
                    title,
                    author,
                    state,
                    head_sha,
                })
            })
            .collect();

        Ok(prs)
    }

    pub async fn get_pr_diff(&self, repo: &str, pr_number: u64) -> Result<String, VcsError> {
        let state = self.mock_state.read().await;
        if state.is_rate_limited {
            return Err(VcsError::RateLimited(state.rate_limit_reset_secs));
        }
        if let Some(diff) = state.diffs.get(&pr_number) {
            return Ok(diff.clone());
        }
        if self.server_token.is_empty() {
            return Err(VcsError::NotFound(format!("PR #{pr_number}")));
        }
        drop(state);

        let url = format!("https://api.github.com/repos/{repo}/pulls/{pr_number}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.server_token))
            .header("User-Agent", "sovereign-agents-server")
            .header("Accept", "application/vnd.github.v3.diff")
            .send()
            .await
            .map_err(|e| VcsError::NetworkError(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(VcsError::RateLimited(60));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(VcsError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(VcsError::NotFound(format!("PR #{pr_number}")));
        }
        if !status.is_success() {
            return Err(VcsError::NetworkError(format!(
                "status {}",
                status.as_u16()
            )));
        }

        resp.text()
            .await
            .map_err(|e| VcsError::NetworkError(e.to_string()))
    }

    pub async fn get_check_runs(
        &self,
        repo: &str,
        commit_sha: &str,
    ) -> Result<Vec<CheckRunStatus>, VcsError> {
        let state = self.mock_state.read().await;
        if state.is_rate_limited {
            return Err(VcsError::RateLimited(state.rate_limit_reset_secs));
        }
        if let Some(runs) = state.checks.get(commit_sha) {
            return Ok(runs.clone());
        }
        if self.server_token.is_empty() {
            return Ok(Vec::new());
        }
        drop(state);

        let url = format!("https://api.github.com/repos/{repo}/commits/{commit_sha}/check-runs");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.server_token))
            .header("User-Agent", "sovereign-agents-server")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| VcsError::NetworkError(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(VcsError::RateLimited(60));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(VcsError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(VcsError::NotFound(commit_sha.to_owned()));
        }
        if !status.is_success() {
            return Err(VcsError::NetworkError(format!(
                "status {}",
                status.as_u16()
            )));
        }

        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| VcsError::NetworkError(e.to_string()))?;

        let runs = parsed
            .get("check_runs")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|cr| {
                        let name = cr.get("name")?.as_str()?.to_owned();
                        let status = cr.get("status")?.as_str()?.to_owned();
                        let conclusion = cr
                            .get("conclusion")
                            .and_then(serde_json::Value::as_str)
                            .map(ToOwned::to_owned);
                        Some(CheckRunStatus {
                            name,
                            status,
                            conclusion,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(runs)
    }

    pub fn generate_review_summary(&self, pr_number: u64, diff: &str) -> ReviewSummaryReport {
        let mut technical_notes = Vec::new();
        let mut suggested_verifications = Vec::new();

        let added_lines = diff
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        let deleted_lines = diff
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();

        technical_notes.push(format!(
            "Diff size: +{added_lines} / -{deleted_lines} lines."
        ));

        if diff.contains("fn ") || diff.contains("pub fn ") {
            suggested_verifications
                .push("Run workspace unit tests and verify signature contracts.".to_string());
        }
        if diff.contains("Cargo.toml") || diff.contains("package.json") {
            suggested_verifications
                .push("Audit third-party dependencies and verify notices.".to_string());
        }

        ReviewSummaryReport {
            pr_number,
            technical_notes,
            suggested_verifications,
        }
    }
}
