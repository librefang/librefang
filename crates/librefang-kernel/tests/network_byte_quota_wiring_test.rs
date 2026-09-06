//! The runtime reaches the network byte quota through `AgentControl`, so the kernel must actually override those two methods.
//!
//! `AgentControl::check_network_quota` / `record_network_bytes` both carry defaults — `Ok(())` and a no-op — so that stub kernels in the runtime's own tests need no impl.
//! That makes the kernel's override the single load-bearing wiring point for `agent.toml: [resources] max_network_bytes_per_hour`: if it is ever dropped, every call silently takes the permissive default and the field goes back to being the inert config it was before, with no compile error anywhere.
//! These tests observe the wiring behaviourally, against a real booted kernel, rather than trusting the call site.
//!
//! Real `boot_with_config` boots against a temp home dir; no LLM credentials are needed (nothing dials a provider at boot).

use librefang_kernel::{AgentSubsystemApi, LibreFangKernel};
use librefang_kernel_handle::AgentControl;
use librefang_types::agent::{AgentId, ResourceQuota};
use librefang_types::config::{DefaultModelConfig, KernelConfig};

fn boot_kernel(tag: &str) -> LibreFangKernel {
    let tmp = std::env::temp_dir().join(format!("librefang-net-quota-{tag}"));
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
        ..KernelConfig::default()
    };
    LibreFangKernel::boot_with_config(config).expect("kernel boot")
}

/// A quota whose only live limit is the hourly network byte cap.
fn network_only_quota(max_network_bytes_per_hour: u64) -> ResourceQuota {
    ResourceQuota {
        max_llm_tokens_per_hour: Some(0), // unlimited tokens
        max_tool_calls_per_minute: 0,     // unlimited tool calls
        max_network_bytes_per_hour,
        ..Default::default()
    }
}

/// Bytes reported through the kernel handle must land on the agent's scheduler window and, once the cap is reached, must make the handle refuse the next egress tool.
/// With the kernel's override missing this test fails on the second assertion: the trait default returns `Ok(())` no matter how much the agent transferred.
#[test]
fn kernel_handle_charges_reported_bytes_against_the_agent_quota() {
    let kernel = boot_kernel("charge");
    let id = AgentId::new();
    let id_str = id.to_string();
    kernel
        .scheduler_ref()
        .register(id, network_only_quota(4096));

    assert!(
        AgentControl::check_network_quota(&kernel, &id_str).is_ok(),
        "a fresh agent must be under its cap"
    );

    AgentControl::record_network_bytes(&kernel, &id_str, 4096);
    assert_eq!(
        kernel.scheduler_ref().get_usage(id).unwrap().network_bytes,
        4096,
        "the handle must report into the agent's own scheduler window"
    );

    let err = AgentControl::check_network_quota(&kernel, &id_str)
        .expect_err("4096 of 4096 bytes must exhaust the hourly cap");
    assert!(
        err.to_string().contains("4096 / 4096 bytes per hour"),
        "the refusal must name both the spend and the cap: {err}"
    );

    kernel.shutdown();
}

/// `0` means unlimited on the kernel handle exactly as it does in the scheduler — an operator who clears the field must not find their agent throttled.
#[test]
fn kernel_handle_treats_a_zero_cap_as_unlimited() {
    let kernel = boot_kernel("unlimited");
    let id = AgentId::new();
    let id_str = id.to_string();
    kernel.scheduler_ref().register(id, network_only_quota(0));

    AgentControl::record_network_bytes(&kernel, &id_str, 512 * 1024 * 1024);
    assert!(
        AgentControl::check_network_quota(&kernel, &id_str).is_ok(),
        "a cap of 0 must never refuse, however many bytes were transferred"
    );

    kernel.shutdown();
}

/// An id that is not a known agent must not error or panic: the runtime hands over whatever string it was given as the caller, and the MCP HTTP bridge in particular passes ids the kernel has never registered.
#[test]
fn kernel_handle_is_silent_for_an_unknown_agent_id() {
    let kernel = boot_kernel("unknown");

    AgentControl::record_network_bytes(&kernel, "not-a-uuid", 1_000_000);
    assert!(AgentControl::check_network_quota(&kernel, "not-a-uuid").is_ok());
    assert!(AgentControl::check_network_quota(&kernel, &AgentId::new().to_string()).is_ok());

    kernel.shutdown();
}
