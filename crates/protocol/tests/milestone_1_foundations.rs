#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use agents_protocol::{
    ConfigDiagnosticSeverity, ConfigLayer, ConfigScope, ConversationMessage, EffectiveSettings,
    ProviderErrorClass, ProviderProtocol, StateInvariantError, TaskState, ToolDecisionKind,
    ToolDefinition, ToolPolicyRule, TwoTierSessionBinding, UniversalCompletionRequest,
    UniversalStreamChunk, WorkgroupId, build_hygienic_tool_env, classify_provider_error,
    evaluate_monotonic_tool_decision, parse_models_catalog_json, parse_sse_line,
    resolve_settings_hierarchy, scrub_sensitive_tokens, serialize_minimal_diff,
    validate_tenant_isolation_invariant, validate_turn_completion_invariant,
    validate_worker_quota_invariant,
};
use serde_json::json;

#[test]
fn universal_provider_protocol_normalizer_and_sse_parser() {
    let openai_proto = ProviderProtocol::from_kind("deepseek");
    assert_eq!(openai_proto, ProviderProtocol::OpenAiCompatible);
    assert_eq!(
        openai_proto.completion_url("https://api.deepseek.com", "deepseek-chat"),
        "https://api.deepseek.com/v1/chat/completions"
    );

    let anthropic_proto = ProviderProtocol::from_kind("anthropic");
    assert_eq!(anthropic_proto, ProviderProtocol::AnthropicMessages);
    assert_eq!(
        anthropic_proto.completion_url("https://api.anthropic.com", "claude-3-7-sonnet"),
        "https://api.anthropic.com/v1/messages"
    );

    let gemini_proto = ProviderProtocol::from_kind("gemini");
    assert_eq!(gemini_proto, ProviderProtocol::GeminiGenerateContent);

    let req = UniversalCompletionRequest {
        model: "claude-3-7-sonnet".to_owned(),
        messages: vec![
            ConversationMessage {
                role: "system".to_owned(),
                content: "Be concise".to_owned(),
                tool_call_id: None,
            },
            ConversationMessage {
                role: "user".to_owned(),
                content: "Hello".to_owned(),
                tool_call_id: None,
            },
        ],
        temperature: Some(0.2),
        max_tokens: Some(1024),
        tools: vec![ToolDefinition {
            name: "read_file".to_owned(),
            description: "Read file".to_owned(),
            parameters: json!({"type": "object"}),
        }],
        stream: true,
    };

    let anth_payload = req.to_protocol_payload(&ProviderProtocol::AnthropicMessages);
    assert_eq!(anth_payload["system"], "Be concise");
    assert_eq!(anth_payload["messages"].as_array().unwrap().len(), 1);
    assert_eq!(anth_payload["tools"][0]["name"], "read_file");

    let oai_payload = req.to_protocol_payload(&ProviderProtocol::OpenAiCompatible);
    assert_eq!(oai_payload["messages"].as_array().unwrap().len(), 2);
    assert_eq!(oai_payload["tools"][0]["type"], "function");

    let sse_text = parse_sse_line(
        &ProviderProtocol::OpenAiCompatible,
        r#"data: {"choices":[{"delta":{"content":"world"}}]}"#,
    );
    assert_eq!(
        sse_text,
        Some(UniversalStreamChunk::TextDelta {
            delta: "world".to_owned()
        })
    );

    let sse_anth = parse_sse_line(
        &ProviderProtocol::AnthropicMessages,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    );
    assert_eq!(
        sse_anth,
        Some(UniversalStreamChunk::TextDelta {
            delta: "hi".to_owned()
        })
    );

    let catalog = parse_models_catalog_json(
        &json!({
            "data": [
                { "id": "deepseek-chat", "owned_by": "deepseek", "context_window": 65536 },
                { "name": "models/gemini-2.5-pro", "inputTokenLimit": 1048576 }
            ]
        }),
        "custom",
    );
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog[0].id, "deepseek-chat");
    assert_eq!(catalog[1].id, "gemini-2.5-pro");
    assert_eq!(catalog[1].context_window, Some(1_048_576));
}

#[test]
fn config_precedence_hierarchy_and_tolerant_validation() {
    let mut user_layer = ConfigLayer {
        theme: Some("light".to_owned()),
        default_model: Some("user-model".to_owned()),
        max_turns: Some(999),
        ..Default::default()
    };
    user_layer
        .unknown_fields
        .insert("cosmetic Experimental".to_owned(), json!(true));

    let project_layer = ConfigLayer {
        default_model: Some("project-model".to_owned()),
        theme: Some("neon-invalid-theme".to_owned()),
        ..Default::default()
    };

    let session_layer = ConfigLayer {
        default_model: Some("session-model".to_owned()),
        ..Default::default()
    };

    let cli_layer = ConfigLayer {
        default_model: Some("cli-flag-model".to_owned()),
        bind_addr: Some("0.0.0.0:9090".to_owned()),
        ..Default::default()
    };

    let outcome = resolve_settings_hierarchy(&[
        (ConfigScope::Project, project_layer),
        (ConfigScope::User, user_layer),
        (ConfigScope::CliFlags, cli_layer),
        (ConfigScope::Session, session_layer),
    ]);

    assert!(outcome.is_valid());
    assert_eq!(outcome.settings.default_model, "cli-flag-model");
    assert_eq!(outcome.settings.bind_addr, "0.0.0.0:9090");
    assert_eq!(outcome.settings.theme, "light");
    assert_eq!(outcome.settings.max_turns, 25);
    assert_eq!(
        outcome.provenance.get("defaultModel"),
        Some(&ConfigScope::CliFlags)
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.severity == ConfigDiagnosticSeverity::Warning && w.field == "theme")
    );

    let diff = serialize_minimal_diff(&outcome.settings, &EffectiveSettings::default());
    assert_eq!(diff["defaultModel"], "cli-flag-model");
    assert_eq!(diff["theme"], "light");
    assert!(diff.get("language").is_none());

    let fatal_outcome = resolve_settings_hierarchy(&[(
        ConfigScope::CliFlags,
        ConfigLayer {
            bind_addr: Some("invalid-address-no-port".to_owned()),
            credential_master_key: Some("short".to_owned()),
            ..Default::default()
        },
    )]);
    assert!(!fatal_outcome.is_valid());
    assert_eq!(fatal_outcome.fatals.len(), 2);
}

#[test]
fn two_tier_stable_identity_and_formal_invariants_10k_cases() {
    let mut binding = TwoTierSessionBinding::new("ses_stable_001", "wg_alpha");
    assert_eq!(binding.session_id.0, "ses_stable_001");
    assert_eq!(binding.generation_id.0, 1);

    binding.rebind_ephemeral(
        "box_ephemeral_1",
        "boxer.stream.1",
        Some("ws_conn_1".to_owned()),
    );
    let stable_before = binding.session_id.clone();
    binding.rebind_ephemeral(
        "box_ephemeral_2",
        "boxer.stream.2",
        Some("ws_conn_2".to_owned()),
    );
    assert_eq!(binding.session_id, stable_before);
    assert_eq!(binding.generation_id.0, 3);
    assert_eq!(binding.active_box_id.as_ref().unwrap().0, "box_ephemeral_2");

    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    for i in 0..10_000_u64 {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let msgs = (seed % 4) as usize;
        let tools = ((seed >> 8) % 4) as usize;
        let tokens = (seed >> 16) % 500;
        let mem = (seed >> 32) % 512;

        let completion_res =
            validate_turn_completion_invariant(&format!("tsk_{i}"), &TaskState::Done, msgs, tools);
        if msgs == 0 && tools == 0 {
            assert!(matches!(
                completion_res,
                Err(StateInvariantError::CompletionWithoutOutput { .. })
            ));
        } else {
            assert!(completion_res.is_ok());
        }

        let quota_res = validate_worker_quota_invariant("worker_1", tokens, mem);
        if tokens == 0 || mem == 0 {
            assert!(matches!(
                quota_res,
                Err(StateInvariantError::QuotaExhaustedDispatch { .. })
            ));
        } else {
            assert!(quota_res.is_ok());
        }

        let wg_a = WorkgroupId(format!("wg_{}", seed % 10));
        let wg_b = WorkgroupId(format!("wg_{}", (seed >> 4) % 10));
        let tenant_res = validate_tenant_isolation_invariant("ses_1", &wg_a, &wg_b);
        if wg_a == wg_b {
            assert!(tenant_res.is_ok());
        } else {
            assert!(matches!(
                tenant_res,
                Err(StateInvariantError::CrossTenantCollision { .. })
            ));
        }
    }
}

#[test]
fn monotonic_tool_decision_sink_state_and_env_hygiene() {
    let rules = vec![
        ToolPolicyRule {
            rule_id: "r1-allow-all".to_owned(),
            tool_pattern: "*".to_owned(),
            decision: ToolDecisionKind::Allow,
            reason: "workspace baseline allow".to_owned(),
        },
        ToolPolicyRule {
            rule_id: "r2-ask-shell".to_owned(),
            tool_pattern: "shell_*".to_owned(),
            decision: ToolDecisionKind::Ask,
            reason: "shell requires confirmation".to_owned(),
        },
        ToolPolicyRule {
            rule_id: "r3-deny-rm".to_owned(),
            tool_pattern: "shell_rm".to_owned(),
            decision: ToolDecisionKind::Deny,
            reason: "destructive rm banned by admin".to_owned(),
        },
        ToolPolicyRule {
            rule_id: "r4-subordinate-allow".to_owned(),
            tool_pattern: "shell_rm".to_owned(),
            decision: ToolDecisionKind::Allow,
            reason: "attempt to override deny".to_owned(),
        },
    ];

    let verdict = evaluate_monotonic_tool_decision("shell_rm", &rules, ToolDecisionKind::Allow);
    assert_eq!(verdict.decision, ToolDecisionKind::Deny);
    assert_eq!(verdict.winning_rule_id, "r3-deny-rm");

    let ask_verdict = evaluate_monotonic_tool_decision("shell_ls", &rules, ToolDecisionKind::Allow);
    assert_eq!(ask_verdict.decision, ToolDecisionKind::Ask);

    let mut host_env = BTreeMap::new();
    host_env.insert("HOME".to_owned(), "/home/agent".to_owned());
    host_env.insert(
        "AGENTS_JWT_SECRET".to_owned(),
        "super-secret-jwt-key".to_owned(),
    );
    host_env.insert(
        "DATABASE_URL".to_owned(),
        "postgres://admin:pass@db/prod".to_owned(),
    );
    host_env.insert("OPENAI_API_KEY".to_owned(), "sk-live-12345".to_owned());

    let allowlist = vec![
        "HOME".to_owned(),
        "AGENTS_JWT_SECRET".to_owned(),
        "DATABASE_URL".to_owned(),
    ];
    let clean_env = build_hygienic_tool_env(&host_env, &allowlist, &BTreeMap::new());
    assert_eq!(
        clean_env.get("HOME").map(String::as_str),
        Some("/home/agent")
    );
    assert_eq!(
        clean_env.get("NLA_SANDBOXED").map(String::as_str),
        Some("1")
    );
    assert!(!clean_env.contains_key("AGENTS_JWT_SECRET"));
    assert!(!clean_env.contains_key("DATABASE_URL"));
    assert!(!clean_env.contains_key("OPENAI_API_KEY"));
}

#[test]
fn explicit_error_taxonomy_and_credential_obfuscation() {
    let err_400 = classify_provider_error(
        Some(400),
        "Invalid JSON syntax near line 1 with key sk-proj-9999999999",
        0,
        3,
    );
    assert_eq!(err_400.class, ProviderErrorClass::NonRetryableClient);
    assert!(!err_400.class.is_retryable());
    assert!(err_400.recommended_backoff_ms.is_none());
    assert!(!err_400.should_degrade_to_fallback);
    assert!(!err_400.sanitized_message.contains("sk-proj-9999999999"));
    assert!(err_400.sanitized_message.contains("[REDACTED_CREDENTIAL]"));

    let err_429_first = classify_provider_error(Some(429), "Rate limit reached", 0, 3);
    assert_eq!(err_429_first.class, ProviderErrorClass::RetryableTransient);
    assert!(err_429_first.recommended_backoff_ms.is_some());
    assert!(!err_429_first.should_degrade_to_fallback);

    let err_429_exhausted = classify_provider_error(Some(429), "Rate limit reached", 3, 3);
    assert_eq!(
        err_429_exhausted.class,
        ProviderErrorClass::RetryableTransient
    );
    assert!(err_429_exhausted.recommended_backoff_ms.is_none());
    assert!(err_429_exhausted.should_degrade_to_fallback);

    let err_quota = classify_provider_error(
        Some(429),
        r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#,
        0,
        3,
    );
    assert_eq!(err_quota.class, ProviderErrorClass::TerminalQuotaExhausted);
    assert!(err_quota.recommended_backoff_ms.is_none());
    assert!(err_quota.should_degrade_to_fallback);

    let scrubbed = scrub_sensitive_tokens(
        "Failed request with Authorization: Bearer=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9 and ghp_secret1234",
    );
    assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
    assert!(!scrubbed.contains("ghp_secret1234"));
}
