//! Field-by-field parity guard for `AgentManifest` (refs #7742).
//!
//! `PATCH /api/agents/{id}` with a `manifest_toml` body is the only surface that reaches every
//! manifest field, and the kernel persists the result by handing the whole struct to
//! `toml::to_string_pretty` (`kernel/agent_state.rs: persist_full_manifest_at`).
//! That makes the TOML round trip the real definition of "editable through the API": a field that
//! does not survive it is a field an operator cannot change, whatever the route accepts.
//!
//! Two properties are asserted here.
//! Every field, set to a value distinct from its default, comes back unchanged through
//! serialize-then-parse.
//! And a manifest whose optional free-form JSON is absent still serializes at all — the failure
//! mode is not one lost field but a `to_string_pretty` error that aborts the write for all 59.

use librefang_types::agent::{
    AgentManifest, AsyncTasksConfig, AutonomousConfig, CompactionOverrides, FallbackModel,
    ManifestCapabilities, ManifestTrigger, ModelConfig, ModelRoutingConfig, OrphanPolicy, Priority,
    ResourceQuota, RlExportOverride, ScheduleMode, SessionMode, SkillWorkshopConfig, ToolConfig,
    ToolProfile, WebSearchAugmentationMode, WorkspaceDecl,
};
use librefang_types::config::{ContextInjection, InjectionPosition};
use librefang_types::memory::ProactiveMemoryOverrides;
use librefang_types::tool_exec::BackendKind;
use std::collections::HashMap;
use std::path::PathBuf;

/// Every field of `AgentManifest` set to something a `Default::default()` manifest does not have.
///
/// Written as one exhaustive struct literal with no `..Default::default()` at the top level, so the
/// compiler — not a reviewer — is what notices a 59th field being added.
/// The point of the fixture is that a field whose type cannot survive TOML breaks
/// `every_manifest_field_survives_a_toml_round_trip` here, rather than in production on the first
/// `agent.toml` write, where the failure is a swallowed log and a frozen file.
fn fully_populated_manifest() -> AgentManifest {
    AgentManifest {
        name: "parity".into(),
        version: "9.9.9".into(),
        description: "a manifest with nothing left at its default".into(),
        author: "houko".into(),
        owner: Some("group:platform".into()),
        module: "builtin:chat".into(),
        schedule: ScheduleMode::Periodic {
            cron: "0 9 * * *".into(),
        },
        session_mode: SessionMode::New,
        model: ModelConfig {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            max_tokens: 4242,
            temperature: 0.42,
            system_prompt: "you are a parity fixture".into(),
            api_key_env: Some("PARITY_KEY".into()),
            base_url: Some("https://example.invalid/v1".into()),
            context_window: Some(131_072),
            max_output_tokens: Some(8_192),
            ..Default::default()
        },
        fallback_models: Some(vec![FallbackModel {
            provider: "anthropic".into(),
            model: "fallback-model".into(),
            ..Default::default()
        }]),
        resources: ResourceQuota {
            max_memory_bytes: 7,
            max_cpu_time_ms: 8,
            max_tool_calls_per_minute: 9,
            max_llm_tokens_per_hour: Some(10),
            max_cost_per_hour_usd: 1.5,
            ..Default::default()
        },
        priority: Priority::Critical,
        capabilities: ManifestCapabilities {
            network: vec!["example.invalid:443".into()],
            tools: vec!["file_read".into()],
            agent_spawn: true,
            memory_read: Some(vec!["user/*".into()]),
            memory_write: Some(vec!["user/*".into()]),
            ..Default::default()
        },
        profile: Some(ToolProfile::Coding),
        tools: HashMap::from([(
            "shell_exec".to_string(),
            ToolConfig {
                params: HashMap::from([("timeout_secs".to_string(), serde_json::json!(30))]),
            },
        )]),
        skills: vec!["coder".into()],
        skills_disabled: true,
        mcp_servers: vec!["filesystem".into()],
        channels: vec!["telegram".into()],
        mcp_disabled: true,
        metadata: HashMap::from([("owner".to_string(), serde_json::json!("platform"))]),
        tags: vec!["research".into()],
        routing: Some(ModelRoutingConfig::default()),
        autonomous: Some(AutonomousConfig::default()),
        pinned_model: Some("gpt-4o-2024-11-20".into()),
        workspace: Some(PathBuf::from("/tmp/parity-workspace")),
        generate_identity_files: false,
        workspaces: HashMap::from([(
            "library".to_string(),
            WorkspaceDecl {
                path: Some(PathBuf::from("shared/library")),
                ..Default::default()
            },
        )]),
        exec_policy: Some(Default::default()),
        tool_allowlist: vec!["file_read".into()],
        tool_blocklist: vec!["shell_exec".into()],
        tools_disabled: true,
        response_format: Some(Default::default()),
        enabled: false,
        allowed_plugins: vec!["notes".into()],
        inherit_parent_context: false,
        thinking: Some(Default::default()),
        context_injection: vec![ContextInjection {
            name: "house-style".into(),
            content: "answer in one paragraph".into(),
            position: InjectionPosition::BeforeUser,
            condition: Some("always".into()),
        }],
        is_hand: true,
        web_search_augmentation: WebSearchAugmentationMode::Always,
        auto_dream_enabled: true,
        auto_dream_min_hours: Some(3.5),
        auto_dream_min_sessions: Some(4),
        show_progress: false,
        auto_evolve: false,
        channel_overrides: Some(Default::default()),
        max_history_messages: Some(99),
        max_concurrent_invocations: Some(3),
        assignee_wake: Some(false),
        cache_context: true,
        tool_exec_backend: Some(BackendKind::Docker),
        skill_workshop: SkillWorkshopConfig {
            enabled: true,
            ..Default::default()
        },
        proactive_memory: ProactiveMemoryOverrides {
            enabled: Some(false),
            ..Default::default()
        },
        compaction: Some(CompactionOverrides {
            keep_recent: Some(5),
            ..Default::default()
        }),
        context_engine: Some(Default::default()),
        rl_export: RlExportOverride {
            enabled: Some(true),
        },
        triggers: vec![ManifestTrigger {
            pattern: serde_json::json!({"type": "task_posted"}),
            prompt_template: "New task: {{event}}".into(),
            max_fires: 5,
            cooldown_secs: 60,
            session_mode: Some(SessionMode::New),
            target_agent: Some("other".into()),
            workflow_id: Some("wf-1".into()),
            enabled: false,
        }],
        reconcile_orphans: OrphanPolicy::Delete,
        async_tasks: AsyncTasksConfig {
            default_timeout_secs: Some(60),
            notify_on_timeout: false,
        },
    }
}

/// Compare one field on both sides of the round trip, collecting rather than panicking so a run
/// reports every dropped field at once instead of only the first.
macro_rules! compare_field {
    ($dropped:ident, $before:ident, $after:ident, $field:ident) => {
        let before = format!("{:?}", $before.$field);
        let after = format!("{:?}", $after.$field);
        if before != after {
            $dropped.push(format!(
                "{}: wrote {before}, read back {after}",
                stringify!($field)
            ));
        }
    };
}

#[test]
fn every_manifest_field_survives_a_toml_round_trip() {
    let before = fully_populated_manifest();
    let toml_text = toml::to_string_pretty(&before).expect(
        "a fully populated manifest must be serializable, or no field on it is persistable",
    );
    let after: AgentManifest =
        toml::from_str(&toml_text).expect("the kernel's own output must parse back");

    let mut dropped: Vec<String> = Vec::new();

    compare_field!(dropped, before, after, name);
    compare_field!(dropped, before, after, version);
    compare_field!(dropped, before, after, description);
    compare_field!(dropped, before, after, author);
    compare_field!(dropped, before, after, owner);
    compare_field!(dropped, before, after, module);
    compare_field!(dropped, before, after, schedule);
    compare_field!(dropped, before, after, session_mode);
    compare_field!(dropped, before, after, model);
    compare_field!(dropped, before, after, fallback_models);
    compare_field!(dropped, before, after, resources);
    compare_field!(dropped, before, after, priority);
    compare_field!(dropped, before, after, capabilities);
    compare_field!(dropped, before, after, profile);
    compare_field!(dropped, before, after, tools);
    compare_field!(dropped, before, after, skills);
    compare_field!(dropped, before, after, skills_disabled);
    compare_field!(dropped, before, after, mcp_servers);
    compare_field!(dropped, before, after, channels);
    compare_field!(dropped, before, after, mcp_disabled);
    compare_field!(dropped, before, after, metadata);
    compare_field!(dropped, before, after, tags);
    compare_field!(dropped, before, after, routing);
    compare_field!(dropped, before, after, autonomous);
    compare_field!(dropped, before, after, pinned_model);
    compare_field!(dropped, before, after, workspace);
    compare_field!(dropped, before, after, generate_identity_files);
    compare_field!(dropped, before, after, workspaces);
    compare_field!(dropped, before, after, exec_policy);
    compare_field!(dropped, before, after, tool_allowlist);
    compare_field!(dropped, before, after, tool_blocklist);
    compare_field!(dropped, before, after, tools_disabled);
    compare_field!(dropped, before, after, response_format);
    compare_field!(dropped, before, after, enabled);
    compare_field!(dropped, before, after, allowed_plugins);
    compare_field!(dropped, before, after, inherit_parent_context);
    compare_field!(dropped, before, after, thinking);
    compare_field!(dropped, before, after, context_injection);
    compare_field!(dropped, before, after, is_hand);
    compare_field!(dropped, before, after, web_search_augmentation);
    compare_field!(dropped, before, after, auto_dream_enabled);
    compare_field!(dropped, before, after, auto_dream_min_hours);
    compare_field!(dropped, before, after, auto_dream_min_sessions);
    compare_field!(dropped, before, after, show_progress);
    compare_field!(dropped, before, after, auto_evolve);
    compare_field!(dropped, before, after, channel_overrides);
    compare_field!(dropped, before, after, max_history_messages);
    compare_field!(dropped, before, after, max_concurrent_invocations);
    compare_field!(dropped, before, after, assignee_wake);
    compare_field!(dropped, before, after, cache_context);
    compare_field!(dropped, before, after, tool_exec_backend);
    compare_field!(dropped, before, after, skill_workshop);
    compare_field!(dropped, before, after, proactive_memory);
    compare_field!(dropped, before, after, compaction);
    compare_field!(dropped, before, after, context_engine);
    compare_field!(dropped, before, after, rl_export);
    compare_field!(dropped, before, after, triggers);
    compare_field!(dropped, before, after, reconcile_orphans);
    compare_field!(dropped, before, after, async_tasks);

    assert!(
        dropped.is_empty(),
        "these manifest fields did not survive the TOML round trip the kernel persists through:\n{}",
        dropped.join("\n")
    );
}

/// Guards the count itself, so a new field cannot be added without someone deciding whether it
/// belongs in the round trip above.
///
/// Counting the keys a fully populated manifest emits is the closest thing to reflection available
/// here — a field left at its default with `skip_serializing_if` would be invisible, which is why
/// the fixture sets all of them.
#[test]
fn the_populated_fixture_covers_every_serialized_manifest_key() {
    let toml_text = toml::to_string_pretty(&fully_populated_manifest()).expect("serializable");
    let table: toml::Table = toml::from_str(&toml_text).expect("parses as a table");
    assert_eq!(
        table.len(),
        59,
        "AgentManifest emitted {} top-level keys, not the 59 this parity sweep enumerated. \
         Add the new field to fully_populated_manifest() and to the round-trip assertions, \
         then update docs/architecture/agent-manifest-field-parity.md.",
        table.len()
    );
}

/// The regression that motivated the whole guard.
///
/// A `[[triggers]]` block that omits `pattern` is accepted by the deserializer (struct-level
/// `#[serde(default)]` fills it with `Value::Null`) and is inert at reconcile time, but TOML cannot
/// represent null.
/// Before #7742 that turned every later `agent.toml` write for the agent into a logged-and-swallowed
/// `unsupported unit type`, so the manifest on disk froze while the in-memory copy accepted edits —
/// and boot reconciliation then restored the frozen file over them.
#[test]
fn a_trigger_with_no_pattern_does_not_block_manifest_serialization() {
    let authored = "name = \"forgot-the-pattern\"\n\n[[triggers]]\nprompt_template = \"go\"\n";
    let m: AgentManifest = toml::from_str(authored).expect("an omitted pattern still parses");
    assert!(
        m.triggers[0].pattern.is_null(),
        "fixture must reproduce the null-pattern state, not a normalized one"
    );

    let serialized = toml::to_string_pretty(&m)
        .expect("a null trigger pattern must not make the whole manifest unwritable");

    let back: AgentManifest = toml::from_str(&serialized).expect("round trips");
    assert_eq!(back.name, "forgot-the-pattern");
    assert_eq!(back.triggers.len(), 1, "the trigger entry is not dropped");
    assert!(
        back.triggers[0].pattern.is_null(),
        "and it reads back in the same inert state the kernel skips with a warning"
    );
    assert_eq!(back.triggers[0].prompt_template, "go");
}
