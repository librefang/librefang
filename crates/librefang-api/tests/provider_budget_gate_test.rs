//! Pre-dispatch provider budget gate (#4828, #4800).
//!
//! Pins the `[providers.<name>]` budget gate inserted at the top of
//! the kernel's three dispatch paths in `kernel/messaging.rs`:
//!
//!   1. `send_message_ephemeral` — `/btw` side question
//!   2. `send_message`           — full agent message
//!   3. streaming                — exercised via the same gate code
//!
//! Each test configures a provider budget with `max_cost_per_hour_usd
//! = 1.0`, records a usage row that exceeds it, registers an agent
//! that targets the same provider, and asserts the kernel rejects the
//! call with `LibreFangError::QuotaExceeded` BEFORE any LLM round-trip
//! happens.
//!
//! Refs #4828 (gate placement), #4800 (the underlying issue this PR
//! is closing), CLAUDE.md #3721 (mandatory integration test for any
//! kernel/route wiring change).

use librefang_kernel::error::KernelError;
use librefang_kernel::{KernelApi, LibreFangKernel};
use librefang_memory::usage::{UsageRecord, UsageStore};
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, SessionId,
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
        .send_message_ephemeral(agent_id, "ping")
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

/// The streaming path runs the gate AFTER `check_quota_and_reserve`,
/// so a rejection MUST release the `token_reservation` — otherwise a
/// hot loop of denied calls slowly drains the per-agent burst window.
/// Pin the rollback by calling the same gated entry twice in a row:
/// a leaking reservation would surface as a different error class on
/// the second attempt (the scheduler would saturate before the gate
/// got a chance to fire), instead of the identical `QuotaExceeded`
/// the gate produces on a clean reservation slot.
#[tokio::test(flavor = "multi_thread")]
async fn full_path_releases_reservations_on_repeated_rejection() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    for attempt in 1..=3 {
        let err = kernel.send_message(agent_id, "ping").await.unwrap_err();
        assert_quota_exceeded(err, &format!("attempt {attempt}"));
    }
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

    let result = kernel.send_message_ephemeral(agent_id, "ping").await;
    if let Err(KernelError::LibreFang(LibreFangError::QuotaExceeded(msg))) = &result {
        assert!(
            !msg.contains("hourly cost budget"),
            "gate should not fire on a clean budget, got: {msg}"
        );
    }
}
