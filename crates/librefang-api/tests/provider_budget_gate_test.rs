//! Pre-dispatch provider-budget gate REMOVAL (#4807).
//!
//! Originally (#4828, #4800) the kernel rejected any send_message* call
//! whose target provider had already crossed its `[providers.<name>]`
//! cost / token budget. That gate ran *before* token / USD reservation
//! and returned `LibreFangError::QuotaExceeded` synchronously, which
//! solved the over-spend problem at the cost of preventing the LLM
//! fallback chain from trying alternative providers — see #4807.
//!
//! Per #4807 the gate has been removed from all three dispatch paths
//! (`send_message_ephemeral`, `send_message`, `send_message_streaming`).
//! Budget exhaustion is now propagated via the shared
//! `ProviderExhaustionStore`: when `MeteringEngine::check_provider_budget`
//! refuses a provider (called by post-call accounting and dashboards),
//! the engine flags the provider so the LLM fallback chain skips it on
//! the next request and falls over to a healthy slot.
//!
//! These tests pin the new contract: the kernel must NOT short-circuit
//! a send_message* call with `QuotaExceeded(... hourly cost budget ...)`
//! when only the per-provider budget is over. They do *not* assert
//! end-to-end success — without a live LLM the call still fails
//! downstream — they only assert the specific gate-shaped failure mode
//! no longer happens, which would regress #4807.
//!
//! A separate test (`check_provider_budget_flags_exhaustion_store`)
//! pins the substrate behaviour the chain consumes: when
//! `MeteringEngine::check_provider_budget` refuses, the metering
//! engine's attached `ProviderExhaustionStore` MUST carry a
//! `BudgetExceeded` record so the fallback chain can skip the slot.
//!
//! The unconditional global-budget gate inside `reserve_global_budget`
//! is unaffected (and is exercised by `metering_test.rs`); only the
//! per-provider gate that #4807 explicitly asked to drop is gone.

use librefang_kernel::error::KernelError;
use librefang_kernel::{KernelApi, LibreFangKernel, MeteringSubsystemApi};
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
///   - `[providers.ollama]` carries a `$1.00 / hour` cost limit so a
///     single oversized usage row blows it.
///   - The global `[budget]` carries a `$100 / hour` cap so a global
///     gate isn't the thing that fires; we want to be sure the
///     per-provider gate is the one being asserted gone.
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
/// would have consulted at the old gate.
fn exhaust_provider_budget(kernel: &LibreFangKernel) {
    let store = UsageStore::new(kernel.memory_substrate().pool());
    let mut rec = UsageRecord::anonymous(AgentId::new(), PROVIDER, MODEL, 100, 200, 5.0, 0, 10);
    rec.session_id = Some(SessionId::new());
    store.record(&rec).unwrap();
}

/// Register an agent whose manifest targets `PROVIDER` so the gate
/// (if it still existed) would key off the right provider name.
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

/// Helper: assert that, *if* the call returned an error, it was NOT a
/// `QuotaExceeded(... per-provider hourly cost budget ...)` — i.e. the
/// old pre-dispatch gate did not fire. Other failures (no LLM wired,
/// driver init error, etc.) are tolerated; we are only asserting the
/// specific gate-shaped rejection is gone.
fn assert_no_per_provider_gate(result: Result<impl std::fmt::Debug, KernelError>, label: &str) {
    if let Err(KernelError::LibreFang(LibreFangError::QuotaExceeded(msg))) = &result {
        assert!(
            !(msg.contains(PROVIDER) && msg.contains("hourly cost budget")),
            "{label}: per-provider pre-dispatch gate must be gone per #4807, but got: {msg}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ephemeral_path_no_longer_pre_dispatch_gates_on_provider_budget() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    let result = kernel.send_message_ephemeral(agent_id, "ping", None).await;
    assert_no_per_provider_gate(result, "ephemeral");
}

#[tokio::test(flavor = "multi_thread")]
async fn full_path_no_longer_pre_dispatch_gates_on_provider_budget() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    let result = kernel.send_message(agent_id, "ping").await;
    assert_no_per_provider_gate(result, "full");
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_path_no_longer_pre_dispatch_gates_on_provider_budget() {
    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);
    let agent_id = register_agent(&kernel);

    // `send_message_streaming` is sync; if the old gate were still in
    // place the synchronous error would surface a `QuotaExceeded`. The
    // post-removal contract: the gate does not fire here.
    let result = kernel.send_message_streaming(agent_id, "ping", None);
    if let Err(KernelError::LibreFang(LibreFangError::QuotaExceeded(msg))) = &result {
        assert!(
            !(msg.contains(PROVIDER) && msg.contains("hourly cost budget")),
            "streaming: per-provider pre-dispatch gate must be gone per #4807, but got: {msg}"
        );
    }
}

/// Substrate behaviour the LLM fallback chain consumes (#4807 Blocker 5
/// follow-up). When the metering engine's
/// `check_provider_budget` refuses an over-budget provider, the
/// engine MUST flag that provider in its attached
/// `ProviderExhaustionStore` with reason `BudgetExceeded`. That is the
/// hand-off point: the kernel no longer rejects pre-dispatch, but the
/// next dispatch routes through `FallbackChain` / `FallbackDriver`
/// which reads the same store and skips the slot.
#[tokio::test(flavor = "multi_thread")]
async fn check_provider_budget_flags_exhaustion_store() {
    use librefang_runtime::llm_driver::exhaustion::ExhaustionReason;

    let (kernel, _tmp) = build_kernel();
    exhaust_provider_budget(&kernel);

    let pb = ProviderBudget {
        max_cost_per_hour_usd: 1.0,
        ..Default::default()
    };
    let result = kernel
        .metering_engine()
        .check_provider_budget(PROVIDER, &pb);
    assert!(
        matches!(&result, Err(LibreFangError::QuotaExceeded(_))),
        "post-call gate must still refuse over-budget provider, got: {result:?}"
    );

    // The attached store now carries a BudgetExceeded record.
    let store = kernel
        .metering_engine()
        .exhaustion_store()
        .expect("boot must wire an exhaustion store into the metering engine (#4807)");
    let rec = store
        .is_exhausted(PROVIDER)
        .expect("over-budget provider must be flagged in the exhaustion store");
    assert_eq!(rec.reason, ExhaustionReason::BudgetExceeded);
}
