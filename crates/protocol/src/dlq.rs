//! Poison message protection and Dead Letter Queue (DLQ) tracking for messaging loops.

use std::{collections::HashMap, sync::Mutex};

/// Maximum delivery attempts before a malformed or crashing message is moved to DLQ.
pub const DEFAULT_MAX_DELIVERY_ATTEMPTS: u32 = 3;

/// Record of a quarantined poison message.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetterEntry {
    pub message_id: String,
    pub subject: String,
    pub payload_preview: String,
    pub error_reason: String,
    pub delivery_attempts: u32,
    pub quarantined_at_ms: u64,
}

/// Action taken after evaluating a delivery failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DlqDecision {
    /// Retry attempt under the max delivery threshold.
    Retry { attempt: u32, max_deliver: u32 },
    /// Delivery threshold exceeded (max_deliver reached); quarantined into DLQ.
    Quarantined(DeadLetterEntry),
}

/// Thread-safe poison message supervisor enforcing `max_deliver: 3` DLQ quarantine.
#[derive(Debug)]
pub struct PoisonMessageFilter {
    max_deliver: u32,
    attempts: Mutex<HashMap<String, u32>>,
    dlq: Mutex<Vec<DeadLetterEntry>>,
}

impl Default for PoisonMessageFilter {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_DELIVERY_ATTEMPTS)
    }
}

impl PoisonMessageFilter {
    #[must_use]
    pub fn new(max_deliver: u32) -> Self {
        Self {
            max_deliver: max_deliver.max(1),
            attempts: Mutex::new(HashMap::new()),
            dlq: Mutex::new(Vec::new()),
        }
    }

    /// Evaluates a message failure: retries if under threshold, or quarantines to DLQ on attempt 3.
    pub fn record_failure(
        &self,
        message_id: &str,
        subject: &str,
        raw_payload: &[u8],
        error_reason: &str,
        now_ms: u64,
    ) -> DlqDecision {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = attempts.entry(message_id.to_owned()).or_insert(0);
        *count = count.saturating_add(1);
        let current_attempt = *count;

        if current_attempt >= self.max_deliver {
            attempts.remove(message_id);
            let preview = if let Ok(s) = std::str::from_utf8(raw_payload) {
                s.chars().take(200).collect()
            } else {
                format!("<binary data: {} bytes>", raw_payload.len())
            };

            let entry = DeadLetterEntry {
                message_id: message_id.to_owned(),
                subject: subject.to_owned(),
                payload_preview: preview,
                error_reason: error_reason.to_owned(),
                delivery_attempts: current_attempt,
                quarantined_at_ms: now_ms,
            };

            self.dlq
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(entry.clone());
            DlqDecision::Quarantined(entry)
        } else {
            DlqDecision::Retry {
                attempt: current_attempt,
                max_deliver: self.max_deliver,
            }
        }
    }

    /// Records a successful message delivery, clearing failure tracking.
    pub fn record_success(&self, message_id: &str) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        attempts.remove(message_id);
    }

    /// Returns a snapshot of all quarantined dead letter entries.
    pub fn quarantined_entries(&self) -> Vec<DeadLetterEntry> {
        self.dlq
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns the number of messages quarantined in the DLQ.
    pub fn quarantined_count(&self) -> usize {
        self.dlq
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}
