use std::path::PathBuf;

use pulsar_boxer::{
    error::BoxerError,
    runtime::{ProviderEndpointConfig, ProviderKind, ProviderRegistry},
    swarm::{
        build_signed_swarm_message, compute_swarm_signature, provision_task_worktree,
        swarm_nats_subject, validate_child_handoff, verify_swarm_signature,
    },
};
use pulsar_protocol::{PermissionMask, SwarmTopic};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn read_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: false,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["grep".to_owned(), "read".to_owned()],
    }
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn provider_registry_and_swarm_asi05_asi07_and_worktree_drop_guard() -> TestResult {
    let mut reg = ProviderRegistry::new();
    for (id, kind) in [
        ("ollama-local", ProviderKind::Ollama),
        ("lmstudio-local", ProviderKind::LmStudio),
        ("openai-main", ProviderKind::OpenAi),
        ("anthropic-main", ProviderKind::Anthropic),
        ("deepinfra-main", ProviderKind::DeepInfra),
        (
            "provider_endpoint:vllm-custom-99",
            ProviderKind::Custom("vllm".to_owned()),
        ),
    ] {
        reg.register(ProviderEndpointConfig {
            id: id.to_owned(),
            kind,
            base_url: "http://127.0.0.1:8080".to_owned(),
            model: "test-model".to_owned(),
            fallback_endpoint_id: None,
            api_key_env: None,
            default_headers: Default::default(),
        });
    }
    assert!(reg.resolve("provider_endpoint:vllm-custom-99").is_some());
    assert!(reg.resolve("vllm-custom-99").is_some());

    // Swarm ASI-05 signed messages & NATS subjects
    let secret = b"swarm-hmac-test-secret";
    let mut msg = build_signed_swarm_message(
        secret,
        "sess-sw",
        "planner-1",
        SwarmTopic::DirectAgent("coder-2".to_owned()),
        json!({ "task": "implement module" }),
        read_mask(),
    );
    assert_eq!(
        swarm_nats_subject(&msg.session_id, &msg.to),
        "swarm.sess-sw.agent.coder-2"
    );
    assert_eq!(msg.signature, compute_swarm_signature(secret, &msg));
    verify_swarm_signature(secret, &msg)?;

    // Tampering fails closed
    msg.payload = json!({ "task": "rm -rf /" });
    assert!(verify_swarm_signature(secret, &msg).is_err());

    // Swarm ASI-07 child permission escalation rejection
    let parent = read_mask();
    let mut escalated = read_mask();
    escalated.can_exec_shell = true;
    let err = validate_child_handoff(
        &parent,
        "planner-1",
        "coder-2",
        &escalated,
        Some("feat/sub".to_owned()),
        "try shell",
    );
    assert!(matches!(err, Err(BoxerError::Permission(_))));

    let valid = validate_child_handoff(
        &parent,
        "planner-1",
        "reader-2",
        &read_mask(),
        Some("feat/sub".to_owned()),
        "inspect code",
    )?;
    assert_eq!(valid.delegated_mask, parent);

    // Worktree isolation + DropGuard
    let root = unique_temp_dir("boxer-worktree")?;
    let canonical_root = std::fs::canonicalize(&root)?;
    let (wt_path, _guard) = provision_task_worktree(&root, "sess-wt", "task_alpha_1")?;
    assert!(wt_path.starts_with(&canonical_root));
    assert!(provision_task_worktree(&root, "sess-wt", "../etc").is_ok());
    assert!(provision_task_worktree(&root, "sess-wt", "///").is_err());
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn universal_llm_tool_calls_parsing_test() {
    use pulsar_boxer::runtime::providers::parse_tool_calls_and_reason;
    use pulsar_protocol::StopReason;
    use serde_json::json;

    // 1. OpenAI / DeepSeek format
    let openai_payload = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Running command",
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "bash",
                        "arguments": "{\"command\":\"ls -la\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "total_tokens": 42 }
    });

    let (calls, stop, tokens) = parse_tool_calls_and_reason(&openai_payload);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_name, "bash");
    assert_eq!(calls[0].call_id, "call_123");
    assert_eq!(calls[0].input["command"], "ls -la");
    assert_eq!(stop, StopReason::ToolUse);
    assert_eq!(tokens, 42);

    // 2. Anthropic tool use format
    let anthropic_payload = json!({
        "content": [
            { "type": "text", "text": "Auditing repository" },
            {
                "type": "tool_use",
                "id": "toolu_abc",
                "name": "read_file",
                "input": { "path": "/etc/hosts" }
            }
        ],
        "stop_reason": "tool_use",
        "usage": { "input_tokens": 10, "output_tokens": 15 }
    });

    let (calls2, stop2, tokens2) = parse_tool_calls_and_reason(&anthropic_payload);
    assert_eq!(calls2.len(), 1);
    assert_eq!(calls2[0].tool_name, "read_file");
    assert_eq!(calls2[0].call_id, "toolu_abc");
    assert_eq!(calls2[0].input["path"], "/etc/hosts");
    assert_eq!(stop2, StopReason::ToolUse);
    assert_eq!(tokens2, 25);

    // 3. Plain text end turn
    let plain_payload = json!({
        "choices": [{
            "message": { "role": "assistant", "content": "Done" },
            "finish_reason": "stop"
        }],
        "usage": { "total_tokens": 12 }
    });

    let (calls3, stop3, tokens3) = parse_tool_calls_and_reason(&plain_payload);
    assert!(calls3.is_empty());
    assert_eq!(stop3, StopReason::EndTurn);
    assert_eq!(tokens3, 12);
}
