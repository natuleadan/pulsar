use std::sync::Arc;

use pulsar_client::core::adapters::{MockTransport, ServerModeProvider, ThirdPartyAdapter};
use pulsar_client::core::backend::{ControlAction, HttpResponse, SandboxProvider};
use pulsar_protocol::{InteractionMode, ThirdPartyProvider};
use serde_json::json;

#[tokio::test]
async fn server_api_covers_nodes_tasks_workflows_autocron_keys_and_a2a() {
    let transport = Arc::new(MockTransport::default());
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({
            "nodes": [{
                "nodeId": "boxer-1",
                "vlanIp": "10.8.0.2",
                "status": "online",
                "activeBoxes": 1,
                "maxConcurrentBoxes": 8
            }]
        })
        .to_string(),
    });
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({ "workflows": [{ "id": "wf-1", "name": "deploy", "approved": true }] })
            .to_string(),
    });
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({
            "schedules": [{
                "id": "cron-1",
                "name": "nightly",
                "cron_expr": "0 0 * * *",
                "enabled": true
            }]
        })
        .to_string(),
    });
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({
            "api_keys": [{ "id": "k-1", "name": "ci", "prefix": "ag_", "created_at": "2026-01-01" }]
        })
        .to_string(),
    });
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({ "name": "pulsar-server" }).to_string(),
    });
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({ "ok": true }).to_string(),
    });

    let provider = ServerModeProvider::new("https://pulsar.example/", transport.clone());
    assert_eq!(
        provider.session_ws_url("s-1"),
        "wss://pulsar.example/api/v1/sessions/s-1/ws"
    );
    assert_eq!(
        provider.session_sse_url("s-1"),
        "https://pulsar.example/api/v1/sessions/s-1/events/stream"
    );

    let nodes = provider.list_nodes("tok").await.expect("list nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node_id, "boxer-1");

    let wfs = provider.list_workflows("tok").await.expect("workflows");
    assert_eq!(wfs[0].id, "wf-1");

    let crons = provider
        .list_autocron_schedules("tok")
        .await
        .expect("autocron");
    assert!(crons[0].enabled);

    let keys = provider.list_api_keys("tok").await.expect("api keys");
    assert_eq!(keys[0].name, "ci");

    let card = provider.get_agent_card().await.expect("agent card");
    assert_eq!(card["name"], "pulsar-server");

    let receipt = provider
        .send_control("tok", "task-7", ControlAction::Pause, "idem-pause")
        .await
        .expect("pause control");
    assert!(receipt.accepted);

    let recorded = transport.recorded_requests();
    assert_eq!(recorded.len(), 6);
    assert!(recorded[0].url.ends_with("/api/v1/nodes"));
    assert!(recorded[4].url.ends_with("/.well-known/agent-card.json"));
    assert!(recorded[5].url.ends_with("/api/v1/tasks/task-7/state"));
    assert_eq!(recorded[5].body.as_deref(), Some(r#"{"state":"paused"}"#));
}

#[test]
fn sse_frame_parser_extracts_event_envelopes() {
    let sample = pulsar_protocol::EventEnvelope {
        seq: 1,
        emitted_at_ms: 100,
        codec: pulsar_protocol::CodecId::EventsV1,
        session_id: "s1".to_owned(),
        task_id: Some("t1".to_owned()),
        payload: pulsar_protocol::StreamPayload::TaskChunk {
            chunk: "hi".to_owned(),
            is_stderr: false,
        },
    };
    let raw = format!(
        "event: message\ndata: {}\n\n",
        serde_json::to_string(&sample).expect("serialize envelope")
    );
    let events = ServerModeProvider::parse_sse_events(&raw);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 1);
}

#[tokio::test]
async fn third_party_adapter_routes_per_provider() {
    let transport = Arc::new(MockTransport::default());
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({ "id": "box-up-1" }).to_string(),
    });
    transport.push_response(HttpResponse {
        status: 200,
        body: json!({ "id": "sb-day-1" }).to_string(),
    });

    let upstash = ThirdPartyAdapter::new(
        ThirdPartyProvider::UpstashBox,
        "https://box.upstash.io",
        transport.clone(),
    );
    let s1 = upstash
        .create_session("k1", "demo", InteractionMode::Act)
        .await
        .expect("upstash session");
    assert_eq!(s1.id, "box-up-1");

    let daytona = ThirdPartyAdapter::new(
        ThirdPartyProvider::Daytona,
        "https://app.daytona.io",
        transport.clone(),
    );
    let s2 = daytona
        .create_session("k2", "demo2", InteractionMode::Act)
        .await
        .expect("daytona session");
    assert_eq!(s2.id, "sb-day-1");

    let reqs = transport.recorded_requests();
    assert_eq!(reqs[0].url, "https://box.upstash.io/v1/boxes");
    assert_eq!(reqs[1].url, "https://app.daytona.io/api/sandbox");
}
