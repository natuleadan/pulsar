#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::{Method, StatusCode};
use pulsar_protocol::{CodecId, EventEnvelope, PermissionMask, StreamPayload, WorkerStatus};
use pulsar_server::{ServerConfig, infra::bus::TwoLaneEventFeedBuffer};
use serde_json::json;

use common::{build_test_harness, register_and_login, send_json};

fn full_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: true,
        can_exec_shell: true,
        can_network: true,
        allowed_tools: vec!["grep".to_owned(), "read".to_owned()],
    }
}

#[tokio::test]
async fn subagent_inception_guardrails_and_mediated_chat_enforce_limits() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) =
        register_and_login(&harness.router, "hito4-swarm@example.com", "pw-hito4-12345").await;

    let (st, sess) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&jwt),
        Some(json!({ "title": "Hito 4 Subagent Tree", "mode": "act" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let session_id = sess["id"].as_str().unwrap();

    // Depth 0 -> Depth 1 (leader -> sub1)
    let (h1_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&jwt),
        Some(json!({
            "parentAgentId": "leader",
            "childAgentId": "sub-1",
            "parentMask": full_mask(),
            "requestedMask": full_mask(),
            "goal": "Level 1 task"
        })),
    )
    .await;
    assert_eq!(h1_st, StatusCode::OK);

    // Depth 1 -> Depth 2 (sub-1 -> sub-2)
    let (h2_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&jwt),
        Some(json!({
            "parentAgentId": "sub-1",
            "childAgentId": "sub-2",
            "parentMask": full_mask(),
            "requestedMask": full_mask(),
            "goal": "Level 2 task"
        })),
    )
    .await;
    assert_eq!(h2_st, StatusCode::OK);

    // Depth 2 -> Depth 3 must be rejected with SUBAGENT_DEPTH_EXCEEDED
    let (h3_st, h3_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&jwt),
        Some(json!({
            "parentAgentId": "sub-2",
            "childAgentId": "sub-3",
            "parentMask": full_mask(),
            "requestedMask": full_mask(),
            "goal": "Level 3 illegal inception"
        })),
    )
    .await;
    assert_eq!(h3_st, StatusCode::FORBIDDEN);
    assert!(
        h3_body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("SUBAGENT_DEPTH_EXCEEDED")
    );

    // Cycle detection (sub-1 -> leader) must be rejected with SUBAGENT_CYCLE_DETECTED
    let (cyc_st, cyc_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&jwt),
        Some(json!({
            "parentAgentId": "sub-1",
            "childAgentId": "leader",
            "parentMask": full_mask(),
            "requestedMask": full_mask(),
            "goal": "Cycle back to leader"
        })),
    )
    .await;
    assert_eq!(cyc_st, StatusCode::FORBIDDEN);
    assert!(
        cyc_body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("SUBAGENT_CYCLE_DETECTED")
    );

    // Mediated subagent dialogue endpoint
    let (chat_st, chat_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/subagents/sub-1/chat"),
        Some(&jwt),
        Some(json!({
            "message": "Focus on crates/protocol first"
        })),
    )
    .await;
    assert_eq!(chat_st, StatusCode::OK);
    assert_eq!(chat_body["targetAgentId"], "sub-1");
    assert_eq!(chat_body["payload"]["kind"], "mediated_operator_chat");
}

#[tokio::test]
async fn steering_fencing_token_and_two_lane_event_feed_preserve_control_signals() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) =
        register_and_login(&harness.router, "hito4-steer@example.com", "pw-hito4-12345").await;

    let (_, sess) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&jwt),
        Some(json!({ "title": "Steering Fencing", "mode": "act" })),
    )
    .await;
    let session_id = sess["id"].as_str().unwrap();

    let (t_st, task) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/tasks"),
        Some(&jwt),
        Some(json!({ "prompt": "Long running build" })),
    )
    .await;
    assert_eq!(t_st, StatusCode::OK);
    let task_id = task["id"].as_str().unwrap();

    let (l_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/lease"),
        Some(&jwt),
        Some(json!({ "boxerNodeId": "node-1" })),
    )
    .await;
    assert_eq!(l_st, StatusCode::OK);

    // Stale fencing token 0 (< active 1) must be rejected with 409 Conflict
    let (stale_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/steer"),
        Some(&jwt),
        Some(json!({
            "content": "stale instruction",
            "kind": "immediateIntervention",
            "fencingToken": 0
        })),
    )
    .await;
    assert_eq!(stale_st, StatusCode::CONFLICT);

    // Current fencing token 1 succeeds and followUpPrompt queues into session follow-ups
    let (ok_st, ok_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/steer"),
        Some(&jwt),
        Some(json!({
            "content": "run clippy after build",
            "kind": "followUpPrompt",
            "fencingToken": 1
        })),
    )
    .await;
    assert_eq!(ok_st, StatusCode::OK);
    assert_eq!(ok_body["kind"], "followUpPrompt");
    assert_eq!(ok_body["fencingToken"], 1);

    // Two-Lane Event-Feed: flooding render lane compacts render events and emits StreamLagged
    // without dropping control lane events.
    let mut feed = TwoLaneEventFeedBuffer::new(4);
    feed.push(EventEnvelope {
        seq: 1,
        emitted_at_ms: 100,
        codec: CodecId::EventsV1,
        session_id: session_id.to_owned(),
        task_id: Some(task_id.to_owned()),
        payload: StreamPayload::WorkerStateChanged {
            node_id: "node-1".to_owned(),
            status: WorkerStatus::Working,
            reason: None,
        },
    });
    for seq in 2..=7 {
        feed.push(EventEnvelope {
            seq,
            emitted_at_ms: 100 + seq,
            codec: CodecId::EventsV1,
            session_id: session_id.to_owned(),
            task_id: Some(task_id.to_owned()),
            payload: StreamPayload::TaskChunk {
                chunk: format!("chunk-{seq}"),
                is_stderr: false,
            },
        });
    }
    feed.push(EventEnvelope {
        seq: 8,
        emitted_at_ms: 200,
        codec: CodecId::EventsV1,
        session_id: session_id.to_owned(),
        task_id: Some(task_id.to_owned()),
        payload: StreamPayload::SteeringAck {
            message_id: "m1".to_owned(),
            accepted: true,
        },
    });

    assert!(feed.total_dropped_render_events() > 0);
    let first = feed.pop_next().unwrap();
    assert!(matches!(
        first.payload,
        StreamPayload::WorkerStateChanged { .. }
    ));
    let second = feed.pop_next().unwrap();
    assert!(matches!(second.payload, StreamPayload::StreamLagged { .. }));
}
