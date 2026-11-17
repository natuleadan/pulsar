//! Poison message protection and Dead Letter Queue (DLQ) tracking for worker loops.

pub use pulsar_protocol::{
    DEFAULT_MAX_DELIVERY_ATTEMPTS, DeadLetterEntry, DlqDecision, PoisonMessageFilter,
};
