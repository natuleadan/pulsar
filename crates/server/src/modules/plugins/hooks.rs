//! Plugin Lifecycle Hook Dispatcher
//!
//! Intercepts agent loop phases (`pre_turn`, `post_turn`, `on_tool_call`, `on_error`).

use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookStage {
    PreTurn,
    PostTurn,
    OnToolCall,
    OnError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFailurePolicy {
    FailClosed,
    Ignore,
}

pub type HookHandler = Arc<
    dyn Fn(
            HookStage,
            Value,
        ) -> futures_util::future::BoxFuture<'static, Result<Option<Value>, String>>
        + Send
        + Sync,
>;

pub struct RegisteredHook {
    pub plugin_id: String,
    pub stage: HookStage,
    pub policy: HookFailurePolicy,
    pub handler: HookHandler,
}

#[derive(Default)]
pub struct HookDispatcher {
    hooks: RwLock<Vec<RegisteredHook>>,
}

impl HookDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register_hook(&self, hook: RegisteredHook) {
        let mut lock = self.hooks.write().await;
        lock.push(hook);
    }

    /// Dispatches an event through registered hooks for `stage`.
    pub async fn dispatch(
        &self,
        stage: HookStage,
        initial_payload: Value,
    ) -> Result<Value, String> {
        let lock = self.hooks.read().await;
        let mut current_payload = initial_payload;

        for hook in lock.iter().filter(|h| h.stage == stage) {
            match (hook.handler)(stage.clone(), current_payload.clone()).await {
                Ok(Some(mutated)) => {
                    current_payload = mutated;
                }
                Ok(None) => {}
                Err(err) => {
                    if hook.policy == HookFailurePolicy::FailClosed {
                        return Err(format!(
                            "hook failure in plugin `{}` on {:?}: {}",
                            hook.plugin_id, stage, err
                        ));
                    }
                }
            }
        }

        Ok(current_payload)
    }
}
