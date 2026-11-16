#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_server::ServerConfig;
use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{build_test_harness, register_and_login, send_json};

#[tokio::test]
async fn workgroup_three_tier_quotas_cross_delegation_and_rbac() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice-wg@example.com", "pw-alice-12345").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob-wg@example.com", "pw-bob-123456").await;

    // 1. Alice creates Workgroup A
    let (st_wg, wg_a) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/workgroups",
        Some(&alice_jwt),
        Some(json!({
            "name": "Team Alpha",
            "maxConcurrentBoxes": 12,
            "maxSubagentDepth": 2,
            "maxSubagentsPerTree": 5,
            "internalTokensQuota": 50_000_000,
            "dollarBudgetCents": 10_000,
            "failClosedOnBudget": true,
            "allowedStorageMb": 10_240
        })),
    )
    .await;
    assert_eq!(st_wg, StatusCode::OK);
    let wg_a_id = wg_a["id"].as_str().unwrap();
    assert_eq!(wg_a["maxConcurrentBoxes"], 12);

    // 2. Bob attempts to modify Workgroup A limits (RBAC check -> 403 Forbidden)
    let (st_forbid, _) = send_json(
        &harness.router,
        Method::PUT,
        &format!("/api/v1/workgroups/{wg_a_id}/limits"),
        Some(&bob_jwt),
        Some(json!({ "maxConcurrentBoxes": 20 })),
    )
    .await;
    assert_eq!(st_forbid, StatusCode::FORBIDDEN);

    // 3. Alice adds Bob as WorkgroupAdmin
    let (st_mem, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/workgroups/{wg_a_id}/members"),
        Some(&alice_jwt),
        Some(json!({
            "userId": "usr_bob_id",
            "role": "workgroup_admin"
        })),
    )
    .await;
    assert_eq!(st_mem, StatusCode::NO_CONTENT);

    // 4. Cross-group delegation: delegate 2 boxes to Team Beta
    let (st_del, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/workgroups/{wg_a_id}/delegations"),
        Some(&alice_jwt),
        Some(json!({
            "targetWorkgroupId": "wg_beta",
            "agentRoleSlug": "pepe-analyst",
            "maxDelegatedBoxes": 2,
            "durationSeconds": 3600
        })),
    )
    .await;
    assert_eq!(st_del, StatusCode::NO_CONTENT);

    // 5. Query usage in real time
    let (st_usg, usg) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/workgroups/{wg_a_id}/usage"),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(st_usg, StatusCode::OK);
    assert_eq!(usg["maxConcurrentBoxes"], 12);
    assert_eq!(usg["dollarBudgetCents"], 10_000);
}

#[tokio::test]
async fn coordinator_pricing_quotes_tamper_proofing_and_checkout() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) = register_and_login(&harness.router, "buyer@example.com", "pw-buyer-12345").await;

    // 1. Request formal task quote
    let (st_q, quote) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/coordinator/quotes",
        Some(&jwt),
        Some(json!({
            "taskId": "task-quote-001",
            "prompt": "Full stack security audit and compilation",
            "estimatedSteps": 15,
            "sandboxRequired": true
        })),
    )
    .await;
    assert_eq!(st_q, StatusCode::OK);
    let quote_id = quote["quoteId"].as_str().unwrap();
    let signature = quote["signature"].as_str().unwrap();
    assert_eq!(quote["amountCents"], 5 + 15 * 2 + 10); // 45 cents
    assert!(!signature.is_empty());

    // 2. Tampered signature checkout rejected with 403 Forbidden
    let (st_tamp, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/coordinator/checkout",
        Some(&jwt),
        Some(json!({
            "quoteId": quote_id,
            "taskId": "task-quote-001",
            "signature": "tampered-bad-signature"
        })),
    )
    .await;
    assert_eq!(st_tamp, StatusCode::FORBIDDEN);

    // 3. Valid settlement checkout unblocks task to Pending
    let (st_ok, checkout) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/coordinator/checkout",
        Some(&jwt),
        Some(json!({
            "quoteId": quote_id,
            "taskId": "task-quote-001",
            "signature": signature
        })),
    )
    .await;
    assert_eq!(st_ok, StatusCode::OK);
    assert!(checkout["settled"].as_bool().unwrap());
    assert_eq!(checkout["state"], "Pending");
    assert!(!checkout["receiptId"].as_str().unwrap().is_empty());
}
