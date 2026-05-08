//! Hand lifecycle tests: activation, deactivation, pause/resume, deterministic IDs,
//! agent tagging, tool inheritance, state persistence, coexistence, and error cases.
//!
//! The last test (`test_six_agent_fleet`) is a live LLM integration test
//! that only runs when GROQ_API_KEY is set.

use librefang_kernel::triggers::TriggerPattern;
use librefang_kernel::AgentSubsystemApi;
use librefang_kernel::LibreFangKernel;
use librefang_kernel::SkillsSubsystemApi;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::collections::HashMap;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn test_config(name: &str) -> KernelConfig {
    let tmp = std::env::temp_dir().join(format!("librefang-hand-test-{name}"));
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

/// Install a hand from TOML content into the kernel's hand registry.
fn install_hand(kernel: &LibreFangKernel, toml_content: &str) {
    kernel
        .hand_registry_ref()
        .install_from_content(toml_content, "")
        .unwrap_or_else(|e| panic!("Failed to install hand: {e}"));
}

const HAND_A: &str = r#"
id = "test-clip"
name = "Test Clip Hand"
description = "A test hand for clip content"
category = "content"
icon = "🎬"
tools = ["file_read", "file_write", "shell_exec"]

[routing]
aliases = ["test clip"]

[agent]
name = "clip-agent"
description = "Creates short clips"
module = "builtin:chat"
provider = "default"
model = "default"
system_prompt = "You are a clip agent."
tools = ["file_read"]
"#;

const HAND_B: &str = r#"
id = "test-devops"
name = "Test DevOps Hand"
description = "A test hand for devops"
category = "development"
icon = "🔧"
tools = ["shell_exec"]

[routing]
aliases = ["test devops"]

[agent]
name = "devops-agent"
description = "Manages CI/CD"
module = "builtin:chat"
system_prompt = "You are a devops agent."
"#;

const HAND_C: &str = r#"
id = "test-research"
name = "Test Research Hand"
description = "A test hand with an explicit non-main coordinator"
category = "data"
icon = "🧠"
tools = ["file_read"]

[routing]
aliases = ["test research"]

[agents.analyst]
name = "analyst-agent"
description = "Analyzes information"
module = "builtin:chat"

[agents.analyst.model]
provider = "default"
model = "default"
system_prompt = "You are an analyst."

[agents.planner]
coordinator = true
name = "planner-agent"
description = "Plans the work"
module = "builtin:chat"

[agents.planner.model]
provider = "default"
model = "default"
system_prompt = "You are a planner."
"#;

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_activate_hand_spawns_agent() {
    let kernel = LibreFangKernel::boot_with_config(test_config("activate")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();

    assert_eq!(instance.hand_id, "test-clip");
    assert!(instance.agent_id().is_some(), "Agent should be spawned");

    let agent_id = instance.agent_id().unwrap();
    assert!(
        kernel.agent_registry_ref().get(agent_id).is_some(),
        "Agent should exist in registry"
    );

    kernel.shutdown();
}

#[test]
fn test_deterministic_agent_id() {
    let kernel = LibreFangKernel::boot_with_config(test_config("deterministic")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    // `activate_hand` passes None for instance_id, so the legacy format is used
    let expected = AgentId::from_hand_agent("test-clip", "main", None);

    assert_eq!(
        instance.agent_id().unwrap(),
        expected,
        "Agent ID should be deterministic from hand_id + role (legacy format)"
    );

    kernel.shutdown();
}

#[test]
fn test_explicit_coordinator_role_used_for_routes() {
    let kernel = LibreFangKernel::boot_with_config(test_config("explicit-coordinator")).unwrap();
    install_hand(&kernel, HAND_C);

    let instance = kernel
        .activate_hand("test-research", HashMap::new())
        .unwrap();

    assert_eq!(instance.coordinator_role.as_deref(), Some("planner"));
    assert_eq!(instance.agent_name(), "planner");
    assert_eq!(
        instance.agent_id(),
        instance.agent_ids.get("planner").copied(),
        "Hand routes should resolve to the explicit coordinator role"
    );

    kernel.shutdown();
}

#[test]
fn test_deterministic_id_stable_across_reactivation() {
    let kernel = LibreFangKernel::boot_with_config(test_config("reactivate")).unwrap();
    install_hand(&kernel, HAND_A);

    // First activation
    let inst1 = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let id1 = inst1.agent_id().unwrap();

    // Agent ID uses legacy format for single-instance activation.
    let expected1 = AgentId::from_hand_agent("test-clip", "main", None);
    assert_eq!(
        id1, expected1,
        "Agent ID should use legacy format for single-instance activation"
    );

    // Deactivate
    kernel.deactivate_hand(inst1.instance_id).unwrap();

    // Re-install (since deactivate doesn't remove the definition, but it may
    // already be registered — wrap in allow-already-active)
    let _ = kernel.hand_registry_ref().install_from_content(HAND_A, "");

    // Second activation gets a new instance_id and therefore a new unique agent ID.
    let inst2 = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let id2 = inst2.agent_id().unwrap();

    // Re-activation also uses legacy format — same ID as first activation.
    let expected2 = AgentId::from_hand_agent("test-clip", "main", None);
    assert_eq!(
        id2, expected2,
        "Agent ID should use legacy format for single-instance activation"
    );

    // Single-instance activations use legacy format — same hand+role = same ID.
    assert_eq!(
        id1, id2,
        "Single-instance reactivation preserves the same agent ID (legacy format)"
    );

    kernel.shutdown();
}

#[test]
fn test_deactivate_kills_agent() {
    let kernel = LibreFangKernel::boot_with_config(test_config("deactivate")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    // Agent should exist before deactivation
    assert!(kernel.agent_registry_ref().get(agent_id).is_some());

    kernel.deactivate_hand(instance.instance_id).unwrap();

    // Agent should be gone after deactivation
    assert!(
        kernel.agent_registry_ref().get(agent_id).is_none(),
        "Agent should be killed after deactivation"
    );

    kernel.shutdown();
}

#[test]
fn test_pause_and_resume_hand() {
    let kernel = LibreFangKernel::boot_with_config(test_config("pause-resume")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let instance_id = instance.instance_id;
    let agent_id = instance.agent_id().unwrap();

    // Pause
    kernel.pause_hand(instance_id).unwrap();
    let paused = kernel
        .hand_registry_ref()
        .get_instance(instance_id)
        .unwrap();
    assert_eq!(paused.status.to_string(), "Paused");

    // Agent should still exist (paused, not killed)
    assert!(
        kernel.agent_registry_ref().get(agent_id).is_some(),
        "Paused agent should still exist"
    );

    // Resume
    kernel.resume_hand(instance_id).unwrap();
    let resumed = kernel
        .hand_registry_ref()
        .get_instance(instance_id)
        .unwrap();
    assert_eq!(resumed.status.to_string(), "Active");

    kernel.shutdown();
}

#[test]
fn test_agent_tagged_with_hand_metadata() {
    let kernel = LibreFangKernel::boot_with_config(test_config("tags")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    let entry = kernel.agent_registry_ref().get(agent_id).unwrap();
    assert!(
        entry.tags.contains(&"hand:test-clip".to_string()),
        "Agent should be tagged with hand ID"
    );
    assert!(
        entry
            .tags
            .contains(&format!("hand_instance:{}", instance.instance_id)),
        "Agent should be tagged with instance ID"
    );

    kernel.shutdown();
}

#[test]
fn test_hand_tools_applied_to_agent() {
    let kernel = LibreFangKernel::boot_with_config(test_config("tools")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    let entry = kernel.agent_registry_ref().get(agent_id).unwrap();
    // HAND_A defines tools = ["file_read", "file_write", "shell_exec"]
    for tool in &["file_read", "file_write", "shell_exec"] {
        assert!(
            entry
                .manifest
                .capabilities
                .tools
                .contains(&tool.to_string()),
            "Agent should have tool '{tool}' from hand definition"
        );
    }

    kernel.shutdown();
}

#[test]
fn test_activate_nonexistent_hand_fails() {
    let kernel = LibreFangKernel::boot_with_config(test_config("nonexistent")).unwrap();

    let result = kernel.activate_hand("does-not-exist", HashMap::new());
    assert!(result.is_err(), "Activating nonexistent hand should fail");

    kernel.shutdown();
}

#[test]
fn test_deactivate_nonexistent_instance_fails() {
    let kernel = LibreFangKernel::boot_with_config(test_config("deactivate-none")).unwrap();

    let fake_id = uuid::Uuid::new_v4();
    let result = kernel.deactivate_hand(fake_id);
    assert!(
        result.is_err(),
        "Deactivating nonexistent instance should fail"
    );

    kernel.shutdown();
}

#[test]
fn test_hand_state_persistence() {
    let config = test_config("persistence");
    let state_path = config.home_dir.join("data").join("hand_state.json");

    let kernel = LibreFangKernel::boot_with_config(config).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    // State file should exist after activation
    assert!(
        state_path.exists(),
        "State file should be persisted after activation"
    );

    let state_json = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();

    assert_eq!(state["version"], 5, "State should be version 5");
    let instances = state["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 1);

    let inst = &instances[0];
    assert_eq!(inst["hand_id"], "test-clip");

    // Validate v4 typed persistence fields
    assert!(
        inst["instance_id"].is_string(),
        "v4 should have string instance_id"
    );
    assert!(inst["status"].is_string(), "v4 should have string status");
    assert!(
        inst["activated_at"].is_string(),
        "v4 should have string activated_at"
    );
    assert!(
        inst["updated_at"].is_string(),
        "v4 should have string updated_at"
    );

    // v3 uses agent_ids map
    let agent_ids_map = inst["agent_ids"].as_object().unwrap();
    assert!(agent_ids_map
        .values()
        .any(|v| v.as_str() == Some(&agent_id.to_string())));

    kernel.shutdown();
}

#[test]
fn test_multi_agent_hand_state_persists_coordinator_role() {
    let config = test_config("multi-persistence");
    let state_path = config.home_dir.join("data").join("hand_state.json");

    let kernel = LibreFangKernel::boot_with_config(config).unwrap();
    install_hand(&kernel, HAND_C);

    let instance = kernel
        .activate_hand("test-research", HashMap::new())
        .unwrap();
    assert_eq!(instance.coordinator_role.as_deref(), Some("planner"));

    let state_json = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();
    let inst = &state["instances"].as_array().unwrap()[0];
    assert_eq!(inst["coordinator_role"], "planner");

    kernel.shutdown();
}

/// A hand with `[[settings]]` declaring two keys with non-empty defaults.
const HAND_WITH_SETTINGS: &str = r#"
id = "test-settings"
name = "Test Settings Hand"
description = "Has [[settings]] for default-seeding tests"
category = "content"
icon = "⚙️"

[[settings]]
key = "verbosity"
label = "Verbosity"
setting_type = "select"
default = "normal"
[[settings.options]]
value = "quiet"
label = "Quiet"
[[settings.options]]
value = "normal"
label = "Normal"

[[settings]]
key = "max_concurrency"
label = "Max concurrency"
setting_type = "text"
default = "5"

[agent]
name = "settings-agent"
description = "Test agent"
module = "builtin:chat"
provider = "default"
model = "default"
system_prompt = "You are a settings agent."
"#;

#[test]
fn test_activation_seeds_schema_defaults_into_config() {
    let config = test_config("seed-defaults");
    let state_path = config.home_dir.join("data").join("hand_state.json");

    let kernel = LibreFangKernel::boot_with_config(config).unwrap();
    install_hand(&kernel, HAND_WITH_SETTINGS);

    // Activate with NO user overrides — all keys should be filled from defaults.
    let instance = kernel
        .activate_hand("test-settings", HashMap::new())
        .unwrap();

    assert_eq!(
        instance.config.get("verbosity").and_then(|v| v.as_str()),
        Some("normal"),
        "verbosity should be seeded from schema default"
    );
    assert_eq!(
        instance
            .config
            .get("max_concurrency")
            .and_then(|v| v.as_str()),
        Some("5"),
        "max_concurrency should be seeded from schema default"
    );

    // Persisted state on disk should reflect the seeded values, not `{}`.
    let state_json = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();
    let inst = state["instances"]
        .as_array()
        .and_then(|a| a.iter().find(|i| i["hand_id"] == "test-settings"))
        .expect("persisted instance should exist");
    assert_eq!(inst["config"]["verbosity"], "normal");
    assert_eq!(inst["config"]["max_concurrency"], "5");

    kernel.shutdown();
}

#[test]
fn test_activation_preserves_user_overrides_over_defaults() {
    let kernel = LibreFangKernel::boot_with_config(test_config("seed-defaults-override")).unwrap();
    install_hand(&kernel, HAND_WITH_SETTINGS);

    // User overrides one key; the other should still get the default.
    let mut user_config = HashMap::new();
    user_config.insert(
        "verbosity".to_string(),
        serde_json::Value::String("quiet".to_string()),
    );

    let instance = kernel.activate_hand("test-settings", user_config).unwrap();

    assert_eq!(
        instance.config.get("verbosity").and_then(|v| v.as_str()),
        Some("quiet"),
        "user override must win over schema default"
    );
    assert_eq!(
        instance
            .config
            .get("max_concurrency")
            .and_then(|v| v.as_str()),
        Some("5"),
        "untouched key should still receive its schema default"
    );

    kernel.shutdown();
}

/// Schema-evolution backfill: when the persisted config is missing a key
/// that the current schema declares (e.g. the hand was first activated
/// against an older HAND.toml that didn't have the key, or the hand_state.json
/// pre-dates this PR), re-activation must fill the missing key from the
/// schema default while leaving every other previously-accepted value alone.
#[test]
fn test_reactivation_backfills_missing_schema_keys() {
    let kernel = LibreFangKernel::boot_with_config(test_config("seed-backfill")).unwrap();
    install_hand(&kernel, HAND_WITH_SETTINGS);

    // Mimic restart-recovery: hand_state.json carries a partial config —
    // `max_concurrency` was accepted by the operator at "12", but `verbosity`
    // is missing entirely (older state file, or schema added the key later).
    let mut prior_config = HashMap::new();
    prior_config.insert(
        "max_concurrency".to_string(),
        serde_json::Value::String("12".to_string()),
    );

    let instance = kernel.activate_hand("test-settings", prior_config).unwrap();

    assert_eq!(
        instance
            .config
            .get("max_concurrency")
            .and_then(|v| v.as_str()),
        Some("12"),
        "previously-accepted user value must survive backfill"
    );
    assert_eq!(
        instance.config.get("verbosity").and_then(|v| v.as_str()),
        Some("normal"),
        "key absent from prior config must be backfilled with its schema default"
    );

    kernel.shutdown();
}

#[test]
fn test_multiple_hands_coexist() {
    let kernel = LibreFangKernel::boot_with_config(test_config("coexist")).unwrap();
    install_hand(&kernel, HAND_A);
    install_hand(&kernel, HAND_B);

    let clip = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let devops = kernel.activate_hand("test-devops", HashMap::new()).unwrap();

    assert!(clip.agent_id().is_some());
    assert!(devops.agent_id().is_some());
    assert_ne!(
        clip.agent_id().unwrap(),
        devops.agent_id().unwrap(),
        "Different hands should have different agent IDs"
    );

    // Both agents exist
    assert!(kernel
        .agent_registry_ref()
        .get(clip.agent_id().unwrap())
        .is_some());
    assert!(kernel
        .agent_registry_ref()
        .get(devops.agent_id().unwrap())
        .is_some());

    kernel.shutdown();
}

#[test]
fn test_deactivate_one_hand_preserves_other() {
    let kernel = LibreFangKernel::boot_with_config(test_config("preserve")).unwrap();
    install_hand(&kernel, HAND_A);
    install_hand(&kernel, HAND_B);

    let clip = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let devops = kernel.activate_hand("test-devops", HashMap::new()).unwrap();
    let devops_agent_id = devops.agent_id().unwrap();

    // Deactivate clip
    kernel.deactivate_hand(clip.instance_id).unwrap();

    // Devops agent should still be alive
    assert!(
        kernel.agent_registry_ref().get(devops_agent_id).is_some(),
        "DevOps agent should survive clip deactivation"
    );

    kernel.shutdown();
}

#[test]
fn test_find_instance_by_agent_id() {
    let kernel = LibreFangKernel::boot_with_config(test_config("find-by-agent")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    let found = kernel.hand_registry_ref().find_by_agent(agent_id);
    assert!(found.is_some(), "Should find instance by agent ID");
    assert_eq!(found.unwrap().instance_id, instance.instance_id);

    // Random agent ID should not find any instance
    let random_id = AgentId::from_hand_id("nonexistent");
    assert!(kernel
        .hand_registry_ref()
        .find_by_agent(random_id)
        .is_none());

    kernel.shutdown();
}

#[test]
fn test_agent_id_from_hand_id_is_deterministic() {
    // Pure unit test — no kernel needed
    let id1 = AgentId::from_hand_id("clip");
    let id2 = AgentId::from_hand_id("clip");
    let id3 = AgentId::from_hand_id("devops");

    assert_eq!(id1, id2, "Same hand_id should produce same ID");
    assert_ne!(id1, id3, "Different hand_ids should produce different IDs");
}

#[test]
fn test_system_prompt_preserved() {
    let kernel = LibreFangKernel::boot_with_config(test_config("prompt")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    let entry = kernel.agent_registry_ref().get(agent_id).unwrap();
    assert!(
        entry.manifest.model.system_prompt.contains("clip agent"),
        "System prompt should contain the hand's prompt"
    );

    kernel.shutdown();
}

#[test]
fn test_default_provider_resolved_to_kernel_default() {
    let tc = test_config("provider");
    let kernel = LibreFangKernel::boot_with_config(tc).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    let agent_id = instance.agent_id().unwrap();

    let entry = kernel.agent_registry_ref().get(agent_id).unwrap();
    // Activation resolves the default provider sentinel against the effective
    // kernel config. The effective provider may differ from the test config's
    // initial value when the primary driver fails and auto-detect kicks in
    // (e.g. groq with no API key → deepseek auto-detected from env). Either
    // way, the sentinel must NOT remain as the literal string "default".
    assert_ne!(
        entry.manifest.model.provider, "default",
        "Provider should be resolved from kernel config, not left as 'default'"
    );
    // Model resolution depends on the auto-detected provider having a catalog
    // entry; in environments where it doesn't, the model may legitimately be
    // the string "default" as a deferred sentinel. Skip asserting on model
    // since this test is specifically about provider resolution.

    kernel.shutdown();
}

#[test]
fn test_hand_instance_status_active_on_creation() {
    let kernel = LibreFangKernel::boot_with_config(test_config("status")).unwrap();
    install_hand(&kernel, HAND_A);

    let instance = kernel.activate_hand("test-clip", HashMap::new()).unwrap();
    assert_eq!(instance.status.to_string(), "Active");

    kernel.shutdown();
}

#[test]
fn test_pause_nonexistent_instance_fails() {
    let kernel = LibreFangKernel::boot_with_config(test_config("pause-none")).unwrap();

    let fake_id = uuid::Uuid::new_v4();
    let result = kernel.pause_hand(fake_id);
    assert!(result.is_err(), "Pausing nonexistent instance should fail");

    kernel.shutdown();
}

#[test]
fn test_resume_nonexistent_instance_fails() {
    let kernel = LibreFangKernel::boot_with_config(test_config("resume-none")).unwrap();

    let fake_id = uuid::Uuid::new_v4();
    let result = kernel.resume_hand(fake_id);
    assert!(result.is_err(), "Resuming nonexistent instance should fail");

    kernel.shutdown();
}

#[test]
fn test_reactivation_restores_triggers_to_original_roles() {
    let kernel = LibreFangKernel::boot_with_config(test_config("trigger-reactivation")).unwrap();
    install_hand(&kernel, HAND_C);

    let instance = kernel
        .activate_hand("test-research", HashMap::new())
        .unwrap();
    let analyst_id = *instance
        .agent_ids
        .get("analyst")
        .expect("analyst role agent id");
    let planner_id = *instance
        .agent_ids
        .get("planner")
        .expect("planner role agent id");

    kernel
        .register_trigger(
            analyst_id,
            TriggerPattern::System,
            "wake analyst".to_string(),
            0,
        )
        .unwrap();
    assert_eq!(kernel.list_triggers(Some(analyst_id)).len(), 1);

    // Remove the instance entry without killing the agents to force the
    // activation path to clean up and migrate the stale hand agents.
    kernel
        .hand_registry_ref()
        .deactivate(instance.instance_id)
        .unwrap();

    let reactivated = kernel
        .activate_hand("test-research", HashMap::new())
        .unwrap();
    let reactivated_analyst_id = *reactivated
        .agent_ids
        .get("analyst")
        .expect("reactivated analyst role agent id");
    let reactivated_planner_id = *reactivated
        .agent_ids
        .get("planner")
        .expect("reactivated planner role agent id");

    // Single-instance reactivation uses legacy format — same hand+role = same ID.
    assert_eq!(
        reactivated_analyst_id, analyst_id,
        "Reactivated analyst keeps same agent ID (legacy format)"
    );
    assert_eq!(
        reactivated_planner_id, planner_id,
        "Reactivated planner keeps same agent ID (legacy format)"
    );
    assert_eq!(
        kernel.list_triggers(Some(reactivated_analyst_id)).len(),
        1,
        "Analyst triggers should stay attached to the analyst role after reactivation"
    );
    assert!(
        kernel
            .list_triggers(Some(reactivated_planner_id))
            .is_empty(),
        "Planner should not inherit analyst triggers during reactivation"
    );

    kernel.deactivate_hand(reactivated.instance_id).unwrap();
    kernel.shutdown();
}

// ── Live LLM integration test (requires GROQ_API_KEY) ───────────────────────

fn load_manifest(toml_str: &str) -> AgentManifest {
    toml::from_str(toml_str).expect("Should parse manifest")
}

#[tokio::test]
async fn test_six_agent_fleet() {
    if std::env::var("GROQ_API_KEY").is_err() {
        eprintln!("GROQ_API_KEY not set, skipping multi-agent test");
        return;
    }

    let kernel =
        LibreFangKernel::boot_with_config(test_config("fleet")).expect("Kernel should boot");

    let agents = vec![
        (
            "coder",
            r#"
name = "coder"
module = "builtin:chat"
[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are Coder. Reply with 'CODER:' prefix. Be concise."
[capabilities]
tools = ["file_read", "file_write"]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            "Write a one-line Rust function that adds two numbers.",
        ),
        (
            "researcher",
            r#"
name = "researcher"
module = "builtin:chat"
[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are Researcher. Reply with 'RESEARCHER:' prefix. Be concise."
[capabilities]
tools = ["web_fetch"]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            "What is Rust's primary advantage over C++? One sentence.",
        ),
        (
            "writer",
            r#"
name = "writer"
module = "builtin:chat"
[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are Writer. Reply with 'WRITER:' prefix. Be concise."
[capabilities]
tools = ["file_read", "file_write"]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            "Write a one-sentence tagline for an Agent Operating System.",
        ),
        (
            "ops",
            r#"
name = "ops"
module = "builtin:chat"
[model]
provider = "groq"
model = "llama-3.1-8b-instant"
system_prompt = "You are Ops. Reply with 'OPS:' prefix. Be concise."
[capabilities]
tools = ["shell_exec"]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            "What would you check first if a server is running slowly?",
        ),
        (
            "analyst",
            r#"
name = "analyst"
module = "builtin:chat"
[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are Analyst. Reply with 'ANALYST:' prefix. Be concise."
[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            "What are the top 3 metrics to track for an API service?",
        ),
        (
            "hello-world",
            r#"
name = "hello-world"
module = "builtin:chat"
[model]
provider = "groq"
model = "llama-3.1-8b-instant"
system_prompt = "You are a friendly greeter. Reply with 'HELLO:' prefix. Be concise."
[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            "Greet the user in a fun way.",
        ),
    ];

    println!("\n{}", "=".repeat(60));
    println!("  LIBREFANG MULTI-AGENT FLEET TEST");
    println!("  Spawning {} agents...", agents.len());
    println!("{}\n", "=".repeat(60));

    let mut agent_ids = Vec::new();
    for (name, manifest_str, _) in &agents {
        let manifest = load_manifest(manifest_str);
        let id = kernel
            .spawn_agent(manifest)
            .unwrap_or_else(|e| panic!("Failed to spawn {name}: {e}"));
        println!("  Spawned: {name:<12} -> {id}");
        agent_ids.push(id);
    }

    assert_eq!(
        kernel.agent_registry_ref().count(),
        6,
        "Should have 6 agents"
    );
    println!(
        "\n  All {} agents spawned. Sending messages...\n",
        agents.len()
    );

    let mut results = Vec::new();
    for (i, (name, _, message)) in agents.iter().enumerate() {
        let result = kernel
            .send_message(agent_ids[i], message)
            .await
            .unwrap_or_else(|e| panic!("Failed to message {name}: {e}"));

        println!("--- {name} ---");
        println!("  Q: {message}");
        println!("  A: {}", result.response);
        println!(
            "  [{} tokens in, {} tokens out, {} iters]",
            result.total_usage.input_tokens, result.total_usage.output_tokens, result.iterations
        );
        println!();

        assert!(
            !result.response.is_empty(),
            "{name} response should not be empty"
        );
        results.push(result);
    }

    let total_input: u64 = results.iter().map(|r| r.total_usage.input_tokens).sum();
    let total_output: u64 = results.iter().map(|r| r.total_usage.output_tokens).sum();
    println!("============================================================");
    println!("  FLEET SUMMARY");
    println!("  Agents:       {}", agents.len());
    println!("  Total input:  {} tokens", total_input);
    println!("  Total output: {} tokens", total_output);
    println!("  All responded: YES");
    println!("============================================================");

    for id in agent_ids {
        kernel.kill_agent(id).unwrap();
    }
    kernel.shutdown();
}
