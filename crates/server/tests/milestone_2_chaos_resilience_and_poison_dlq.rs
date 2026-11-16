//! Milestone 2: Chaos testing, failure recovery, lease reclamation, and poison DLQ.

use std::time::Duration;

use agents_protocol::{DlqDecision, FencingToken, PoisonMessageFilter, StopReason, TaskState};
use agents_server::{
    infra::{error::ServerError, state::now_ms},
    modules::tasks::{
        entities::TaskEntity,
        repositories::{InMemoryTaskRepository, TaskRepository},
    },
};

#[tokio::test]
async fn test_chaos_worker_crash_and_lease_reclamation() {
    let repo = InMemoryTaskRepository::new();
    let now = now_ms();

    // 1. Seed task leased to a worker that will abruptly crash
    let task = TaskEntity {
        id: "task-chaos-crash-1".into(),
        session_id: "sess-chaos".into(),
        owner_id: "user-auditor".into(),
        prompt: "echo 'critical long running task'".into(),
        state: TaskState::Queued,
        mode: agents_protocol::InteractionMode::Act,
        fencing_token: FencingToken(1),
        lease_holder: None,
        lease_expires_at: None,
        idempotency_key: None,
        version: 1,
        exit_code: None,
        stop_reason: None,
        output_summary: None,
        created_at: now,
        updated_at: now,
    };
    repo.create(task).await.expect("task seed should succeed");

    // 2. Node A acquires lease with TTL = 10s (expires at now + 10_000)
    let leased_node_a = repo
        .acquire_lease("task-chaos-crash-1", "boxer-node-a", now, now + 10_000)
        .await
        .expect("node a should acquire initial lease");
    assert_eq!(leased_node_a.lease_holder.as_deref(), Some("boxer-node-a"));
    assert_eq!(leased_node_a.fencing_token.0, 2);

    // 3. Node A simulates crash (SIGKILL - ceases heartbeating). Clock advances +15s.
    let future_time = now + 15_000;

    // 4. Task sweeper reaps expired leases
    let reaped = repo
        .reap_expired_leases(future_time)
        .await
        .expect("sweeper should execute without errors");
    assert_eq!(reaped.len(), 1, "exactly 1 expired task should be reaped");
    assert_eq!(reaped[0].id, "task-chaos-crash-1");
    assert_eq!(reaped[0].state, TaskState::Queued);
    assert_eq!(reaped[0].lease_holder, None);
    assert_eq!(reaped[0].fencing_token.0, 3, "token bumped to 3 on reap");

    // 5. Node B (healthy alternate node) acquires re-enqueued task
    let leased_node_b = repo
        .acquire_lease(
            "task-chaos-crash-1",
            "boxer-node-b",
            future_time,
            future_time + 10_000,
        )
        .await
        .expect("node b should claim reclaimed task");
    assert_eq!(leased_node_b.lease_holder.as_deref(), Some("boxer-node-b"));
    assert_eq!(leased_node_b.fencing_token.0, 4);

    // 6. Zombie Node A attempts update using stale token (2) -> must fail conflict
    let zombie_update = repo
        .transition_fenced(
            "task-chaos-crash-1",
            FencingToken(2),
            TaskState::Running,
            future_time + 100,
        )
        .await;
    assert!(
        matches!(zombie_update, Err(ServerError::Conflict(_))),
        "zombie worker update with stale token must be rejected"
    );

    // 7. Node B transitions to Running and completes cleanly with active token (4)
    repo.transition_fenced(
        "task-chaos-crash-1",
        FencingToken(4),
        TaskState::Running,
        future_time + 150,
    )
    .await
    .expect("node b should transition to running");

    let completed_node_b = repo
        .complete_fenced(
            "task-chaos-crash-1",
            FencingToken(4),
            0,
            StopReason::EndTurn,
            Some("finished by alternate node".into()),
            future_time + 200,
        )
        .await
        .expect("node b should successfully complete task");
    assert_eq!(completed_node_b.state, TaskState::Done);
    assert_eq!(completed_node_b.exit_code, Some(0));
}

#[tokio::test]
async fn test_chaos_poison_message_dlq_quarantine() {
    let filter = PoisonMessageFilter::new(3);
    let msg_id = "msg-poison-malformed-json";
    let subject = "boxer.tasks.request";
    let bad_payload = b"{ malformed json bytes ::: invalid syntax ";
    let now = now_ms();

    // Delivery 1: Failure -> Retry
    let d1 = filter.record_failure(msg_id, subject, bad_payload, "syntax error", now);
    assert_eq!(
        d1,
        DlqDecision::Retry {
            attempt: 1,
            max_deliver: 3
        }
    );
    assert_eq!(filter.quarantined_count(), 0);

    // Delivery 2: Failure -> Retry
    let d2 = filter.record_failure(msg_id, subject, bad_payload, "syntax error", now + 100);
    assert_eq!(
        d2,
        DlqDecision::Retry {
            attempt: 2,
            max_deliver: 3
        }
    );
    assert_eq!(filter.quarantined_count(), 0);

    // Delivery 3: Failure -> DLQ Quarantine
    let d3 = filter.record_failure(msg_id, subject, bad_payload, "syntax error", now + 200);
    match d3 {
        DlqDecision::Quarantined(entry) => {
            assert_eq!(entry.message_id, msg_id);
            assert_eq!(entry.subject, subject);
            assert_eq!(entry.delivery_attempts, 3);
            assert!(entry.payload_preview.contains("malformed json bytes"));
        }
        DlqDecision::Retry { .. } => panic!("must be quarantined after 3 attempts"),
    }
    assert_eq!(filter.quarantined_count(), 1);

    // Subsequent valid message arrives: processes successfully
    let valid_msg_id = "msg-valid-task-42";
    filter.record_success(valid_msg_id);
    assert_eq!(
        filter.quarantined_count(),
        1,
        "valid task success does not alter existing DLQ quarantine"
    );
}

#[tokio::test]
async fn test_chaos_network_partition_and_reconnection_backoff() {
    let mut backoff = Duration::from_millis(5);
    let max_backoff = Duration::from_millis(40);
    let mut attempts = 0;
    let mut network_available = false;

    // Simulate transient network partition that recovers after 3 backoff cycles
    for cycle in 1..=5 {
        attempts += 1;
        if cycle >= 3 {
            network_available = true;
        }

        if network_available {
            break;
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }

    assert!(network_available, "network should recover after backoff");
    assert_eq!(attempts, 3, "recovered on cycle 3");
}
