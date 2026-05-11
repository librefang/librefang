//! Pre-dispatch provider budget gate (#4828, #4800).
//!
//! Pins the `[providers.<name>]` budget gate inserted at the top of
//! the kernel's three dispatch paths in `kernel/messaging.rs`:
//!
//!   1. `send_message_ephemeral`  — `/btw` side question
//!   2. `send_message`            — full agent message
//!   3. `send_message_streaming`  — streaming agent message
//!
//! All three paths call the shared helper `check_provider_budget_for`
//! BEFORE acquiring any token or USD reservation, so a rejection
//! cannot leak the per-agent burst window or the in-memory pending
//! USD ledger. Each test below configures a provider budget with
//! `max_cost_per_hour_usd = 1.0`, records a usage row that exceeds
//! it, registers an agent that targets the same provider, and asserts
//! the kernel rejects the call with `LibreFangError::QuotaExceeded`
//! BEFORE any LLM round-trip happens.
//!
//! `full_path_does_not_leak_reservations_on_repeated_rejection`
//! additionally asserts the no-leak contract directly: after three
//! back-to-back denials, both the global `pending_reserved_usd()`
//! ledger and the per-agent `scheduler.total_tokens` counter remain
//! at zero. Any future regression that moves the gate AFTER a
//! reservation without restoring an explicit rollback would flip
//! either of those values to non-zero and fail the test.
//!
//! Refs #4828 (gate placement), #4800 (the underlying issue this PR
//! is closing), CLAUDE.md #3721 (mandatory integration test for any
//! kernel/route wiring change).

use librefang_kernel::error::KernelError;
use librefang_kernel::{KernelApi, LibreFangKernel, MeteringSubsystemApi};
use librefang_memory::usage::{UsageRecord, UsageStore};
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, ResourceQuota, SessionId,
};
use librefang_types::config::ProviderBudget;
use librefang_types::error::LibreFangError;
use std::sync::Arc;
use tempfile::TempDir;

const PROVIDER: &str = "ollama";
const MODEL: &str = "test-model";

/// Build a kernel where:
///   - The default model points at `PROVIDER` / `MODEL` so any agent
///     registered without an explicit model inherits that pair.
///   - `[providers.ollama]` carries a `$1.00 / hour` cost limit.
///   - The global `[budget]` carries a `$100 / hour` cap so
///     `reserve_global_budget` actually charges the in-memory pending
///     ledger — a leaked reservation would then surface via
///     `pending_reserved_usd() > 0` after the call returns.
fn build_kernel() -> (Arc<LibreFangKernel>, TempDir) {
    MockKernelBuilder::new()
        .with_config(|cfg| {
            cfg.default_model = librefang_types::config::DefaultModelConfig {
                provider: PROVIDER.to_string(),
                model: MODEL.to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
                message_timeout_secs: 300,
                extra_params: std::collections::HashMap::new(),
                cli_profile_dirs: Vec::new(),
            };
            cfg.budget.providers.insert(
                PROVIDER.to_string(),
                ProviderBudget {
                    max_cost_per_hour_usd: 1.0,
                    ..Default::default()
                },
            );
            cfg.budget.max_hourly_usd = 100.0;
        })
        .build()
}

/// Insert a usage row attributed to `PROVIDER` whose cost crosses the
/// hourly limit. The metering store reads this back inside
/// `query_provider_hourly`, which is what `check_provider_budget`
/// consults at the gate.
fn exhaust_provider_budget(kernel: &LibreFangKernel) {
    let store = UsageStore::new(kernel.memory_substrate().pool());
    let mut rec = UsageRecord::anonymous(AgentId::new(), PROVIDER, MODEL, 100, 200, 5.0, 0, 10);
    rec.session_id = Some(SessionId::new());
    store.record(&rec).unwrap();
}

/// Register an agent whose manifest targets `PROVIDER` so the gate
/// looks up the budget for the right provider name.
fn register_agent(kernel: &LibreFangKernel) -> AgentId {
    register_agent_with_quota(kernel, 0, false)
}

/// Like [`register_agent`] but optionally sets a non-default
/// `manifest.model.max_tokens` and installs a per-agent
/// `ResourceQuota` on the scheduler. Used by the no-leak test so the
/// scheduler's `total_tokens` counter is observable via
/// `scheduler_ref().get_usage(agent_id)` (the counter only exists
/// after a `register` on the scheduler).
fn register_agent_with_quota(
    kernel: &LibreFangKernel,
    max_tokens: u32,
    install_scheduler_quota: bool,
) -> AgentId {
    let id = AgentId::new();
    let mut manifest = AgentManifest {
        name: "budget-test".to_string(),
        description: "test agent".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        ..Default::default()
    };
    manifest.model.provider = PROVIDER.to_string();
    manifest.model.model = MODEL.to_string();
    if max_tokens > 0 {
        manifest.model.max_tokens = max_tokens;
    }
    let entry = AgentEntry {
        id,
        name: "budget-test".to_string(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };
    kernel.agent_registry().register(entry).unwrap();
    if install_scheduler_quota {
        kernel.scheduler_ref().register(
            id,
            ResourceQuota {
                max_llm_tokens_per_hour: Some(20_000),
                ..Default::default()
            },
        );
    }
    id
}

fn assert_quota_exceeded(err: KernelError, label: &str) {
    match err {
        KernelError::LibreFang(LibreFangError::QuotaExceeded(msg)) => {
            assert!(
                msg.contains(PROVIDER),
                "{label}: QuotaExceeded should name the provider, got: {msg}"
            );
            assert!(
                msg.contains("hourly"),
                "{label}: QuotaExceeded should attribute to the hourly window, got: {msg}"
            );
        }
        other => panic!("{label}: expected QuotaExceeded, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ephemeral_path_rejects_when_provider_hourly_budget_exhausted() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    let err = kernel
        .send_message_ephemeral(agent_id, "ping", None)
        .await
        .expect_err("ephemeral path must refuse over-budget call");
    assert_quota_exceeded(err, "ephemeral");
}

#[tokio::test(flavor = "multi_thread")]
async fn full_path_rejects_when_provider_hourly_budget_exhausted() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    let err = kernel
        .send_message(agent_id, "ping")
        .await
        .expect_err("full path must refuse over-budget call");
    assert_quota_exceeded(err, "full");
}

/// Direct streaming-path test. `send_message_streaming` is a sync
/// entry that returns `KernelResult<(Receiver, JoinHandle)>` — gate
/// rejection comes back synchronously, before the spawn, so the
/// receiver doesn't need to be driven at all.
#[tokio::test(flavor = "multi_thread")]
async fn streaming_path_rejects_when_provider_hourly_budget_exhausted() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    let err = kernel
        .send_message_streaming(agent_id, "ping", None)
        .expect_err("streaming path must refuse over-budget call");
    assert_quota_exceeded(err, "streaming");
}

/// No-leak contract: 3 back-to-back denied full-path calls must NOT
/// poison the global pending USD ledger or the per-agent scheduler
/// `total_tokens` counter. Setup tightens both:
///
///   - `cfg.budget.max_hourly_usd = $100` so `reserve_global_budget`
///     would actually charge the in-memory pending ledger if it ran.
///   - Per-agent `ResourceQuota::max_llm_tokens_per_hour = 20_000`
///     plus `manifest.model.max_tokens = 2000` so a successful
///     `check_quota_and_reserve` would pre-charge 2000 tokens visible
///     via `scheduler_ref().get_usage(...).total_tokens`.
///
/// With the gate placed BEFORE both reservations, none of the three
/// denied calls ever reaches `reserve_global_budget` or
/// `check_quota_and_reserve` — both ledgers stay at 0. Any future
/// regression that moves the gate back AFTER either reservation
/// without restoring an explicit rollback would flip the snapshots
/// to non-zero and fail the test.
#[tokio::test(flavor = "multi_thread")]
async fn full_path_does_not_leak_reservations_on_repeated_rejection() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent_with_quota(&kernel, 2000, true);

    for attempt in 1..=3 {
        let err = kernel.send_message(agent_id, "ping").await.unwrap_err();
        assert_quota_exceeded(err, &format!("attempt {attempt}"));
    }

    assert_eq!(
        kernel.metering_engine().pending_reserved_usd(),
        0.0,
        "pending USD ledger must be empty after 3 denied calls"
    );
    let usage = kernel
        .scheduler_ref()
        .get_usage(agent_id)
        .expect("scheduler usage tracker is created by register()");
    assert_eq!(
        usage.total_tokens, 0,
        "scheduler total_tokens must be 0 after 3 denied calls — leak suspected"
    );
}

/// Negative test: with no usage on file the gate must NOT fire. We
/// don't assert success — without a live Ollama the call will fail
/// downstream of the gate, which is fine. The only forbidden outcome
/// is a `QuotaExceeded` whose message attributes to the hourly cost
/// budget when there is no spend at all.
#[tokio::test(flavor = "multi_thread")]
async fn ephemeral_path_passes_when_provider_budget_not_exhausted() {
    let (kernel, _tmp) = build_kernel();
    let agent_id = register_agent(&kernel);

    let result = kernel.send_message_ephemeral(agent_id, "ping", None).await;
    if let Err(KernelError::LibreFang(LibreFangError::QuotaExceeded(msg))) = &result {
        assert!(
            !msg.contains("hourly cost budget"),
            "gate should not fire on a clean budget, got: {msg}"
        );
    }
}
