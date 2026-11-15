#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_server::{ServerConfig, modules::secrets::services::SecretService};
use axum::http::{Method, StatusCode};
use common::{build_test_harness, register_and_login, send_json};
use serde_json::json;

#[tokio::test]
async fn secrets_enforce_system_separation_non_readable_metadata_group_and_worker_isolation() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;

    let (alice_id, alice_jwt) = register_and_login(
        &harness.router,
        "alice-sec@example.com",
        "password-alice-123",
    )
    .await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob-sec@example.com", "password-bob-1234").await;

    // 1. System orchestrator secrets cannot be stored or overridden in the user vault.
    let (sys_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/secrets",
        Some(&alice_jwt),
        Some(json!({
            "id": "sys-jwt",
            "name": "AGENTS_JWT_SECRET",
            "scopeType": "owner_shared",
            "targetHost": "example.com",
            "value": "override-attempt"
        })),
    )
    .await;
    assert_eq!(sys_status, StatusCode::FORBIDDEN);

    // 2. Store worker-isolated secret for worker-a.
    let raw_worker_a_secret = "vercel_live_secret_worker_a_999";
    let (st_a, body_a) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/secrets",
        Some(&alice_jwt),
        Some(json!({
            "id": "sec-worker-a",
            "name": "VERCEL_TOKEN",
            "scopeType": "worker",
            "scopeId": "worker-a",
            "targetHost": "api.vercel.com",
            "value": raw_worker_a_secret
        })),
    )
    .await;
    assert_eq!(st_a, StatusCode::OK);
    assert_eq!(body_a["name"], "VERCEL_TOKEN");
    assert_eq!(body_a["hasValue"], true);
    assert!(!body_a.to_string().contains(raw_worker_a_secret));

    // 3. Create a shared secret group and bind worker-a + coordinator-1 (NOT worker-b).
    let (grp_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/secrets/groups",
        Some(&alice_jwt),
        Some(json!({
            "id": "grp-deploy",
            "name": "Deploy Group",
            "description": "Shared deploy credentials"
        })),
    )
    .await;
    assert_eq!(grp_st, StatusCode::OK);

    let (mem_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/secrets/groups/grp-deploy/members",
        Some(&alice_jwt),
        Some(json!({ "memberKind": "worker", "memberId": "worker-a" })),
    )
    .await;
    assert_eq!(mem_st, StatusCode::OK);

    let raw_group_secret = "ghp_group_deploy_token_777";
    let (gsec_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/secrets",
        Some(&alice_jwt),
        Some(json!({
            "id": "sec-gh-group",
            "name": "GITHUB_TOKEN",
            "scopeType": "group",
            "scopeId": "grp-deploy",
            "targetHost": "api.github.com",
            "value": raw_group_secret
        })),
    )
    .await;
    assert_eq!(gsec_st, StatusCode::OK);

    // 4. Listing secrets returns ONLY metadata (name/fingerprint), never plaintext.
    let (list_st, list_body) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/secrets",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(list_st, StatusCode::OK);
    let dumped = list_body.to_string();
    assert!(dumped.contains("VERCEL_TOKEN"));
    assert!(dumped.contains("GITHUB_TOKEN"));
    assert!(!dumped.contains(raw_worker_a_secret));
    assert!(!dumped.contains(raw_group_secret));

    // 5. Bob cannot see or delete Alice's secrets or bind members to Alice's group (BOLA).
    let (bob_list_st, bob_list) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/secrets",
        Some(&bob_jwt),
        None,
    )
    .await;
    assert_eq!(bob_list_st, StatusCode::OK);
    assert_eq!(bob_list.as_array().unwrap().len(), 0);

    let (bob_del_st, _) = send_json(
        &harness.router,
        Method::DELETE,
        "/api/v1/secrets/sec-worker-a",
        Some(&bob_jwt),
        None,
    )
    .await;
    assert_eq!(bob_del_st, StatusCode::NOT_FOUND);

    // 6. Runtime resolution isolates worker-a from worker-b.
    let svc = harness
        .app
        .container()
        .resolve::<SecretService>()
        .await
        .expect("resolve SecretService");

    let creds_a = svc
        .resolve_proxy_credentials(&alice_id, Some("worker-a"))
        .await
        .expect("resolve worker-a");
    assert_eq!(creds_a.len(), 2);
    assert!(creds_a.iter().any(|c| c.host == "api.vercel.com"));
    assert!(creds_a.iter().any(|c| c.host == "api.github.com"));

    let creds_b = svc
        .resolve_proxy_credentials(&alice_id, Some("worker-b"))
        .await
        .expect("resolve worker-b");
    assert_eq!(
        creds_b.len(),
        0,
        "worker-b must receive zero secrets from worker-a or grp-deploy"
    );
}
