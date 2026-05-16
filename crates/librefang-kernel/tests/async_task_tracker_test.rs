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
async fn register_dedupes_workflow_kind_against_existing_run_id() {
    // #5033 review fix: a second `register_async_task` for the same
    // `TaskKind::Workflow { run_id }` must return the existing handle,
    // not mint a fresh `TaskId` that silently orphans on completion.
    let kernel =
        LibreFangKernel::boot_with_config(test_config("register-dedupe-workflow")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());
    let run_id = WorkflowRunId(Uuid::new_v4());

    let first = kernel.register_async_task(agent_id, session_id, TaskKind::Workflow { run_id });
    let second = kernel.register_async_task(agent_id, session_id, TaskKind::Workflow { run_id });

    assert_eq!(
        first.id, second.id,
        "duplicate registration for the same run_id must return the existing handle"
    );
    assert_eq!(
        kernel.pending_async_task_count(),
        1,
        "registry must hold exactly one entry for the deduped pair"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn register_dedupes_delegation_kind_against_existing_target_and_hash() {
    // Mirror of the workflow case for delegation kinds — same
    // (agent_id, prompt_hash) is treated as the same task.
    let kernel =
        LibreFangKernel::boot_with_config(test_config("register-dedupe-delegation")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());
    let target = AgentId(Uuid::new_v4());

    let kind = TaskKind::Delegation {
        agent_id: target,
        prompt_hash: "sha256:deadbeef".to_string(),
    };
    let first = kernel.register_async_task(agent_id, session_id, kind.clone());
    let second = kernel.register_async_task(agent_id, session_id, kind);

    assert_eq!(first.id, second.id);
    assert_eq!(kernel.pending_async_task_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn register_does_not_dedupe_distinct_delegations_to_same_target() {
    // Different `prompt_hash` for the same target agent is a distinct
    // delegation — two registrations, two handles.
    let kernel = LibreFangKernel::boot_with_config(test_config("register-distinct")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());
    let target = AgentId(Uuid::new_v4());

    let first = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Delegation {
            agent_id: target,
            prompt_hash: "sha256:aaaa".to_string(),
        },
    );
    let second = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Delegation {
            agent_id: target,
            prompt_hash: "sha256:bbbb".to_string(),
        },
    );

    assert_ne!(
        first.id, second.id,
        "different prompt hashes must mint distinct task ids"
    );
    assert_eq!(kernel.pending_async_task_count(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn register_dedupe_is_cross_session_for_delegation_kind() {
    // #5033 re-review: the dedupe match key is `kind`-structural and
    // ignores the caller `(agent_id, session_id)`. Two distinct
    // sessions that register the same `(target_agent, prompt_hash)`
    // delegation share a single registry handle. This pins that
    // behaviour as intentional: callers that need per-session isolation
    // must salt their `prompt_hash` themselves. The completion event
    // for the shared handle lands on whichever `(agent_id, session_id)`
    // attached the registry entry first (the second caller's session
    // does **not** receive a duplicate event).
    let kernel = LibreFangKernel::boot_with_config(test_config("register-cross-session")).unwrap();
    let caller_a_agent = AgentId(Uuid::new_v4());
    let caller_a_session = SessionId(Uuid::new_v4());
    let caller_b_agent = AgentId(Uuid::new_v4());
    let caller_b_session = SessionId(Uuid::new_v4());
    let target = AgentId(Uuid::new_v4());

    let kind = TaskKind::Delegation {
        agent_id: target,
        prompt_hash: "sha256:shared".to_string(),
    };
    let first = kernel.register_async_task(caller_a_agent, caller_a_session, kind.clone());
    let second = kernel.register_async_task(caller_b_agent, caller_b_session, kind);

    assert_eq!(
        first.id, second.id,
        "cross-session registration with the same (target, prompt_hash) \
         must return the existing handle — callers that need isolation \
         must salt prompt_hash"
    );
    assert_eq!(
        kernel.pending_async_task_count(),
        1,
        "registry must hold exactly one entry across the two sessions"
    );

    // Confirm the stored entry retains caller A's (agent, session) —
    // caller B silently shares it. This is the intentional contract
    // (see `register_async_task` docstring "Cross-caller dedupe
    // semantics"). On completion, the event will route to caller A's
    // session only; caller B's session does not receive a duplicate.
    let mut rx_a = attach_injection_receiver(&kernel, caller_a_agent, caller_a_session);
    let mut rx_b = attach_injection_receiver(&kernel, caller_b_agent, caller_b_session);
    let delivered = kernel
        .complete_async_task(
            first.id,
            TaskStatus::Completed(json!({"output": "shared-result"})),
        )
        .await
        .expect("complete_async_task ok");
    assert!(delivered, "first caller's receiver accepts the signal");

    assert!(
        rx_a.try_recv().is_ok(),
        "caller A's session must receive the completion event"
    );
    assert!(
        rx_b.try_recv().is_err(),
        "caller B's session must NOT receive a duplicate — the cross-session \
         share routes completion to the first caller only"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_falls_through_to_wake_idle_when_injection_channel_is_full() {
    // #5033 re-review: the `Backpressure(Full)` arm in
    // `complete_async_task` (`task_registry.rs:269-278`) must fall
    // through to the wake-idle path rather than bubbling the error.
    // Pin that contract with a capacity-1 injection channel saturated
    // by an unrelated signal, so the kernel's `try_send` for the
    // completion event gets `TrySendError::Full` and the kernel takes
    // the wake-idle branch.
    use std::sync::Arc;

    let kernel = Arc::new(
        LibreFangKernel::boot_with_config(test_config("complete-backpressure-fallthrough"))
            .unwrap(),
    );
    // `self_handle` is what lets the wake-idle path acquire the
    // kernel Arc and spawn a turn — without it the fallthrough would
    // return `Ok(false)` (entry drained, no spawn) which is a weaker
    // assertion. Set it so the test pins the full
    // backpressure → wake-idle-spawn → Ok(true) chain.
    kernel.set_self_handle();

    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());

    // Capacity-1 channel, pre-filled with one stray signal so the
    // tracker's `try_send` will hit `TrySendError::Full`. Hold `_rx`
    // alive (do NOT drop) so the channel stays open — closing it
    // would route the kernel down the `Closed` branch instead.
    let (tx, _rx) = tokio::sync::mpsc::channel::<AgentLoopSignal>(1);
    // Fill the single slot. We send a dummy `TaskCompleted` signal
    // (any variant works — the kernel only cares about send-side
    // capacity, not what is already queued).
    let stray_event = librefang_types::task::TaskCompletionEvent {
        handle: librefang_types::task::TaskHandle {
            id: librefang_types::task::TaskId::new(),
            kind: TaskKind::Workflow {
                run_id: WorkflowRunId(Uuid::new_v4()),
            },
            started_at: chrono::Utc::now(),
        },
        status: TaskStatus::Cancelled,
        completed_at: chrono::Utc::now(),
    };
    tx.try_send(AgentLoopSignal::TaskCompleted { event: stray_event })
        .expect("capacity-1 channel accepts first send");
    kernel
        .injection_senders_ref()
        .insert((agent_id, session_id), tx);

    // Register a real task and complete it. The injection channel
    // for this (agent, session) is now full, so
    // `inject_task_completion_signal` returns
    // `Err(KernelError::Backpressure(_))`. The `complete_async_task`
    // `Backpressure(_)` arm logs and sets `injected = false`, which
    // sends control through to `spawn_wake_idle_turn`. With
    // `self_handle` set, that spawn succeeds and the function
    // returns `Ok(true)`.
    let handle = kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );
    assert_eq!(kernel.pending_async_task_count(), 1);

    let delivered = kernel
        .complete_async_task(handle.id, TaskStatus::Completed(json!({"k": "v"})))
        .await
        .expect(
            "complete_async_task must return Ok even when the injection \
             channel is full — the Backpressure arm falls through, it does \
             not propagate the error",
        );

    assert!(
        delivered,
        "Backpressure(Full) on the mid-turn channel must fall through \
         to wake-idle; with self_handle set, wake-idle reports delivered=true"
    );
    // Delete-on-delivery contract: the registry entry is consumed
    // regardless of which delivery path fired.
    assert_eq!(kernel.pending_async_task_count(), 0);
    assert!(kernel.lookup_async_task(handle.id).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_synthesizes_failed_event_for_matching_pending_workflow() {
    // #5033 review fix: `synthesize_task_failures_for_recovered_runs`
    // walks the registry for `TaskKind::Workflow { run_id }` entries
    // matching the recovered set, drains them, and synthesizes a
    // `TaskStatus::Failed("...interrupted by daemon restart")` event.
    // Pin both the drain-and-inject behaviour and the bytes of the
    // synthesized message.
    let kernel = LibreFangKernel::boot_with_config(test_config("recovery-synthesize")).unwrap();
    let agent_id = AgentId(Uuid::new_v4());
    let session_id = SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&kernel, agent_id, session_id);

    let run_id = WorkflowRunId(Uuid::new_v4());
    let handle = kernel.register_async_task(agent_id, session_id, TaskKind::Workflow { run_id });
    assert_eq!(kernel.pending_async_task_count(), 1);

    // Drive the recovery hook directly. In production this is called
    // from boot.rs after `recover_stale_running_runs` returns the
    // demoted run ids.
    kernel.synthesize_task_failures_for_recovered_runs(&[run_id]);

    // Registry entry was drained.
    assert_eq!(kernel.pending_async_task_count(), 0);
    assert!(kernel.lookup_async_task(handle.id).is_none());

    // A `Failed` event with the canonical restart message arrived on
    // the injection channel.
    let signal = rx.try_recv().expect("Failed signal queued");
    match signal {
        AgentLoopSignal::TaskCompleted { event } => {
            assert_eq!(event.handle.id, handle.id);
            match &event.status {
                TaskStatus::Failed(msg) => {
                    assert_eq!(
                        msg, "workflow run interrupted by daemon restart",
                        "recovery message bytes must match the documented contract"
                    );
                }
                other => panic!("expected Failed status, got {other:?}"),
            }
        }
        other => panic!("expected TaskCompleted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_noop_when_no_pending_task_matches_recovered_run() {
    // Sanity: if no registry entry references the recovered run id,
    // the helper is a no-op and does not panic.
    let kernel = LibreFangKernel::boot_with_config(test_config("recovery-noop")).unwrap();
    let stranger = WorkflowRunId(Uuid::new_v4());
    kernel.synthesize_task_failures_for_recovered_runs(&[stranger]);
    assert_eq!(kernel.pending_async_task_count(), 0);
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
