#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::{Method, StatusCode};
use pulsar_server::ServerConfig;
use serde_json::json;
use sha2::{Digest, Sha256};

use common::{build_test_harness, register_and_login, send_json};

#[tokio::test]
async fn append_only_log_and_derived_snapshots_and_revert() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) =
        register_and_login(&harness.router, "hito5-log@example.com", "pw-hito5-12345").await;

    let (st, sess) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&jwt),
        Some(json!({ "title": "Hito 5 Persistence", "mode": "act" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let session_id = sess["id"].as_str().unwrap();

    // Turn 1 event
    let (e1_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/events"),
        Some(&jwt),
        Some(json!({
            "turnSeq": 1,
            "eventKind": "user_message",
            "content": "Analyze repository",
            "vfsCommitSha": "commit-turn-1"
        })),
    )
    .await;
    assert_eq!(e1_st, StatusCode::OK);

    // Turn 2 event
    let (e2_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/events"),
        Some(&jwt),
        Some(json!({
            "turnSeq": 2,
            "eventKind": "tool_execution",
            "content": "cargo check output",
            "toolName": "cargo",
            "vfsCommitSha": "commit-turn-2"
        })),
    )
    .await;
    assert_eq!(e2_st, StatusCode::OK);

    // Verify snapshot derived deterministically
    let (snap_st, snap) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{session_id}/snapshot"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(snap_st, StatusCode::OK);
    assert_eq!(snap["activeTurnSeq"], 2);
    assert_eq!(snap["latestVfsCommitSha"], "commit-turn-2");
    assert_eq!(snap["events"].as_array().unwrap().len(), 2);

    // Revert to Turn 1
    let (rev_st, rev) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/revert"),
        Some(&jwt),
        Some(json!({ "targetTurnSeq": 1 })),
    )
    .await;
    assert_eq!(rev_st, StatusCode::OK);
    assert_eq!(rev["restoredTurnSeq"], 1);

    // Snapshot after revert
    let (_, snap_after) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{session_id}/snapshot"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(snap_after["activeTurnSeq"], 1);
    assert_eq!(snap_after["latestVfsCommitSha"], "commit-turn-1");
    assert_eq!(snap_after["events"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn faithful_export_import_search_and_cold_archiving() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) = register_and_login(
        &harness.router,
        "hito5-export@example.com",
        "pw-hito5-12345",
    )
    .await;

    let (_, sess) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&jwt),
        Some(json!({ "title": "Unique Searchable Title Alpha", "mode": "act" })),
    )
    .await;
    let session_id = sess["id"].as_str().unwrap();

    send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/events"),
        Some(&jwt),
        Some(json!({
            "turnSeq": 1,
            "eventKind": "user_message",
            "content": "quantum algorithm test vector"
        })),
    )
    .await;

    // Export faithful bundle
    let (exp_st, exp) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/export"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(exp_st, StatusCode::OK);
    let jsonl = exp["jsonl"].as_str().unwrap();

    // Import into new session with collision protection
    let (imp_st, imp) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions/import",
        Some(&jwt),
        Some(json!({
            "preferredSessionId": session_id,
            "title": "Imported Clone",
            "jsonl": jsonl
        })),
    )
    .await;
    assert_eq!(imp_st, StatusCode::OK);
    assert_eq!(imp["remappedFromId"], session_id);
    assert_eq!(imp["importedEvents"], 1);

    // Search Projector
    let (src_st, src) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/sessions/search?q=quantum",
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(src_st, StatusCode::OK);
    let hits = src.as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter()
            .any(|h| h["matchedExcerpt"].as_str().unwrap().contains("quantum"))
    );

    // Cold Archiving with Zstandard
    let (arc_st, arc) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/archive"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(arc_st, StatusCode::OK);
    assert!(arc["archived"].as_bool().unwrap());
    assert!(arc["archiveKey"].as_str().unwrap().ends_with(".zst"));

    // Transparent In-place Unarchive
    let (unarc_st, unarc) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/unarchive"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(unarc_st, StatusCode::OK);
    assert!(!unarc["archived"].as_bool().unwrap());
}

#[tokio::test]
async fn phase_based_crash_resume_and_verified_artifact_delivery() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) = register_and_login(
        &harness.router,
        "hito5-delivery@example.com",
        "pw-hito5-12345",
    )
    .await;

    let (_, sess) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&jwt),
        Some(json!({ "title": "Crash Resume Task Session", "mode": "act" })),
    )
    .await;
    let session_id = sess["id"].as_str().unwrap();

    let (_, task) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/tasks"),
        Some(&jwt),
        Some(json!({ "prompt": "Compile deliverables" })),
    )
    .await;
    let task_id = task["id"].as_str().unwrap();

    // Lease task so fencing token is 1
    send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/lease"),
        Some(&jwt),
        Some(json!({ "boxerNodeId": "worker-1" })),
    )
    .await;

    // Record phase "inspect"
    let (ph_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/phases"),
        Some(&jwt),
        Some(json!({
            "phaseName": "inspect",
            "outputSha256": "sha-inspect-ok",
            "fencingToken": 1
        })),
    )
    .await;
    assert_eq!(ph_st, StatusCode::OK);

    // Stale fencing token on phase record rejected with 409
    let (stale_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/phases"),
        Some(&jwt),
        Some(json!({
            "phaseName": "patch",
            "outputSha256": "sha-patch-stale",
            "fencingToken": 0
        })),
    )
    .await;
    assert_eq!(stale_st, StatusCode::CONFLICT);

    // Query crash resume plan
    let (res_st, plan) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/tasks/{task_id}/resume-plan"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(res_st, StatusCode::OK);
    assert_eq!(plan["completedPhases"].as_array().unwrap().len(), 1);
    assert_eq!(plan["nextPendingPhase"], "patch");

    // Deliver verified artifact with SHA-256 digest
    let artifact_payload = b"CRITICAL BINARY DELIVERABLE CONTENT";
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, artifact_payload);
    let digest = hex::encode(Sha256::digest(artifact_payload));

    let (deliv_st, receipt) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/deliver"),
        Some(&jwt),
        Some(json!({
            "filename": "release.bin",
            "contentType": "application/octet-stream",
            "dataBase64": b64,
            "expectedSha256": digest
        })),
    )
    .await;
    assert_eq!(deliv_st, StatusCode::OK);
    assert_eq!(receipt["sha256Digest"], digest);
    assert_eq!(receipt["refCount"], 1);
    assert!(!receipt["deliverySignature"].as_str().unwrap().is_empty());
}
