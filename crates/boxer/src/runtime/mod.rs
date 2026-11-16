//! Ultra-lean native Rust two-level agent execution engine (`agents-runtime`),
//! branching session tree with atomic tool compaction, host hooks, and multi-provider adapters.

mod batch;
pub mod codemode;
pub mod durable;
pub mod engine;
pub mod hooks;
pub mod providers;
pub mod session_tree;

pub use codemode::{
    CODEMODE_MAX_CPU_SECS, CODEMODE_MAX_MEMORY_BYTES, CODEMODE_MAX_OUTPUT_CHARS,
    CODEMODE_MAX_STATEMENTS, CodemodeExecutionReport, CodemodeScript, CodemodeStatement,
    SandboxedCodemodeVm,
};
pub use durable::{
    DurableReplayTracker, DurableStepKind, DurableStepLogger, DurableStepRecord,
    InMemoryDurableLogger, JsonlFileDurableLogger,
};
pub use engine::{
    NativeAgentRuntime, NativeRuntimeConfig, NativeTurnOutcome, RetryAuditEntry, RuntimeTool,
    ToolCallRequest, ToolExecutionResult,
};
pub use hooks::{
    DefaultLoopHooks, LoopHooks, OutputSchemaContract, RetryPolicy, SchemaRetryAttempt,
    extract_json_object_slice,
};
pub use providers::{
    HttpProviderClient, ModelClient, ModelTurnRequest, ModelTurnResponse, ProviderEndpointConfig,
    ProviderKind, ProviderRegistry, ScriptedModelClient,
};
pub use session_tree::{
    FileCheckpoint, MemoryEntry, SessionBranch, SessionEntry, SessionNode, SessionTree,
};
