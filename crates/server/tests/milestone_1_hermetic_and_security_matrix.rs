use pulsar_protocol::{
    ApiKeyScope, PluginCapabilityMask, ProviderErrorClass, classify_provider_error,
};
use pulsar_server::infra::hermetic::{HermeticEnvironment, HermeticSandbox};
use pulsar_server::modules::auth::entities::AuthenticatedActor;
use pulsar_server::modules::auth::guards::AgentCircuitBreaker;
use pulsar_server::modules::mcp::dto::McpToolPolicy;

#[test]
fn test_category_01_hermetic_sandbox_isolation_and_drop_cleanup() {
    let sandbox = HermeticSandbox::new().expect("sandbox initialization must succeed");
    let test_dir = sandbox.root.clone();
    assert!(test_dir.exists(), "sandbox root must exist on creation");
    assert!(sandbox.home.exists());
    assert!(sandbox.tmp.exists());
    assert!(sandbox.scratch.exists());

    // Write file into scratch
    let target = sandbox
        .resolve_scratch_path("inner/task_result.json")
        .expect("valid scratch path");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"{\"status\":\"done\"}").unwrap();
    assert!(target.exists());

    // Verify drop cleanup
    drop(sandbox);
    assert!(
        !test_dir.exists(),
        "hermetic sandbox must be deleted on drop"
    );
    assert!(!target.exists(), "scratch files must be completely wiped");
}

#[test]
fn test_category_02_path_traversal_and_null_byte_rejection() {
    let sandbox = HermeticSandbox::new().unwrap();

    let traversal_1 = sandbox.resolve_scratch_path("../../../etc/passwd");
    assert!(traversal_1.is_err(), "parent traversal must be rejected");

    let traversal_2 = sandbox.resolve_scratch_path("safe/../../sensitive");
    assert!(traversal_2.is_err(), "nested traversal must be rejected");

    let null_byte = sandbox.resolve_scratch_path("file\0.txt");
    assert!(null_byte.is_err(), "null byte path must be rejected");
}

#[test]
fn test_category_03_hermetic_env_isolation_without_host_pollution() {
    let mut env = HermeticEnvironment::new();
    env.set("NLA_APP_SECRET", "super-hermetic-key");
    assert_eq!(env.get("NLA_APP_SECRET"), Some("super-hermetic-key"));
    assert_eq!(env.get("NONEXISTENT_VAR"), None);

    // Host process env is untouched
    assert!(std::env::var("NLA_APP_SECRET").is_err());
}

#[test]
fn test_category_04_scope_rbac_enforcement() {
    let policy = McpToolPolicy::default();

    assert_eq!(
        policy.required_scope("health_summary"),
        Some(&ApiKeyScope::Read)
    );
    assert_eq!(
        policy.required_scope("session_get"),
        Some(&ApiKeyScope::Read)
    );
    assert_eq!(
        policy.required_scope("task_submit"),
        Some(&ApiKeyScope::Write)
    );
    assert_eq!(policy.required_scope("unregistered_tool"), None);

    let read_actor = AuthenticatedActor {
        user_id: "user-1".to_string(),
        email: "alice@test.run".to_string(),
        scopes: vec![ApiKeyScope::Read],
        auth_method: "test".to_string(),
    };
    assert!(read_actor.has_scope(&ApiKeyScope::Read));
    assert!(!read_actor.has_scope(&ApiKeyScope::Write));
}

#[test]
fn test_category_05_plugin_capability_network_fencing() {
    let mask = PluginCapabilityMask {
        allow_network: true,
        allowed_hosts: vec!["api.openai.com".to_string(), "*.bunnycdn.com".to_string()],
        ..Default::default()
    };

    assert!(mask.is_host_allowed("api.openai.com"));
    assert!(mask.is_host_allowed("storage.bunnycdn.com"));
    assert!(!mask.is_host_allowed("169.254.169.254")); // Cloud metadata SSRF blocked
    assert!(!mask.is_host_allowed("localhost"));
    assert!(!mask.is_host_allowed("evil.com"));

    let no_network_mask = PluginCapabilityMask::default();
    assert!(!no_network_mask.is_host_allowed("api.openai.com"));
}

#[tokio::test]
async fn test_category_06_circuit_breaker_trips_on_runaway_agent_loop() {
    let breaker = AgentCircuitBreaker::new();
    let agent_id = "agent-loop-violator";
    let now = 1000;

    assert!(!breaker.is_tripped(agent_id, now).await);
    assert!(!breaker.record_violation(agent_id, now).await);
    assert!(!breaker.record_violation(agent_id, now).await);

    // 3rd violation within window trips breaker
    let tripped = breaker.record_violation(agent_id, now).await;
    assert!(tripped);
    assert!(breaker.is_tripped(agent_id, now).await);
}

#[test]
fn test_category_07_classified_provider_error_taxonomy() {
    let err = classify_provider_error(Some(429), "rate limit exceeded", 0, 3);
    assert_eq!(err.class, ProviderErrorClass::RetryableTransient);
    assert!(err.class.is_retryable());

    let quota_err = classify_provider_error(Some(402), "insufficient quota", 0, 3);
    assert_eq!(quota_err.class, ProviderErrorClass::TerminalQuotaExhausted);
    assert!(quota_err.class.triggers_fallback());
}
