//! Dependency-free shared communication protocol, named codecs, and domain DTOs.

#![forbid(unsafe_code)]

pub mod artifact;
pub mod auth;
pub mod config_hierarchy;
pub mod dlq;
pub mod error_taxonomy;
pub mod identity;
pub mod node;
pub mod plugin;
pub mod preview;
pub mod providers;
pub mod robot;
pub mod rpc;
pub mod sandbox;
pub mod schema;
pub mod skills;
pub mod stream;
pub mod swarm;
pub mod task;
pub mod tool_decision;
pub mod update;
pub mod version;
pub mod workflow;

#[cfg(feature = "openapi")]
pub mod openapi;

pub use artifact::{ArtifactEntry, ArtifactManifest, ArtifactValidationError, is_valid_sha256_hex};
pub use auth::{
    ApiKeyMetadata, ApiKeyScope, CreateApiKeyRequest, LoginRequest, RefreshRequest, TokenPair,
    UserClaims,
};
pub use config_hierarchy::{
    ConfigDiagnostic, ConfigDiagnosticSeverity, ConfigLayer, ConfigResolutionOutcome, ConfigScope,
    EffectiveSettings, resolve_settings_hierarchy, serialize_minimal_diff,
};
pub use dlq::{DEFAULT_MAX_DELIVERY_ATTEMPTS, DeadLetterEntry, DlqDecision, PoisonMessageFilter};
pub use error_taxonomy::{
    ClassifiedProviderError, ProviderErrorClass, classify_provider_error,
    compute_exponential_backoff_ms, scrub_sensitive_tokens,
};
pub use identity::{
    BoxId, GenerationId, HEADER_AGENT_IDENTITY_CODE, HEADER_AGENT_INSTALLATION_ID,
    HEADER_AGENT_ORGANIZATION, InstallationIdentityHeaders, SessionId, StateInvariantError, TaskId,
    TwoTierSessionBinding, WorkgroupId, is_valid_alphanumeric_id, is_valid_organization_name,
    validate_tenant_isolation_invariant, validate_turn_completion_invariant,
    validate_worker_quota_invariant,
};
pub use node::{BoxerNodeAnnouncement, NodeStatus, PreviewRouteSync};
pub use plugin::{
    AgentCard, AgentCardEndpoints, PluginCapabilityMask, PluginManifest, PluginToolDefinition,
};
pub use preview::TraefikPreviewRoute;
pub use providers::{
    ConversationMessage, DiscoveredModel, ProviderProtocol, ToolDefinition,
    UniversalCompletionRequest, UniversalStreamChunk, parse_models_catalog_json, parse_sse_line,
};
pub use robot::{
    DeviceHandshake, DeviceRegistrationResponse, NATS_SUBJECT_ROBOT_COMMANDS_PREFIX,
    NATS_SUBJECT_ROBOT_SAFETY_ALERTS, NATS_SUBJECT_ROBOT_TELEMETRY_PREFIX, RobotCommandFrame,
    RobotDeviceSummary, RobotSafetyAlert, RobotTelemetryFrame, robot_command_subject,
    robot_telemetry_subject,
};
pub use rpc::{
    CursorPage, ExecutionScope, METHOD_CATALOG, PageRequest, RequestId, RpcError, RpcErrorCode,
    RpcMethodSpec, RpcNotification, RpcRequest, RpcResponse, TraceContext, TriStateFilter,
};
pub use sandbox::{
    ExecOptions, IsolationKind, RuntimeKind, SandboxProfile, SandboxProfileError, SetupMode,
    ThirdPartyProvider,
};
pub use schema::{ProtocolSchemaBundle, SandboxMatrixEntry, export_protocol_schema};
pub use skills::{
    BUILTIN_SKILL_CODEBASE_INSPECTOR, BUILTIN_SKILL_CORE_TERMINAL, BUILTIN_SKILL_FILESYSTEM_VFS,
    BUILTIN_SKILL_GIT_VCS, MAX_DESCRIPTION_LENGTH, MAX_SKILL_BUDGET_PERCENT, SkillCatalogItem,
    SkillInstallInput, SkillManifest, SkillSecurityViolation, SkillSource, format_prompt_catalog,
    get_builtin_system_skills, scan_skill_security,
};
pub use stream::{EventEnvelope, EventFeedLane, StreamPayload, TaskStreamEvent, ToolPhase};
pub use swarm::{
    AgentHandoff, MAX_CONCURRENT_SUBAGENTS_PER_TREE, MAX_SUBAGENT_DEPTH, PermissionError,
    PermissionMask, SubagentInceptionError, SubagentLineage, SwarmMessage, SwarmTopic,
    validate_subagent_spawn,
};
pub use task::{
    AgentLifecycleMode, AgentLifecyclePolicy, FencingToken, FollowUpMessage, InferenceEndpointSpec,
    InteractionMode, ProxyCredentialSpec, SteeringInterventionKind, SteeringMessage, StopReason,
    TaskLease, TaskRequest, TaskResult, TaskState, TurnDecision, WorkerStatus,
};
pub use tool_decision::{
    ToolDecisionKind, ToolDecisionVerdict, ToolPolicyRule, build_hygienic_tool_env,
    evaluate_monotonic_tool_decision, is_forbidden_secret_env,
};
pub use update::{
    PackageEntry, PlatformAsset, UpdateChannel, UpdateManifest, UpdateValidationError,
};
pub use version::{API_GENERATION, Capability, CodecId, HandshakeRequest, HandshakeResponse};
pub use workflow::{AutocronSchedule, TriggerKind, WorkflowDag, WorkflowEdge, WorkflowNode};
