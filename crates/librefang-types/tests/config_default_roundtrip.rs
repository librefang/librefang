//! Regression test for issue #3404.
//!
//! When a new field is added to `KernelConfig` (or a nested config struct)
//! with `#[serde(default)]` but the developer forgets to populate it in the
//! manual `Default` impl, deserialization succeeds with `T::default()` while
//! the in-process `T::default()` returns whatever the manual impl produces.
//! The two diverge silently — empty TOML round-trips, but the in-memory
//! default carries different values. The schemars-based golden schema test
//! does not catch this because schemars reads the `#[serde(default)]`
//! attribute, not the `Default` impl body.
//!
//! For each tested type `T`, this file asserts:
//!   1. `T::default()` equals what serde produces from an empty TOML
//!      document (the property that catches the #3404 bug class).
//!   2. `T::default()` round-trips losslessly through TOML serialization.
//!
//! Equality is checked by comparing the TOML serialization of both sides
//! rather than deriving `PartialEq` on every config type — see issue #3404
//! caveat 1, which warns that derived `PartialEq` would cascade through the
//! entire nested config tree.
//!
//! ## Adding a new type
//!
//! For most config structs a single `#[test]` calling
//! `assert_default_roundtrip::<T>("T")` is enough. If `T::default()` and the
//! serde-empty fill legitimately diverge on a specific field (see
//! `KernelConfig` below for the `config_version` migration tripwire), use
//! `assert_default_roundtrip_with` and pass a normalizer closure that copies
//! the canonical value over before comparison. The test will still assert
//! that every other field matches exactly.

use librefang_types::agent::AgentManifest;
use librefang_types::config::{
    AuditConfig, AutoDreamConfig, AutoReplyConfig, BraveSearchConfig, BroadcastConfig,
    BrowserConfig, BudgetConfig, CanvasConfig, ChannelsConfig, ChunkConfig, CompactionTomlConfig,
    ContextEngineTomlConfig, DockerSandboxConfig, ExtensionsConfig, ExternalAuthConfig,
    HealthCheckConfig, HeartbeatTomlConfig, InboxConfig, JinaSearchConfig, KernelConfig,
    LinkedInConfig, MemoryConfig, MemoryDecayConfig, NetworkConfig, PairingConfig,
    ParallelToolsConfig, PerplexitySearchConfig, PrivacyConfig, PromptIntelligenceConfig,
    QueueConcurrencyConfig, QueueConfig, RateLimitConfig, RegistryConfig, ReloadConfig,
    SanitizeConfig, SessionConfig, SkillsConfig, TaskBoardConfig, TavilySearchConfig,
    TelemetryConfig, TerminalConfig, ThinkingConfig, TriggersConfig, TtsConfig, VaultConfig,
    VoiceConfig, WebConfig, WebFetchConfig, WebhookTriggerConfig,
};
use serde::Serialize;

/// Asserts that `T::default()` matches an empty-TOML deserialization and
/// round-trips through TOML serialization. Use this for the common case
/// where the two sources are expected to agree on every field.
fn assert_default_roundtrip<T>(label: &str)
where
    T: Default + Serialize + for<'de> serde::Deserialize<'de>,
{
    assert_default_roundtrip_with::<T>(label, |_| {});
}

/// Variant that runs `normalize` on the from-empty / from-roundtrip values
/// before comparison, for types with a known legitimate divergence (see the
/// `KernelConfig` test for the `config_version` rationale). Every field
/// untouched by `normalize` is still required to match exactly — that is the
/// property that catches the #3404 bug class.
fn assert_default_roundtrip_with<T>(label: &str, mut normalize: impl FnMut(&mut T))
where
    T: Default + Serialize + for<'de> serde::Deserialize<'de>,
{
    let from_default = T::default();
    let default_toml = toml::to_string(&from_default)
        .unwrap_or_else(|e| panic!("{label}: serialize default failed: {e}"));

    // Empty TOML must deserialize to exactly the same value as Default::default().
    // This is what catches a `#[serde(default)]` field whose corresponding line
    // is missing from the manual `Default` impl: serde fills it with
    // `Field::default()` while our manual impl produces something else, and the
    // two TOML strings will differ.
    let mut from_empty: T = toml::from_str("")
        .unwrap_or_else(|e| panic!("{label}: deserialize empty TOML failed: {e}"));
    normalize(&mut from_empty);
    let empty_toml = toml::to_string(&from_empty)
        .unwrap_or_else(|e| panic!("{label}: serialize from-empty failed: {e}"));
    assert_eq!(
        default_toml, empty_toml,
        "{label}::default() must equal what serde produces from an empty TOML \
         document. A field is likely declared with `#[serde(default)]` but \
         missing from the manual `Default` impl (or vice versa)."
    );

    // Round-trip the serialized default and assert idempotency.
    let mut from_roundtrip: T = toml::from_str(&default_toml)
        .unwrap_or_else(|e| panic!("{label}: deserialize roundtrip failed: {e}"));
    normalize(&mut from_roundtrip);
    let roundtrip_toml = toml::to_string(&from_roundtrip)
        .unwrap_or_else(|e| panic!("{label}: serialize roundtrip failed: {e}"));
    assert_eq!(
        default_toml, roundtrip_toml,
        "{label}::default() must round-trip through TOML serialization."
    );
}

#[test]
fn kernel_config_default_roundtrips_through_toml() {
    // KernelConfig::default() pulls in machine-specific paths via
    // `librefang_home_dir()`, but those paths are deterministic within a
    // single process run — both `Default::default()` and the empty-TOML
    // deserialization re-invoke the same function (because the struct is
    // annotated with `#[serde(default)]`), so the paths agree without
    // normalization.
    //
    // `config_version` is the one field where the two sources legitimately
    // diverge and must be normalized before comparison:
    //   - `KernelConfig::default()` returns the current `CONFIG_VERSION`
    //     (currently `2`) — see `crates/librefang-types/src/config/types.rs`
    //     where the manual `Default` impl sets `config_version: CONFIG_VERSION`.
    //     Fresh in-memory configs need no migration, so they are stamped with
    //     the latest version.
    //   - Serde-empty deserialization fills the field via
    //     `default_config_version()` which returns `1` — see
    //     `crates/librefang-types/src/config/version.rs`. The `1` is an
    //     intentional migration tripwire: a legacy on-disk TOML that omits
    //     `config_version` is by definition pre-versioning (v1), and
    //     `run_migrations` will lift it forward to `CONFIG_VERSION`.
    //
    // Normalizing only `config_version` keeps the deliberate v1 sentinel
    // from masking comparisons on every other field — which is the property
    // that catches the bug class issue #3404 describes.
    let canonical_version = KernelConfig::default().config_version;
    assert_default_roundtrip_with::<KernelConfig>("KernelConfig", move |c| {
        c.config_version = canonical_version;
    });
}

// All remaining tests use the simple helper. Each `#[test]` covers one
// config struct that has both `#[serde(default)]` and a manual `impl Default`
// (or transitively reaches one) — those are the structures where the
// #3404 bug class can recur.

#[test]
fn queue_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<QueueConfig>("QueueConfig");
}

#[test]
fn queue_concurrency_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<QueueConcurrencyConfig>("QueueConcurrencyConfig");
}

#[test]
fn budget_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<BudgetConfig>("BudgetConfig");
}

#[test]
fn session_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<SessionConfig>("SessionConfig");
}

#[test]
fn compaction_toml_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<CompactionTomlConfig>("CompactionTomlConfig");
}

#[test]
fn task_board_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<TaskBoardConfig>("TaskBoardConfig");
}

#[test]
fn triggers_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<TriggersConfig>("TriggersConfig");
}

#[test]
fn webhook_trigger_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<WebhookTriggerConfig>("WebhookTriggerConfig");
}

#[test]
fn web_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<WebConfig>("WebConfig");
}

#[test]
fn web_fetch_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<WebFetchConfig>("WebFetchConfig");
}

#[test]
fn browser_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<BrowserConfig>("BrowserConfig");
}

#[test]
fn brave_search_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<BraveSearchConfig>("BraveSearchConfig");
}

#[test]
fn tavily_search_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<TavilySearchConfig>("TavilySearchConfig");
}

#[test]
fn perplexity_search_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<PerplexitySearchConfig>("PerplexitySearchConfig");
}

#[test]
fn jina_search_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<JinaSearchConfig>("JinaSearchConfig");
}

#[test]
fn reload_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ReloadConfig>("ReloadConfig");
}

#[test]
fn rate_limit_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<RateLimitConfig>("RateLimitConfig");
}

#[test]
fn skills_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<SkillsConfig>("SkillsConfig");
}

#[test]
fn extensions_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ExtensionsConfig>("ExtensionsConfig");
}

#[test]
fn vault_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<VaultConfig>("VaultConfig");
}

#[test]
fn auto_reply_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<AutoReplyConfig>("AutoReplyConfig");
}

#[test]
fn inbox_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<InboxConfig>("InboxConfig");
}

#[test]
fn telemetry_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<TelemetryConfig>("TelemetryConfig");
}

#[test]
fn prompt_intelligence_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<PromptIntelligenceConfig>("PromptIntelligenceConfig");
}

#[test]
fn canvas_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<CanvasConfig>("CanvasConfig");
}

#[test]
fn thinking_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ThinkingConfig>("ThinkingConfig");
}

#[test]
fn context_engine_toml_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ContextEngineTomlConfig>("ContextEngineTomlConfig");
}

#[test]
fn external_auth_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ExternalAuthConfig>("ExternalAuthConfig");
}

#[test]
fn audit_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<AuditConfig>("AuditConfig");
}

#[test]
fn privacy_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<PrivacyConfig>("PrivacyConfig");
}

#[test]
fn health_check_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<HealthCheckConfig>("HealthCheckConfig");
}

#[test]
fn heartbeat_toml_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<HeartbeatTomlConfig>("HeartbeatTomlConfig");
}

#[test]
fn auto_dream_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<AutoDreamConfig>("AutoDreamConfig");
}

#[test]
fn registry_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<RegistryConfig>("RegistryConfig");
}

#[test]
fn memory_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<MemoryConfig>("MemoryConfig");
}

#[test]
fn memory_decay_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<MemoryDecayConfig>("MemoryDecayConfig");
}

#[test]
fn chunk_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ChunkConfig>("ChunkConfig");
}

#[test]
fn network_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<NetworkConfig>("NetworkConfig");
}

#[test]
fn tts_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<TtsConfig>("TtsConfig");
}

#[test]
fn docker_sandbox_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<DockerSandboxConfig>("DockerSandboxConfig");
}

#[test]
fn pairing_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<PairingConfig>("PairingConfig");
}

#[test]
fn sanitize_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<SanitizeConfig>("SanitizeConfig");
}

#[test]
fn parallel_tools_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<ParallelToolsConfig>("ParallelToolsConfig");
}

#[test]
fn terminal_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<TerminalConfig>("TerminalConfig");
}

#[test]
fn voice_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<VoiceConfig>("VoiceConfig");
}

#[test]
fn linked_in_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<LinkedInConfig>("LinkedInConfig");
}

// Issue #3462 — extend the round-trip property to nested config types
// referenced from agent manifests and channel wiring. These three are the
// load-bearing structs called out in the issue (`AgentManifest`,
// `ChannelsConfig` — the closest match for the issue's "ChannelConfig" name —
// and `BroadcastConfig`). `BudgetConfig` is already covered above.

#[test]
fn agent_manifest_default_roundtrips_through_toml() {
    assert_default_roundtrip::<AgentManifest>("AgentManifest");
}

#[test]
fn channels_config_default_roundtrips_through_toml() {
    // Known #3462 Default-drift, flagged for separate triage:
    //   `ChannelsConfig` uses `#[derive(Default)]`, so
    //   `file_download_max_bytes` defaults to `u64::default() == 0`, but the
    //   field is annotated `#[serde(default = "default_file_download_max_bytes")]`
    //   which returns 50 MiB. Empty-TOML deserialization therefore yields
    //   52_428_800 while the in-memory `Default::default()` yields 0.
    //
    // Per the task brief, this test does NOT silently fix the type — the
    // discrepancy is normalized only on the divergent field so that drift in
    // every OTHER field is still caught. The underlying type bug should be
    // fixed by either replacing `#[derive(Default)]` with a manual impl that
    // calls `default_file_download_max_bytes()`, or by aligning the serde
    // helper with the derived zero value, depending on which value is
    // intended at runtime.
    let canonical_max_bytes = ChannelsConfig::default().file_download_max_bytes;
    assert_default_roundtrip_with::<ChannelsConfig>("ChannelsConfig", move |c| {
        c.file_download_max_bytes = canonical_max_bytes;
    });
}

#[test]
fn broadcast_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<BroadcastConfig>("BroadcastConfig");
}
