//! Integration tests for the built-in Task Board assignee wake (#6728).
//!
//! A task addressed to an agent used to reach that agent only if an operator had separately registered a matching `TaskPosted` trigger; with none, the task sat `pending` with nothing in the log.
//! These tests pin the delivery rule against a real booted kernel:
//!
//! 1. `assigned_task_wakes_an_agent_with_no_trigger` — the gap itself.
//! 2. `assigned_task_wakes_an_agent_addressed_by_name` — `assigned_to` holds either identity form, and both must reach the same agent.
//! 3. `operator_trigger_suppresses_the_builtin_wake` — no double wake.
//! 4. `dormant_trigger_does_not_suppress_the_builtin_wake` — a disabled record is a gap to fill, not a decision to stay silent.
//! 5. `assignee_wake_disabled_globally_produces_no_wake` — the opt-out.
//! 6. `per_agent_override_beats_the_global_default` — manifest wins.
//! 7. `agent_without_task_claim_is_not_woken` — an agent whose declared tool list withholds `task_claim`.
//! 8. `unrestricted_agent_is_woken` — an empty `capabilities.tools` means unrestricted, not "no tools".
//! 9. `agent_denied_task_claim_by_list_is_not_woken` — `tool_allowlist` / `tool_blocklist` withhold it just as effectively.
//! 10. `unassigned_task_wakes_nobody` — pool tasks are claimable, not routed.
//! 11. `only_the_addressed_agent_is_woken` — with several eligible agents present, the wake reaches the assignee and nobody else.
//!
//! The seam is `publish_typed_event`'s return value: it is the exact list the dispatcher consumes, so asserting on it tests the wake without needing an LLM behind `send_message_full`.

use librefang_kernel::triggers::{TriggerMatchSource, TriggerPatch, TriggerPattern};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, ManifestCapabilities};
use librefang_types::event::{Event, EventPayload, EventTarget, SystemEvent};

/// Manifest for an agent that can claim its own tasks — the population the
/// built-in wake targets.
fn worker_manifest(name: &str) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        capabilities: ManifestCapabilities {
            tools: vec!["task_claim".to_string(), "task_complete".to_string()],
            ..ManifestCapabilities::default()
        },
        ..AgentManifest::default()
    }
}

fn task_posted(assigned_to: Option<&str>) -> Event {
    Event::new(
        AgentId::new(),
        EventTarget::Broadcast,
        EventPayload::System(SystemEvent::TaskPosted {
            task_id: "task-1".to_string(),
            title: "Summarize the release notes".to_string(),
            assigned_to: assigned_to.map(str::to_string),
            created_by: Some("orchestrator".to_string()),
        }),
    )
}

/// The reported gap: an agent with no trigger at all still gets woken for a
/// task addressed to it, and the match is attributed to the built-in path
/// rather than to a record that does not exist.
#[tokio::test(flavor = "multi_thread")]
async fn assigned_task_wakes_an_agent_with_no_trigger() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("worker"))
        .expect("spawn must succeed");

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;

    assert_eq!(matches.len(), 1, "the assignee must be woken exactly once");
    assert_eq!(matches[0].agent_id, worker);
    assert_eq!(
        matches[0].source,
        TriggerMatchSource::TaskBoardAssigneeWake {
            task_id: "task-1".to_string()
        },
    );
    assert!(
        matches[0].message.contains("task_claim"),
        "the wake must tell the agent how to pick the task up, got: {}",
        matches[0].message,
    );
    assert!(
        test.state.kernel.list_triggers(Some(worker)).is_empty(),
        "the wake must not leave a trigger record behind",
    );
}

/// `substrate::task_claim` matches `assigned_to` by UUID *or* display name,
/// so a task addressed by name has to wake the same agent — otherwise the
/// claimable-but-unreachable case survives the fix.
#[tokio::test(flavor = "multi_thread")]
async fn assigned_task_wakes_an_agent_addressed_by_name() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("worker"))
        .expect("spawn must succeed");

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some("worker")))
        .await;

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].agent_id, worker);
}

/// An operator who declared their own wake keeps it: exactly one match, and
/// it is theirs, so the prompt / session mode / routing they configured is
/// what runs.
#[tokio::test(flavor = "multi_thread")]
async fn operator_trigger_suppresses_the_builtin_wake() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("worker"))
        .expect("spawn must succeed");
    let trigger_id = test
        .state
        .kernel
        .register_trigger_with_target(
            worker,
            TriggerPattern::TaskPosted {
                assignee_match: Some("self".to_string()),
            },
            "operator wake: {{event}}".to_string(),
            0,
            None,
            None,
            None,
            None,
        )
        .expect("register must succeed");

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;

    assert_eq!(matches.len(), 1, "the assignee must not be woken twice");
    assert_eq!(
        matches[0].source,
        TriggerMatchSource::Registered(trigger_id)
    );
    assert!(matches[0].message.starts_with("operator wake:"));
}

/// The outage shape: a trigger that exists but cannot fire must not keep the
/// assignee unreachable. Treating any record as coverage is what let tasks
/// sit `pending` indefinitely.
#[tokio::test(flavor = "multi_thread")]
async fn dormant_trigger_does_not_suppress_the_builtin_wake() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("worker"))
        .expect("spawn must succeed");
    let trigger_id = test
        .state
        .kernel
        .register_trigger_with_target(
            worker,
            TriggerPattern::TaskPosted {
                assignee_match: Some("self".to_string()),
            },
            "operator wake: {{event}}".to_string(),
            0,
            None,
            None,
            None,
            None,
        )
        .expect("register must succeed");
    assert!(test
        .state
        .kernel
        .update_trigger(
            trigger_id,
            TriggerPatch {
                enabled: Some(false),
                ..TriggerPatch::default()
            },
        )
        .is_some_and(|t| !t.enabled));

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;

    assert_eq!(matches.len(), 1, "the built-in wake must take over");
    assert_eq!(
        matches[0].source,
        TriggerMatchSource::TaskBoardAssigneeWake {
            task_id: "task-1".to_string()
        },
    );
}

/// The global opt-out is honoured — and it is the only thing that silences
/// the path.
#[tokio::test(flavor = "multi_thread")]
async fn assignee_wake_disabled_globally_produces_no_wake() {
    let test = TestAppState::with_builder(
        MockKernelBuilder::new().with_config(|cfg| cfg.task_board.assignee_wake = false),
    );
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("worker"))
        .expect("spawn must succeed");

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;

    assert!(
        matches.is_empty(),
        "assignee_wake = false must suppress the built-in wake"
    );
}

/// Per-agent override resolves against the global default in both
/// directions, so one agent can opt out of an installation-wide default —
/// or into it.
#[tokio::test(flavor = "multi_thread")]
async fn per_agent_override_beats_the_global_default() {
    // Global ON, agent OFF.
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let mut manifest = worker_manifest("opted-out");
    manifest.assignee_wake = Some(false);
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn must succeed");
    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;
    assert!(
        matches.is_empty(),
        "manifest opt-out must win over global ON"
    );

    // Global OFF, agent ON.
    let test = TestAppState::with_builder(
        MockKernelBuilder::new().with_config(|cfg| cfg.task_board.assignee_wake = false),
    );
    let mut manifest = worker_manifest("opted-in");
    manifest.assignee_wake = Some(true);
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn must succeed");
    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;
    assert_eq!(matches.len(), 1, "manifest opt-in must win over global OFF");
}

/// An installation whose board is drained on the agent's behalf (an external claimer, or a human) declares a tool list that withholds `task_claim` — and must not find the kernel racing its claimant after an upgrade.
///
/// Note the manifest declares a non-empty list: withholding is an explicit list without `task_claim` in it, never an empty one.
/// See `unrestricted_agent_is_woken` for the other side of that distinction.
#[tokio::test(flavor = "multi_thread")]
async fn agent_without_task_claim_is_not_woken() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let mut manifest = worker_manifest("no-claim");
    manifest.capabilities.tools = vec!["web_search".to_string()];
    let worker = test
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn must succeed");

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&worker.to_string())))
        .await;

    assert!(
        matches.is_empty(),
        "an agent that cannot claim must not be woken"
    );
}

/// An **empty** `capabilities.tools` means "unrestricted — every tool", which is what an agent with no `[capabilities]` section at all gets, and is the convention `kernel::tools_and_skills::available_tools` encodes as `tools_unrestricted`.
/// Reading the raw field as a deny-list inverts it and silences the wake for exactly the installations that configured nothing — the case this whole feature exists to serve.
///
/// A glob grant is checked in the same test because the declared list is matched with `glob_matches`, not string equality, mirroring how the runtime resolves declared tools at dispatch: `task_*` grants `task_claim` there, so it has to grant it here too.
#[tokio::test(flavor = "multi_thread")]
async fn unrestricted_agent_is_woken() {
    for tools in [
        vec![],
        vec!["*".to_string()],
        vec!["task_*".to_string()],
        vec!["task_claim".to_string()],
    ] {
        let test = TestAppState::with_builder(MockKernelBuilder::new());
        let mut manifest = worker_manifest("unrestricted");
        manifest.capabilities.tools = tools.clone();
        let worker = test
            .state
            .kernel
            .spawn_agent_typed(manifest)
            .expect("spawn must succeed");

        let matches = test
            .state
            .kernel
            .publish_typed_event(task_posted(Some(&worker.to_string())))
            .await;

        assert_eq!(
            matches.len(),
            1,
            "an agent that can reach task_claim must be woken (tools = {tools:?})"
        );
    }
}

/// `capabilities.tools` is not the only way to withhold a tool: `tool_allowlist` narrows whatever survived it, and `tool_blocklist` strips unconditionally (both at Step 4 of `available_tools`).
/// Either one alone can leave an otherwise unrestricted agent unable to claim, so checking the declared set alone would wake an agent whose operator had withheld `task_claim` through the mechanism this codebase actually documents for it — and race the external claimer that withholding exists to protect.
#[tokio::test(flavor = "multi_thread")]
async fn agent_denied_task_claim_by_list_is_not_woken() {
    // Each case leaves `capabilities.tools` unrestricted (empty), so only the
    // allowlist / blocklist under test can withhold the tool.
    let cases: Vec<(&str, Vec<String>, Vec<String>)> = vec![
        ("blocklist-exact", vec![], vec!["task_claim".to_string()]),
        ("blocklist-glob", vec![], vec!["task_*".to_string()]),
        ("allowlist-omits", vec!["web_search".to_string()], vec![]),
    ];

    for (label, allowlist, blocklist) in cases {
        let test = TestAppState::with_builder(MockKernelBuilder::new());
        let mut manifest = worker_manifest(label);
        manifest.capabilities.tools = vec![];
        manifest.tool_allowlist = allowlist;
        manifest.tool_blocklist = blocklist;
        let worker = test
            .state
            .kernel
            .spawn_agent_typed(manifest)
            .expect("spawn must succeed");

        let matches = test
            .state
            .kernel
            .publish_typed_event(task_posted(Some(&worker.to_string())))
            .await;

        assert!(
            matches.is_empty(),
            "an agent that cannot reach task_claim must not be woken ({label})"
        );
    }
}

/// Unassigned tasks are claimable by anyone (`assigned_to = ''` in the claim SQL) but addressed to nobody.
/// Fanning out to every capable agent is a policy decision this path deliberately does not make.
#[tokio::test(flavor = "multi_thread")]
async fn unassigned_task_wakes_nobody() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    test.state
        .kernel
        .spawn_agent_typed(worker_manifest("worker"))
        .expect("spawn must succeed");

    for assigned_to in [None, Some(""), Some("   ")] {
        let matches = test
            .state
            .kernel
            .publish_typed_event(task_posted(assigned_to))
            .await;
        assert!(
            matches.is_empty(),
            "an unassigned task must wake nobody (assigned_to = {assigned_to:?})"
        );
    }
}

/// The property this feature is riskiest on, with more than one eligible agent
/// in the registry: a wake must reach the assignee and nobody else.
///
/// Every other test here runs with a single agent, so a routing mistake that
/// woke "some agent that can claim" rather than "the addressed agent" would
/// pass all of them. On a shared board that mistake is not a missed
/// notification — it hands one tenant's task to another tenant's agent, which
/// then claims it.
#[tokio::test(flavor = "multi_thread")]
async fn only_the_addressed_agent_is_woken() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let addressed = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("addressed"))
        .expect("spawn must succeed");
    let bystander = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("bystander"))
        .expect("spawn must succeed");
    // A third agent that is equally able to claim, to make "wake anyone who
    // can claim" fail rather than accidentally pass with two.
    let third = test
        .state
        .kernel
        .spawn_agent_typed(worker_manifest("third"))
        .expect("spawn must succeed");

    let matches = test
        .state
        .kernel
        .publish_typed_event(task_posted(Some(&addressed.to_string())))
        .await;

    assert_eq!(matches.len(), 1, "exactly one agent may be woken");
    assert_eq!(matches[0].agent_id, addressed);
    assert!(
        !matches
            .iter()
            .any(|m| m.agent_id == bystander || m.agent_id == third),
        "an agent that was not addressed must never be woken for someone else's task"
    );
}
