use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderErrorClass {
    NonRetryableClient,
    RetryableTransient,
    TerminalQuotaExhausted,
}

impl ProviderErrorClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonRetryableClient => "non_retryable_client",
            Self::RetryableTransient => "retryable_transient",
            Self::TerminalQuotaExhausted => "terminal_quota_exhausted",
        }
    }

    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::RetryableTransient)
    }

    #[must_use]
    pub const fn triggers_fallback(self) -> bool {
        matches!(
            self,
            Self::RetryableTransient | Self::TerminalQuotaExhausted
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifiedProviderError {
    pub status_code: Option<u16>,
    pub class: ProviderErrorClass,
    pub sanitized_message: String,
    pub recommended_backoff_ms: Option<u64>,
    pub should_degrade_to_fallback: bool,
}

#[must_use]
pub fn classify_provider_error(
    status_code: Option<u16>,
    raw_body_or_error: &str,
    attempt_index: u32,
    max_retries: u32,
) -> ClassifiedProviderError {
    let sanitized = scrub_sensitive_tokens(raw_body_or_error);
    let lower = sanitized.to_ascii_lowercase();

    let is_quota_zero = lower.contains("insufficient_quota")
        || lower.contains("quota_exceeded")
        || lower.contains("billing_hard_limit_reached")
        || lower.contains("credit balance is too low")
        || status_code == Some(402);

    if is_quota_zero {
        return ClassifiedProviderError {
            status_code,
            class: ProviderErrorClass::TerminalQuotaExhausted,
            sanitized_message: sanitized,
            recommended_backoff_ms: None,
            should_degrade_to_fallback: true,
        };
    }

    let class = match status_code {
        Some(400 | 401 | 403 | 404 | 422) => ProviderErrorClass::NonRetryableClient,
        Some(408 | 429 | 499 | 500 | 502 | 503 | 504) | None => {
            ProviderErrorClass::RetryableTransient
        }
        Some(code) if code >= 500 => ProviderErrorClass::RetryableTransient,
        Some(_) => ProviderErrorClass::NonRetryableClient,
    };

    let can_retry = class == ProviderErrorClass::RetryableTransient && attempt_index < max_retries;
    let backoff = if can_retry {
        Some(compute_exponential_backoff_ms(attempt_index))
    } else {
        None
    };

    let should_degrade = match class {
        ProviderErrorClass::TerminalQuotaExhausted => true,
        ProviderErrorClass::RetryableTransient => !can_retry,
        ProviderErrorClass::NonRetryableClient => false,
    };

    ClassifiedProviderError {
        status_code,
        class,
        sanitized_message: sanitized,
        recommended_backoff_ms: backoff,
        should_degrade_to_fallback: should_degrade,
    }
}

#[must_use]
pub const fn compute_exponential_backoff_ms(attempt_index: u32) -> u64 {
    let shift = if attempt_index > 5 { 5 } else { attempt_index };
    let base = 250_u64.saturating_mul(1_u64 << shift);
    let jitter = ((attempt_index as u64).wrapping_mul(73)) % 120;
    base.saturating_add(jitter)
}

#[must_use]
pub fn scrub_sensitive_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for token in input.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        if is_secret_like_token(token) {
            out.push_str("[REDACTED_CREDENTIAL]");
        } else {
            out.push_str(token);
        }
    }
    out
}

fn is_secret_like_token(token: &str) -> bool {
    let clean = token.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    });
    let lower = clean.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ag_ak_")
        || lower.starts_with("eyj")
        || lower.starts_with("ghp_")
        || lower.starts_with("xoxb-")
        || lower.contains("authorization:")
        || lower.starts_with("bearer=")
        || lower.starts_with("api_key=")
        || lower.starts_with("apikey=")
}
