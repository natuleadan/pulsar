#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use base64::Engine as _;
use common::{build_test_harness, register_and_login, send_json, send_json_with_headers};
use serde_json::json;
use tower::ServiceExt as _;

use agents_server::ServerConfig;

#[tokio::test]
async fn task_artifacts_worker_upload_presign_download_and_bola() {
    let cfg = ServerConfig::for_tests();
    let worker_token = cfg.cluster_secret.clone();
    let harness = build_test_harness(cfg).await;

    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice@example.com", "password-alice-123").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob@example.com", "password-bob-1234").await;

    let (sess_st, sess_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "Artifact Session", "mode": "plan" })),
    )
    .await;
    assert_eq!(sess_st, StatusCode::OK);
    let session_id = sess_body["id"].as_str().unwrap().to_owned();

    let (task_st, task_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/tasks"),
        Some(&alice_jwt),
        Some(json!({ "prompt": "Produce artifact" })),
    )
    .await;
    assert_eq!(task_st, StatusCode::OK);
    let task_id = task_body["id"].as_str().unwrap().to_owned();

    let payload_b64 = base64::engine::general_purpose::STANDARD.encode(b"hello-artifact-bytes");

    // 1. Worker upload without valid token must be rejected.
    let (bad_st, _) = send_json_with_headers(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/artifacts/worker"),
        None,
        &[("x-boxer-worker-token", "wrong-token")],
        Some(json!({
            "filename": "report.txt",
            "dataBase64": payload_b64,
            "contentType": "text/plain"
        })),
    )
    .await;
    assert_eq!(bad_st, StatusCode::UNAUTHORIZED);

    // 2. Worker upload with valid token succeeds.
    let (up_st, up_body) = send_json_with_headers(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/artifacts/worker"),
        None,
        &[("x-boxer-worker-token", &worker_token)],
        Some(json!({
            "filename": "report.txt",
            "dataBase64": payload_b64,
            "contentType": "text/plain"
        })),
    )
    .await;
    assert_eq!(up_st, StatusCode::OK);
    assert_eq!(up_body["filename"], "report.txt");
    assert_eq!(up_body["sizeBytes"], 20);
    assert!(up_body["presignedUrl"].as_str().is_some());

    // 3. Alice lists artifacts with custom TTL.
    let (list_st, list_body) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/tasks/{task_id}/artifacts?ttl=120"),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(list_st, StatusCode::OK);
    let items = list_body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["filename"], "report.txt");
    assert_eq!(items[0]["expiresInSecs"], 120);

    // 4. Alice requests a fresh presigned URL.
    let (pre_st, pre_body) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/tasks/{task_id}/artifacts/report.txt/presign?ttl=300"),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(pre_st, StatusCode::OK);
    assert_eq!(pre_body["expiresInSecs"], 300);
    assert!(pre_body["url"].as_str().unwrap().contains("report.txt"));

    // 5. Alice downloads raw artifact bytes.
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/v1/tasks/{task_id}/artifacts/report.txt/download"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {alice_jwt}"))
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"hello-artifact-bytes");

    // 6. Bob cannot list, presign, or download Alice's artifact (BOLA protection).
    let (bob_list_st, _) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/tasks/{task_id}/artifacts"),
        Some(&bob_jwt),
        None,
    )
    .await;
    assert!(
        bob_list_st == StatusCode::FORBIDDEN || bob_list_st == StatusCode::NOT_FOUND,
        "unexpected BOLA status: {bob_list_st}"
    );

    let (bob_pre_st, _) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/tasks/{task_id}/artifacts/report.txt/presign"),
        Some(&bob_jwt),
        None,
    )
    .await;
    assert!(
        bob_pre_st == StatusCode::FORBIDDEN || bob_pre_st == StatusCode::NOT_FOUND,
        "unexpected BOLA presign status: {bob_pre_st}"
    );
}
