#![allow(clippy::unwrap_used)]

use std::{sync::Arc, time::Duration};

use pulsar_boxer::{
    error::BoxerError,
    leases::BoxFuture,
    runtime::{
        CodemodeScript, CodemodeStatement, DefaultLoopHooks, ModelTurnResponse, NativeAgentRuntime,
        NativeRuntimeConfig, OutputSchemaContract, RetryPolicy, RuntimeTool, SandboxedCodemodeVm,
        ScriptedModelClient, SessionEntry, SessionTree,
    },
    sandbox::{FileMutationQueue, MutationQueueError},
};
use pulsar_protocol::{
    EventEnvelope, FollowUpMessage, InteractionMode, PermissionMask, SteeringMessage, StopReason,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

struct EchoReadTool;

impl RuntimeTool for EchoReadTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn is_mutating(&self) -> bool {
        false
    }

    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>> {
        Box::pin(async move {
            let p = input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Ok(format!("contents_of:{p}"))
        })
    }
}

#[tokio::test]
async fn sandboxed_codemode_vm_executes_tools_and_enforces_limits() {
    let mut vm = SandboxedCodemodeVm::new().with_limits(Duration::from_millis(80), 120);
    vm.register_tool(Arc::new(EchoReadTool));

    let ok_script = CodemodeScript {
        statements: vec![CodemodeStatement {
            step_id: "s1".to_owned(),
            tool_name: "read_file".to_owned(),
            args: json!({"path": "Cargo.toml"}),
        }],
        simulate_infinite_loop: false,
    };
    let rep = vm.execute_script(&ok_script).await.unwrap();
    assert_eq!(rep.steps_executed, 1);
    assert!(!rep.timed_out);
    assert!(rep.consolidated_output.contains("contents_of:Cargo.toml"));

    let host_escape = CodemodeScript {
        statements: vec![CodemodeStatement {
            step_id: "s2".to_owned(),
            tool_name: "net_socket".to_owned(),
            args: json!({}),
        }],
        simulate_infinite_loop: false,
    };
    assert!(vm.execute_script(&host_escape).await.is_err());

    let infinite_script = CodemodeScript {
        statements: Vec::new(),
        simulate_infinite_loop: true,
    };
    let timeout_rep = vm.execute_script(&infinite_script).await.unwrap();
    assert!(timeout_rep.timed_out);
    assert_eq!(
        timeout_rep.truncation_locator.as_deref(),
        Some("codemode://timeout/terminated")
    );

    let huge_script = CodemodeScript {
        statements: vec![
            CodemodeStatement {
                step_id: "s1".to_owned(),
                tool_name: "read_file".to_owned(),
                args: json!({"path": "very_long_file_path_alpha_beta_gamma_delta_epsilon_1"}),
            },
            CodemodeStatement {
                step_id: "s2".to_owned(),
                tool_name: "read_file".to_owned(),
                args: json!({"path": "very_long_file_path_alpha_beta_gamma_delta_epsilon_2"}),
            },
        ],
        simulate_infinite_loop: false,
    };
    let trunc_rep = vm.execute_script(&huge_script).await.unwrap();
    assert!(trunc_rep.truncated);
    assert!(trunc_rep.truncation_locator.is_some());
}

#[tokio::test]
async fn validated_structured_output_corrective_retries_and_exhaustion() {
    let model_recover = ScriptedModelClient::new(vec![
        Ok(ModelTurnResponse {
            assistant_text: "Here is broken json: {not_valid}".to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 10,
        }),
        Ok(ModelTurnResponse {
            assistant_text: r#"Sure! {"status": "ok", "score": 99}"#.to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 12,
        }),
    ]);

    let runtime = NativeAgentRuntime::new(model_recover, Arc::new(DefaultLoopHooks));
    let mut tree = SessionTree::new("ses-schema-1");
    tree.append(SessionEntry::UserMessage {
        text: "Return status and score".to_owned(),
    });
    let (ev_tx, _ev_rx) = mpsc::channel::<EventEnvelope>(32);
    let (_st_tx, st_rx) = mpsc::channel::<SteeringMessage>(8);
    let (_fu_tx, fu_rx) = mpsc::channel::<FollowUpMessage>(8);
    let (_c_tx, c_rx) = watch::channel(false);

    let cfg = NativeRuntimeConfig {
        task_id: "tsk-schema-1".to_owned(),
        endpoint_id: "default".to_owned(),
        mode: InteractionMode::Act,
        permissions: PermissionMask {
            can_read_fs: true,
            can_write_fs: true,
            can_exec_shell: false,
            can_network: false,
            allowed_tools: Vec::new(),
        },
        max_turns: 5,
        compact_keep_tail: 10,
        retry_policy: RetryPolicy::default(),
        output_contract: Some(OutputSchemaContract {
            required_fields: vec!["status".to_owned(), "score".to_owned()],
        }),
    };

    let outcome = runtime
        .run(&cfg, &mut tree, ev_tx, st_rx, fu_rx, c_rx)
        .await
        .unwrap();
    let structured = outcome.structured_json.unwrap();
    assert_eq!(structured["status"], "ok");
    assert_eq!(structured["score"], 99);
    assert_eq!(outcome.retry_audit.len(), 1);

    let model_exhaust = ScriptedModelClient::new(vec![
        Ok(ModelTurnResponse {
            assistant_text: "bad 1".to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 5,
        }),
        Ok(ModelTurnResponse {
            assistant_text: "bad 2".to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 5,
        }),
        Ok(ModelTurnResponse {
            assistant_text: "bad 3".to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 5,
        }),
    ]);
    let runtime_ex = NativeAgentRuntime::new(model_exhaust, Arc::new(DefaultLoopHooks));
    let mut tree_ex = SessionTree::new("ses-schema-2");
    let (ev_tx2, _) = mpsc::channel(32);
    let (_, st_rx2) = mpsc::channel(8);
    let (_, fu_rx2) = mpsc::channel(8);
    let (_, c_rx2) = watch::channel(false);

    let err = runtime_ex
        .run(&cfg, &mut tree_ex, ev_tx2, st_rx2, fu_rx2, c_rx2)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("schema_validation_exhausted"));
}

#[tokio::test]
async fn file_mutation_queue_realpath_serialization_optimistic_conflict_and_reconcile() {
    let dir = std::env::temp_dir().join(format!("pulsar-mutq-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join("shared.txt");

    let queue = FileMutationQueue::new();
    let r1 = queue
        .mutate_optimistic(&file_path, None, b"v1")
        .await
        .unwrap();
    assert_eq!(r1.bytes_written, 2);

    let stale_res = queue
        .mutate_optimistic(&file_path, Some("0000000000000000"), b"v2")
        .await;
    assert!(matches!(
        stale_res,
        Err(MutationQueueError::ConflictStaleBase { .. })
    ));

    let r2 = queue
        .mutate_optimistic(&file_path, Some(&r1.new_sha256), b"v2-updated")
        .await
        .unwrap();
    assert_eq!(r2.previous_sha256, r1.new_sha256);

    let mut handles = Vec::new();
    for idx in 0..10_usize {
        let q = queue.clone();
        let p = dir.join(format!("concurrent_{idx}.txt"));
        handles.push(tokio::spawn(async move {
            q.mutate_optimistic(&p, None, format!("payload-{idx}").as_bytes())
                .await
                .unwrap()
        }));
    }
    for h in handles {
        let rec = h.await.unwrap();
        assert!(rec.bytes_written > 0);
    }

    let state = queue.reconcile_file_state(&file_path).await.unwrap();
    assert!(state.exists);
    assert_eq!(state.current_sha256, r2.new_sha256);
    assert_eq!(state.size_bytes, 10);

    let _ = std::fs::remove_dir_all(&dir);
}
