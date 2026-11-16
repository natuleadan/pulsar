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
    _server_token: String,
    mock_state: Arc<RwLock<MockVcsState>>,
}

impl VcsProxy {
    pub fn new(server_token: impl Into<String>) -> Self {
        Self {
            _server_token: server_token.into(),
            mock_state: Arc::new(RwLock::new(MockVcsState::default())),
        }
    }

    pub async fn set_mock_state(&self, state: MockVcsState) {
        let mut lock = self.mock_state.write().await;
        *lock = state;
    }

    pub async fn list_pull_requests(
        &self,
        _repo: &str,
    ) -> Result<Vec<PullRequestSummary>, VcsError> {
        let state = self.mock_state.read().await;
        if state.is_rate_limited {
            return Err(VcsError::RateLimited(state.rate_limit_reset_secs));
        }
        Ok(state.prs.clone())
    }

    pub async fn get_pr_diff(&self, _repo: &str, pr_number: u64) -> Result<String, VcsError> {
        let state = self.mock_state.read().await;
        if state.is_rate_limited {
            return Err(VcsError::RateLimited(state.rate_limit_reset_secs));
        }
        match state.diffs.get(&pr_number) {
            Some(diff) => Ok(diff.clone()),
            None => Err(VcsError::NotFound(format!("PR #{pr_number}"))),
        }
    }

    pub async fn get_check_runs(
        &self,
        _repo: &str,
        commit_sha: &str,
    ) -> Result<Vec<CheckRunStatus>, VcsError> {
        let state = self.mock_state.read().await;
        if state.is_rate_limited {
            return Err(VcsError::RateLimited(state.rate_limit_reset_secs));
        }
        Ok(state.checks.get(commit_sha).cloned().unwrap_or_default())
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
