//! Integration tests for the async task tracker registry (#4983 step 2).
//!
//! Exercises the kernel-side `register_async_task` /
//! `complete_async_task` pair without booting a full workflow engine —
//! the registry is the integration surface step 2 owns, and the
//! workflow-engine wiring is tested separately in
//! `workflow_integration_test.rs` once the runtime side lands in step
//! 3.
//!
//! Tests cover:
//! - registry insert / lookup / delete on completion
//! - workflow-kind completion injects a `TaskCompleted` signal into the
//!   originating session
//! - delegation-kind completion injects with the right `TaskKind`
//! - mid-turn delivery: signal arrives on the live injection channel
//! - idle delivery: when no receiver is attached, completion still
//!   removes the registry entry (step 3 adds wake-idle)
//! - double-delivery is a no-op (idempotency for retry races)

use librefang_kernel::EventSubsystemApi;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use librefang_types::task::{TaskKind, TaskStatus, WorkflowRunId};
use librefang_types::tool::AgentLoopSignal;
use serde_json::json;
use uuid::Uuid;

fn test_config(name: &str) -> KernelConfig {
    let tmp = std::env::temp_dir().join(format!("librefang-async-task-test-{name}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    KernelConfig {
        home_dir: tmp.clone(),
        data_dir: tmp.join("data"),
        default_model: DefaultModelConfig {
            provider: "groq".to_string(),
            model: "llama-3.3-70b-versatile".to_string(),
            api_key_env: "GROQ_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    }
}

/// Manually wire up an injection sender/receiver pair for `(agent, session)`
/// without going through the agent loop. The tracker's completion path
/// reuses `events.injection_senders`; the test acts as the receiver.
fn attach_injection_receiver(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    session_id: SessionId,
) -> tokio::sync::mpsc::Receiver<AgentLoopSignal> {
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentLoopSignal>(8);
    kernel
        .injection_senders_ref()
        .insert((agent_id, session_id), tx);
    rx
}

#[tokio::test(flavor = "multi_thread")]
async fn register_inserts_into_registry_and_returns_handle() {
    let kernel = LibreFangKernel::boot_with_config(test_config("register-insert")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());

    assert_eq!(kernel.pending_async_task_count(), 0);

    let handle = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );

    assert_eq!(kernel.pending_async_task_count(), 1);
    let looked_up = kernel
        .lookup_async_task(handle.id)
        .expect("registered task should be looked up");
    assert_eq!(looked_up.id, handle.id);
    match looked_up.kind {
        TaskKind::Workflow { .. } => {}
        other => panic!("expected Workflow kind, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_workflow_task_injects_signal_into_originating_session() {
    let kernel = LibreFangKernel::boot_with_config(test_config("workflow-inject")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&kernel, agent_id, session_id);

    let run_id = WorkflowRunId(Uuid::new_v4());
    let handle = kernel.register_async_task(agent_id, session_id, TaskKind::Workflow { run_id });

    let delivered = kernel
        .complete_async_task(
            handle.id,
            TaskStatus::Completed(json!({"output": "report.md"})),
        )
        .await
        .expect("complete_async_task ok");
    assert!(delivered, "live receiver should accept the signal");

    // Registry entry was removed on delivery (cleanup semantics from step 1).
    assert_eq!(kernel.pending_async_task_count(), 0);
    assert!(kernel.lookup_async_task(handle.id).is_none());

    // The signal arrived.
    let signal = rx
        .try_recv()
        .expect("TaskCompleted signal should be queued");
    match signal {
        AgentLoopSignal::TaskCompleted { event } => {
            assert_eq!(event.handle.id, handle.id);
            match event.handle.kind {
                TaskKind::Workflow { run_id: r } => assert_eq!(r, run_id),
                other => panic!("expected Workflow kind in injected event, got {other:?}"),
            }
            match event.status {
                TaskStatus::Completed(value) => {
                    assert_eq!(value, json!({"output": "report.md"}));
                }
                other => panic!("expected Completed status, got {other:?}"),
            }
        }
        other => panic!("expected AgentLoopSignal::TaskCompleted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_delegation_task_injects_signal_with_delegation_kind() {
    let kernel = LibreFangKernel::boot_with_config(test_config("delegation-inject")).unwrap();
    let sender_agent = AgentId(Uuid::new_v4());
    let sender_session = SessionId(Uuid::new_v4());
    let target_agent = AgentId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&kernel, sender_agent, sender_session);

    let handle = kernel.register_async_task(
        sender_agent,
        sender_session,
        TaskKind::Delegation {
            agent_id: target_agent,
            prompt_hash: "sha256:dead".to_string(),
        },
    );

    let delivered = kernel
        .complete_async_task(
            handle.id,
            TaskStatus::Failed("upstream agent rejected the request".to_string()),
        )
        .await
        .expect("complete_async_task ok");
    assert!(delivered);

    let signal = rx
        .try_recv()
        .expect("TaskCompleted signal should be queued");
    match signal {
        AgentLoopSignal::TaskCompleted { event } => {
            match event.handle.kind {
                TaskKind::Delegation {
                    agent_id,
                    prompt_hash,
                } => {
                    assert_eq!(agent_id, target_agent);
                    assert_eq!(prompt_hash, "sha256:dead");
                }
                other => panic!("expected Delegation kind, got {other:?}"),
            }
            match event.status {
                TaskStatus::Failed(msg) => assert!(msg.contains("upstream agent rejected")),
                other => panic!("expected Failed status, got {other:?}"),
            }
        }
        other => panic!("expected AgentLoopSignal::TaskCompleted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_with_no_attached_receiver_still_removes_entry() {
    // Boot WITHOUT calling `set_self_handle` — emulates a kernel
    // mid-boot where the Arc has not been wrapped yet. The wake-idle
    // path in step 3 needs `self_handle` to spawn a turn; when it is
    // unset, the kernel returns `Ok(false)` (no turn spawned) and the
    // entry is still removed (delete-on-delivery contract from step 1).
    let kernel = LibreFangKernel::boot_with_config(test_config("idle-cleanup")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());

    let handle = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );
    assert_eq!(kernel.pending_async_task_count(), 1);

    let delivered = kernel
        .complete_async_task(handle.id, TaskStatus::Cancelled)
        .await
        .expect("complete_async_task ok");
    assert!(
        !delivered,
        "self_handle unset → wake-idle cannot spawn a turn"
    );

    // Entry was still removed (delete-on-delivery contract).
    assert_eq!(kernel.pending_async_task_count(), 0);
    assert!(kernel.lookup_async_task(handle.id).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn wake_idle_path_returns_true_when_self_handle_is_set() {
    // With `set_self_handle` called (the runtime steady-state), the
    // wake-idle path acquires the kernel Arc and spawns a `tokio::task`
    // that drives a turn via `send_message_full`. The function itself
    // returns `Ok(true)` regardless of the spawned turn's outcome —
    // failure to actually drive the agent (e.g. the AgentId isn't
    // registered) is logged but does not block the workflow that
    // called `complete_async_task`. The agent-loop-side proof that
    // the spawned turn lands properly lives in the `librefang-api`
    // integration tests exercising the full `TestServer` path.
    use std::sync::Arc;

    let kernel =
        Arc::new(LibreFangKernel::boot_with_config(test_config("idle-wake-spawn")).unwrap());
    kernel.set_self_handle();

    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());

    let handle = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );

    let delivered = kernel
        .complete_async_task(handle.id, TaskStatus::Completed(json!({"output": "done"})))
        .await
        .expect("complete_async_task ok");
    assert!(
        delivered,
        "wake-idle path with self_handle set spawns a turn and reports delivered=true"
    );
    // Registry entry still removed even though the spawned turn may
    // fail downstream because the AgentId isn't registered.
    assert_eq!(kernel.pending_async_task_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn double_completion_is_a_noop_on_second_call() {
    let kernel = LibreFangKernel::boot_with_config(test_config("double-complete")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&kernel, agent_id, session_id);

    let handle = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );

    // First completion delivers.
    let first = kernel
        .complete_async_task(handle.id, TaskStatus::Completed(json!({"k": "v"})))
        .await
        .expect("first complete ok");
    assert!(first);

    // Second completion finds no entry; returns Ok(false).
    let second = kernel
        .complete_async_task(handle.id, TaskStatus::Cancelled)
        .await
        .expect("second complete ok");
    assert!(!second);

    // Only ONE signal landed on the channel, despite two calls.
    let _ = rx.try_recv().expect("first signal");
    assert!(
        rx.try_recv().is_err(),
        "no second signal should have been injected"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_unknown_task_id_returns_ok_false() {
    let kernel = LibreFangKernel::boot_with_config(test_config("unknown-id")).unwrap();
    let bogus = librefang_types::task::TaskId::new();

    let delivered = kernel
        .complete_async_task(bogus, TaskStatus::Cancelled)
        .await
        .expect("complete_async_task ok");
    assert!(!delivered, "unknown id should return Ok(false), no panic");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_run_id_canonical_definition_lives_in_types_crate() {
    // Sanity check on the step-2 migration: the kernel's
    // `workflow::WorkflowRunId` and the canonical
    // `librefang_types::task::WorkflowRunId` are the same nominal type
    // (re-exported), not two parallel newtypes.
    let canonical: WorkflowRunId = WorkflowRunId(Uuid::nil());
    let via_kernel: librefang_kernel::workflow::WorkflowRunId = canonical;
    assert_eq!(via_kernel.0, Uuid::nil());
}
