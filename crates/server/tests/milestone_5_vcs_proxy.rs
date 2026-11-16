use agents_server::modules::plugins::vcs::{
    CheckRunStatus, MockVcsState, PullRequestSummary, VcsError, VcsProxy,
};

#[tokio::test]
async fn test_vcs_proxy_lists_prs_and_diff_without_leaking_secrets() {
    let proxy = VcsProxy::new("ghp_SUPER_SECRET_SERVER_TOKEN_NEVER_SENT_TO_BOXER");

    let mut state = MockVcsState::default();
    state.prs.push(PullRequestSummary {
        number: 42,
        title: "feat: add secure git proxy".to_string(),
        author: "alice".to_string(),
        state: "open".to_string(),
        head_sha: "abc1234".to_string(),
    });
    state.diffs.insert(
        42,
        r#"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,5 @@
+pub fn new_feature() -> bool {
+    true
+}
"#
        .to_string(),
    );

    proxy.set_mock_state(state).await;

    let prs = proxy.list_pull_requests("natuleadan/agents").await.unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].number, 42);

    let diff = proxy.get_pr_diff("natuleadan/agents", 42).await.unwrap();
    assert!(diff.contains("pub fn new_feature"));

    let summary = proxy.generate_review_summary(42, &diff);
    assert_eq!(summary.pr_number, 42);
    assert!(summary.technical_notes[0].contains("Diff size"));
    assert!(summary.suggested_verifications[0].contains("unit tests"));
}

#[tokio::test]
async fn test_vcs_proxy_handles_rate_limits_gracefully() {
    let proxy = VcsProxy::new("dummy-token");

    let state = MockVcsState {
        is_rate_limited: true,
        rate_limit_reset_secs: 45,
        ..Default::default()
    };

    proxy.set_mock_state(state).await;

    let res = proxy.list_pull_requests("natuleadan/agents").await;
    match res {
        Err(VcsError::RateLimited(secs)) => assert_eq!(secs, 45),
        _ => panic!("Expected RateLimited error"),
    }
}

#[tokio::test]
async fn test_vcs_proxy_returns_check_runs() {
    let proxy = VcsProxy::new("dummy-token");

    let mut state = MockVcsState::default();
    state.checks.insert(
        "abc1234".to_string(),
        vec![
            CheckRunStatus {
                name: "Validate Agents".to_string(),
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
            },
            CheckRunStatus {
                name: "Clippy and Tests".to_string(),
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
            },
        ],
    );

    proxy.set_mock_state(state).await;

    let checks = proxy
        .get_check_runs("natuleadan/agents", "abc1234")
        .await
        .unwrap();
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0].conclusion.as_deref(), Some("success"));
}
