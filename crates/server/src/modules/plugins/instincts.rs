//! Memory with Explicit Provenance and Atomic Instincts
//!
//! Stores contextual memories tagged with turn source and reinforces/decays habits.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProvenance {
    pub source_session_id: String,
    pub turn_index: u32,
    pub timestamp_ms: u64,
    pub author_role: String,
    pub source_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactMemory {
    pub id: String,
    pub fact: String,
    pub provenance: MemoryProvenance,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtomicInstinct {
    pub id: String,
    pub trigger_pattern: String,
    pub action_sequence: Vec<String>,
    pub confidence: f32,
    pub decay_rate: f32,
    pub human_approved: bool,
}

impl AtomicInstinct {
    pub fn new(id: impl Into<String>, trigger: impl Into<String>, actions: Vec<String>) -> Self {
        Self {
            id: id.into(),
            trigger_pattern: trigger.into(),
            action_sequence: actions,
            confidence: 0.5,
            decay_rate: 0.05,
            human_approved: false,
        }
    }

    pub fn reinforce(&mut self) {
        self.confidence = (self.confidence + 0.1).min(1.0);
    }

    pub fn penalize(&mut self) {
        self.confidence = (self.confidence - self.decay_rate).max(0.0);
    }

    pub fn should_purge(&self) -> bool {
        !self.human_approved && self.confidence < 0.15
    }
}

#[derive(Default)]
pub struct MemoryInstinctStore {
    memories: RwLock<HashMap<String, FactMemory>>,
    instincts: RwLock<HashMap<String, AtomicInstinct>>,
}

impl MemoryInstinctStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn record_fact(&self, memory: FactMemory) {
        let mut lock = self.memories.write().await;
        lock.insert(memory.id.clone(), memory);
    }

    pub async fn get_fact(&self, id: &str) -> Option<FactMemory> {
        let lock = self.memories.read().await;
        lock.get(id).cloned()
    }

    pub async fn revoke_fact(&self, id: &str) -> bool {
        let mut lock = self.memories.write().await;
        lock.remove(id).is_some()
    }

    pub async fn register_instinct(&self, instinct: AtomicInstinct) {
        let mut lock = self.instincts.write().await;
        lock.insert(instinct.id.clone(), instinct);
    }

    pub async fn evaluate_instinct(&self, prompt: &str) -> Option<AtomicInstinct> {
        let lock = self.instincts.read().await;
        for instinct in lock.values() {
            if prompt.contains(&instinct.trigger_pattern)
                && (instinct.human_approved || instinct.confidence >= 0.5)
            {
                return Some(instinct.clone());
            }
        }
        None
    }
}
