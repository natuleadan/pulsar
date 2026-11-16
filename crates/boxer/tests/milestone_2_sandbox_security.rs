#![allow(clippy::unwrap_used)]

use std::path::Path;

use agents_boxer::{
    daemon::{
        SessionToolMask, SystemOneDecisionKind, WatchdogAction, WatchdogTelemetrySample,
        evaluate_system_one_watchdog,
    },
    proxy::{CredentialRule, OutboundCredentialProxy, is_banned_raw_tcp_port},
    sandbox::{
        HOST_UNCONFINED_ACK_VALUE, build_linux_landlock_spec, generate_macos_seatbelt_profile,
        validate_hardened_host_mode,
    },
    swarm::{
        SoftSandboxActionVerdict, create_git_worktree_sandbox, evaluate_soft_sandbox_escalation,
        remove_git_worktree_sandbox,
    },
};
use serde_json::json;

#[test]
fn native_landlock_and_seatbelt_profiles_enforce_confined_paths() {
    let ws = Path::new("/tmp/agents-boxes/ses_01");
    let skills = Path::new("/opt/skills");
    let landlock = build_linux_landlock_spec(ws, Some(skills), Some(4040));

    assert!(landlock.raw_tcp_banned);
    assert_eq!(landlock.allowed_loopback_port, Some(4040));
    assert!(
        landlock
            .verify_path_access(Path::new("/tmp/agents-boxes/ses_01/src/main.rs"), true)
            .is_ok()
    );
    assert!(
        landlock
            .verify_path_access(Path::new("/opt/skills/git/SKILL.md"), false)
            .is_ok()
    );
    assert!(
        landlock
            .verify_path_access(Path::new("/opt/skills/git/SKILL.md"), true)
            .is_err()
    );
    assert!(
        landlock
            .verify_path_access(Path::new("/etc/shadow"), false)
            .is_err()
    );
    assert!(
        landlock
            .verify_path_access(Path::new("/Users/admin/.ssh/id_ed25519"), false)
            .is_err()
    );

    let seatbelt = generate_macos_seatbelt_profile(ws, false);
    assert!(seatbelt.scheme_source.contains("(deny default)"));
    assert!(seatbelt.scheme_source.contains("(deny network*)"));
    assert!(
        seatbelt
            .scheme_source
            .contains("(deny file-read* (subpath \"/Users\"))")
    );
}

#[test]
fn hardened_host_mode_enforces_fail_closed_consent() {
    let denied = validate_hardened_host_mode(false, Some(HOST_UNCONFINED_ACK_VALUE));
    assert!(denied.is_err());

    let explicit_ack = validate_hardened_host_mode(true, Some(HOST_UNCONFINED_ACK_VALUE)).unwrap();
    assert!(explicit_ack.allowed);
    assert!(!explicit_ack.sandboxed);
    assert!(explicit_ack.warning_banner.is_some());
}

#[tokio::test]
async fn layer7_egress_proxy_bans_raw_tcp_ports_and_records_ledger() {
    assert!(is_banned_raw_tcp_port(5432));
    assert!(is_banned_raw_tcp_port(6379));
    assert!(is_banned_raw_tcp_port(22));
    assert!(!is_banned_raw_tcp_port(443));

    let proxy = OutboundCredentialProxy::start(
        "127.0.0.1:0".parse().unwrap(),
        true,
        vec!["127.0.0.1".to_owned(), "api.supabase.com".to_owned()],
        vec![CredentialRule {
            host: "127.0.0.1".to_owned(),
            header_name: "authorization".to_owned(),
            header_value: "Bearer secret-supabase-token".to_owned(),
        }],
    )
    .await
    .unwrap();

    let client = reqwest::Client::new();
    let forward_url = format!("{}/forward", proxy.endpoint_url());

    let pg_resp = client
        .post(&forward_url)
        .json(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:5432/postgres"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(pg_resp.status().as_u16(), 403);

    let tcp_scheme_resp = client
        .post(&forward_url)
        .json(&json!({
            "method": "GET",
            "url": "tcp://127.0.0.1:6379"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(tcp_scheme_resp.status().as_u16(), 403);
}

#[test]
fn dynamic_tool_mask_turn_boundary_and_30s_system_one_watchdog() {
    let mut mask = SessionToolMask::new();
    let all_tools = vec![
        "read_file".to_owned(),
        "write_file".to_owned(),
        "shell".to_owned(),
    ];

    mask.stage_disable_tool("shell");
    assert!(mask.is_tool_enabled_for_current_turn("shell"));

    let filtered = mask.filter_manifest_for_next_turn(&all_tools);
    assert!(!filtered.contains(&"shell".to_owned()));
    assert!(filtered.contains(&"read_file".to_owned()));
    assert_eq!(mask.turn_epoch, 1);

    let compiling_sample = WatchdogTelemetrySample {
        silence_duration_secs: 35,
        cpu_usage_permille: 850,
        memory_delta_kb: 4096,
        last_tool_name: Some("shell".to_owned()),
        stdout_tail: "   Compiling agents-server v0.0.0".to_owned(),
    };
    let v_compile = evaluate_system_one_watchdog(&compiling_sample);
    assert_eq!(v_compile.decision, SystemOneDecisionKind::CompilingLongTask);
    assert_eq!(v_compile.action, WatchdogAction::ExtendLeaseSecs(60));

    let zombie_sample = WatchdogTelemetrySample {
        silence_duration_secs: 32,
        cpu_usage_permille: 0,
        memory_delta_kb: 0,
        last_tool_name: Some("shell".to_owned()),
        stdout_tail: String::new(),
    };
    let v_zombie = evaluate_system_one_watchdog(&zombie_sample);
    assert_eq!(v_zombie.decision, SystemOneDecisionKind::HangingDead);
    assert_eq!(v_zombie.action, WatchdogAction::HardKillSwitch);
    assert!(v_zombie.confidence_permille >= 850);
}

#[test]
fn git_worktree_soft_sandbox_lifecycle_and_escalation() {
    let tmp = std::env::temp_dir().join(format!("agents-wt-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();

    let wt = create_git_worktree_sandbox(&tmp, "refactor_01").unwrap();
    assert!(wt.worktree_dir.exists());
    assert_eq!(wt.branch_name, "task-refactor_01");

    assert_eq!(
        evaluate_soft_sandbox_escalation("read_file", false, false),
        SoftSandboxActionVerdict::AllowInWorktree
    );
    assert!(matches!(
        evaluate_soft_sandbox_escalation("shell", false, true),
        SoftSandboxActionVerdict::EscalateToHardContainer { .. }
    ));

    remove_git_worktree_sandbox(&wt);
    assert!(!wt.worktree_dir.exists());
    let _ = std::fs::remove_dir_all(&tmp);
}
