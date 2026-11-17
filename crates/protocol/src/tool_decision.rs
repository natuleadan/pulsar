use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolDecisionKind {
    Allow,
    Ask,
    Cancel,
    Deny,
}

impl ToolDecisionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Cancel => "cancel",
            Self::Deny => "deny",
        }
    }

    #[must_use]
    pub const fn severity_rank(self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Ask => 1,
            Self::Cancel => 2,
            Self::Deny => 3,
        }
    }

    #[must_use]
    pub const fn combine_monotonic(self, next: Self) -> Self {
        if self.severity_rank() >= next.severity_rank() {
            self
        } else {
            next
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyRule {
    pub rule_id: String,
    pub tool_pattern: String,
    pub decision: ToolDecisionKind,
    pub reason: String,
}

impl ToolPolicyRule {
    #[must_use]
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        let pat = self.tool_pattern.trim();
        if pat == "*" {
            return true;
        }
        if let Some(prefix) = pat.strip_suffix('*') {
            return tool_name.starts_with(prefix);
        }
        pat.eq_ignore_ascii_case(tool_name.trim())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDecisionVerdict {
    pub tool_name: String,
    pub decision: ToolDecisionKind,
    pub winning_rule_id: String,
    pub reason: String,
    pub evaluated_rules: usize,
}

#[must_use]
pub fn evaluate_monotonic_tool_decision(
    tool_name: &str,
    rules: &[ToolPolicyRule],
    default_decision: ToolDecisionKind,
) -> ToolDecisionVerdict {
    let mut current = default_decision;
    let mut winning_rule_id = "default_policy".to_owned();
    let mut reason = format!("default policy `{}` applied", default_decision.as_str());
    let mut matched_any = false;
    let mut evaluated = 0;

    for rule in rules {
        if !rule.matches_tool(tool_name) {
            continue;
        }
        evaluated += 1;
        if !matched_any {
            current = rule.decision;
            winning_rule_id = rule.rule_id.clone();
            reason = rule.reason.clone();
            matched_any = true;
        } else {
            let next = current.combine_monotonic(rule.decision);
            if next.severity_rank() > current.severity_rank() {
                current = next;
                winning_rule_id = rule.rule_id.clone();
                reason = rule.reason.clone();
            }
        }
        if current == ToolDecisionKind::Deny {
            break;
        }
    }

    ToolDecisionVerdict {
        tool_name: tool_name.to_owned(),
        decision: current,
        winning_rule_id,
        reason,
        evaluated_rules: evaluated,
    }
}

const FORBIDDEN_HOST_ENV_PREFIXES: &[&str] = &[
    "PULSAR_JWT_SECRET",
    "PULSAR_CREDENTIAL_MASTER_KEY",
    "PULSAR_CLUSTER_SECRET",
    "BOXER_CLUSTER_SECRET",
    "DATABASE_URL",
    "REDIS_URL",
    "VALKEY_URL",
    "NATS_URL",
    "QSTASH_",
    "UPSTASH_",
    "S3_SECRET_KEY",
    "RUSTFS_SECRET_KEY",
    "AWS_SECRET_ACCESS_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "DEEPSEEK_API_KEY",
    "DEEPINFRA_TOKEN",
    "AI_GATEWAY_API_KEY",
];

#[must_use]
pub fn build_hygienic_tool_env(
    host_env: &BTreeMap<String, String>,
    allowlist: &[String],
    tool_overrides: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let allowed_set: BTreeSet<String> = allowlist
        .iter()
        .map(|k| k.trim().to_ascii_uppercase())
        .collect();
    let mut clean = BTreeMap::new();
    clean.insert("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned());
    clean.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    clean.insert("NLA_SANDBOXED".to_owned(), "1".to_owned());

    for (k, v) in host_env {
        let upper = k.trim().to_ascii_uppercase();
        if is_forbidden_secret_env(&upper) {
            continue;
        }
        if allowed_set.contains(&upper) {
            clean.insert(k.clone(), v.clone());
        }
    }

    for (k, v) in tool_overrides {
        let upper = k.trim().to_ascii_uppercase();
        if !is_forbidden_secret_env(&upper) {
            clean.insert(k.clone(), v.clone());
        }
    }

    clean
}

#[must_use]
pub fn is_forbidden_secret_env(upper_key: &str) -> bool {
    FORBIDDEN_HOST_ENV_PREFIXES
        .iter()
        .any(|prefix| upper_key == *prefix || upper_key.starts_with(prefix))
}
