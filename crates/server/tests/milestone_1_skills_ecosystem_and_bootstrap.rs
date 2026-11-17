//! Milestone 1 (Batch 8): Skills ecosystem, 4 built-in system skills,
//! dynamic installation API, context budget enforcement, and static security scanning.

use pulsar_protocol::{SkillInstallInput, SkillSource};
use pulsar_server::{infra::error::ServerError, modules::skills::SkillService};

#[tokio::test]
async fn test_builtin_system_skills_bootstrapped_and_protected() {
    let service = SkillService::new();
    let all = service.list().await;

    // Verify 4 built-in skills exist
    assert!(all.len() >= 4);
    let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"core-terminal"));
    assert!(ids.contains(&"filesystem-vfs"));
    assert!(ids.contains(&"git-vcs"));
    assert!(ids.contains(&"codebase-inspector"));

    // Verify each is marked built_in
    for id in [
        "core-terminal",
        "filesystem-vfs",
        "git-vcs",
        "codebase-inspector",
    ] {
        let skill = service.get(id).await.expect("skill must exist");
        assert!(skill.built_in);
        assert_eq!(skill.source, SkillSource::BuiltIn);
        assert!(!skill.instructions.is_empty());
    }

    // Verify deletion of built-in skill is forbidden
    let del_res = service.delete("core-terminal").await;
    assert!(
        matches!(del_res, Err(ServerError::Forbidden(_))),
        "deleting built-in skill must fail closed"
    );

    // Verify overwriting built-in skill is forbidden
    let overwrite_res = service
        .install(SkillInstallInput::Markdown {
            name: "core-terminal".into(),
            description: Some("malicious hijack".into()),
            content: "echo hijacked".into(),
        })
        .await;
    assert!(
        matches!(overwrite_res, Err(ServerError::Forbidden(_))),
        "overwriting built-in skill must fail closed"
    );
}

#[tokio::test]
async fn test_dynamic_skill_installation_from_markdown_git_and_archive() {
    let service = SkillService::new();

    // 1. Dynamic Markdown skill installation
    let md_skill = service
        .install(SkillInstallInput::Markdown {
            name: "postgres-inspector".into(),
            description: Some("Query and explain slow queries in PostgreSQL".into()),
            content: "# Postgres Inspector\nProvides read-only database query execution.".into(),
        })
        .await
        .expect("markdown install should succeed");
    assert_eq!(md_skill.id, "postgres-inspector");
    assert!(!md_skill.built_in);

    // 2. Dynamic Git URL skill installation
    let git_skill = service
        .install(SkillInstallInput::Git {
            url: "https://github.com/community/docker-sandbox-skill.git".into(),
            branch: Some("main".into()),
        })
        .await
        .expect("git install should succeed");
    assert_eq!(git_skill.id, "docker-sandbox-skill");
    assert!(!git_skill.built_in);

    // 3. Dynamic Archive skill installation
    let sample_payload = b"# Custom Compressed Skill\nInstructions for offline agent.";
    let encoded =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sample_payload);
    let archive_skill = service
        .install(SkillInstallInput::Archive {
            name: "embedded-linter".into(),
            data_base64: encoded,
        })
        .await
        .expect("archive install should succeed");
    assert_eq!(archive_skill.id, "embedded-linter");
    assert!(!archive_skill.built_in);

    // 4. Verify custom skill can be deleted cleanly
    service
        .delete("postgres-inspector")
        .await
        .expect("deleting custom skill should succeed");
    assert!(service.get("postgres-inspector").await.is_none());
}

#[tokio::test]
async fn test_static_security_scanner_rejects_malicious_skills() {
    let service = SkillService::new();

    // Test sudo elevation rejection
    let sudo_bad = service
        .install(SkillInstallInput::Markdown {
            name: "bad-sudo".into(),
            description: None,
            content: "Step 1: run sudo apt install curl".into(),
        })
        .await;
    assert!(matches!(sudo_bad, Err(ServerError::Forbidden(_))));

    // Test curl | bash pipe rejection
    let pipe_bad = service
        .install(SkillInstallInput::Markdown {
            name: "bad-pipe".into(),
            description: None,
            content: "run curl | bash to bootstrap remote agent".into(),
        })
        .await;
    assert!(matches!(pipe_bad, Err(ServerError::Forbidden(_))));

    // Test leaked API secret key rejection
    let secret_bad = service
        .install(SkillInstallInput::Markdown {
            name: "bad-secret".into(),
            description: None,
            content: "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE".into(),
        })
        .await;
    assert!(matches!(secret_bad, Err(ServerError::Forbidden(_))));
}

#[tokio::test]
async fn test_context_budget_enforces_two_percent_limit() {
    let service = SkillService::new();

    // With typical 128k context window, catalog fits comfortably under 2% (~2560 tokens)
    let catalog = service
        .catalog_for_prompt(128_000)
        .await
        .expect("128k context should easily accommodate built-in skills");
    assert!(catalog.contains("Available Skills:"));
    assert!(catalog.contains("core-terminal"));
    assert!(catalog.contains("git-vcs"));

    // With tiny context window (e.g. 50 tokens -> 2% is 1 token), budget must reject
    let tiny_res = service.catalog_for_prompt(50).await;
    assert!(
        matches!(tiny_res, Err(ServerError::Validation(_))),
        "catalog must reject when budget is exceeded"
    );
}
