//! Boot must propagate `[budget] default_burst_ratio` into the agent scheduler (#8115).
//!
//! `boot.rs` seeds the scheduler's global default from `current_budget()` before
//! restoring persisted agents. If that call is ever dropped, agents registered
//! with `burst_ratio: None` silently fall back to the compiled `0.2` — this test
//! observes the propagation behaviourally instead of trusting the call site.
//!
//! These are real `boot_with_config` boots against a temp home dir; no LLM
//! credentials are needed (nothing dials a provider at boot).

use librefang_kernel::{AgentSubsystemApi, LibreFangKernel};
use librefang_types::agent::{AgentId, ResourceQuota};
use librefang_types::config::{BudgetConfig, DefaultModelConfig, KernelConfig};
use librefang_types::message::TokenUsage;

/// Boot a kernel with `budget.default_burst_ratio` set to the given ratio.
fn boot_with_burst_ratio(ratio: f32) -> LibreFangKernel {
    let tmp = std::env::temp_dir().join(format!("librefang-burst-boot-{}", ratio.to_bits()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let config = KernelConfig {
        home_dir: tmp.clone(),
        data_dir: tmp.join("data"),
        default_model: DefaultModelConfig {
            provider: "groq".to_string(),
            model: "llama-3.3-70b-versatile".to_string(),
            api_key_env: "GROQ_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        budget: BudgetConfig {
            default_burst_ratio: ratio,
            ..Default::default()
        },
        ..KernelConfig::default()
    };
    LibreFangKernel::boot_with_config(config).expect("kernel boot")
}

/// An agent whose quota carries `burst_ratio: None` must be capped at
/// `token_limit × config value`. Before the #8115 fix the boot call was
/// missing entirely, so the compiled `0.2` cap (200/min for a 1000/hour
/// quota) applied and 450 tokens in one minute was rejected.
#[test]
fn boot_propagates_budget_default_burst_ratio_to_scheduler() {
    let kernel = boot_with_burst_ratio(0.5); // cap = 1000 × 0.5 = 500/min
    let id = AgentId::new();
    kernel.scheduler_ref().register(
        id,
        ResourceQuota {
            max_llm_tokens_per_hour: Some(1000),
            max_tool_calls_per_minute: 0, // unlimited tool calls
            ..Default::default()
        },
    );

    // 450 tokens in the last minute: over the compiled 0.2 cap (200),
    // under the configured 0.5 cap (500).
    kernel.scheduler_ref().record_usage(
        id,
        &TokenUsage {
            input_tokens: 450,
            output_tokens: 0,
            ..Default::default()
        },
    );
    assert!(
        kernel.scheduler_ref().check_quota(id).is_ok(),
        "450 tokens/min must pass the configured 500/min burst cap; the compiled 0.2 cap of 200 would reject it"
    );

    kernel.shutdown();
}

/// The scheduler-side default must also *tighten* on boot: a config that
/// lowers the ratio below the compiled `0.2` must be honoured, proving the
/// value came from config rather than the compiled fallback.
#[test]
fn boot_honours_a_default_burst_ratio_below_the_compiled_fallback() {
    let kernel = boot_with_burst_ratio(0.1); // cap = 1000 × 0.1 = 100/min
    let id = AgentId::new();
    kernel.scheduler_ref().register(
        id,
        ResourceQuota {
            max_llm_tokens_per_hour: Some(1000),
            max_tool_calls_per_minute: 0, // unlimited tool calls
            ..Default::default()
        },
    );

    // 150 tokens in the last minute: under the compiled 0.2 cap (200) but
    // over the configured 0.1 cap (100).
    kernel.scheduler_ref().record_usage(
        id,
        &TokenUsage {
            input_tokens: 150,
            output_tokens: 0,
            ..Default::default()
        },
    );
    assert!(
        kernel.scheduler_ref().check_quota(id).is_err(),
        "150 tokens/min must breach the configured 100/min burst cap; the compiled 0.2 cap of 200 would allow it"
    );

    kernel.shutdown();
}
