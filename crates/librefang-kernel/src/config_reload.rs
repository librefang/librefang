//! Config hot-reload — diffs two `KernelConfig` instances and produces a `ReloadPlan`.
//!
//! **Hot-reload safe**: channels, skills, usage footer, web config, browser,
//! approval policy, cron settings, webhook triggers, extensions, tool policy,
//! api_key, dashboard credentials, stable_prefix_mode, proxy, provider_api_keys,
//! sanitize, default model, language, mode, log_level (when a
//! [`crate::log_reload::LogLevelReloader`] is installed by the binary).
//!
//! **Restart required**: api_listen, network, memory, home_dir, data_dir, vault.

use librefang_types::config::{KernelConfig, ReloadMode};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// HotAction — what can be changed at runtime without restart
// ---------------------------------------------------------------------------

/// An individual action that can be applied at runtime (hot-reload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotAction {
    /// Channel configuration changed — reload channel bridges.
    ReloadChannels,
    /// Skill configuration changed — reload skill registry.
    ReloadSkills,
    /// Usage footer mode changed.
    UpdateUsageFooter,
    /// Web config changed — rebuild web tools context.
    ReloadWebConfig,
    /// Browser config changed.
    ReloadBrowserConfig,
    /// Approval policy changed.
    UpdateApprovalPolicy,
    /// Cron max jobs changed.
    UpdateCronConfig,
    /// Webhook trigger config changed.
    UpdateWebhookConfig,
    /// Extension config changed.
    ReloadExtensions,
    /// MCP server list changed — reconnect MCP clients.
    ReloadMcpServers,
    /// A2A config changed.
    ReloadA2aConfig,
    /// Fallback provider chain changed.
    ReloadFallbackProviders,
    /// Provider base URL overrides changed.
    ReloadProviderUrls,
    /// Default model changed — update in-place without restart.
    UpdateDefaultModel,
    /// Tool policy changed — update tool filtering rules.
    UpdateToolPolicy,
    /// Proactive memory config changed — update thresholds/toggles in-place.
    UpdateProactiveMemory,
    /// Provider API keys changed — flush driver cache.
    ReloadProviderApiKeys,
    /// Proxy config changed — reinitialize HTTP proxy env.
    ReloadProxy,
    /// Dashboard credentials (user/pass/hash) changed — config swap is sufficient.
    UpdateDashboardCredentials,
    /// `[[taint_rules]]` registry changed — push the new rule sets into
    /// the kernel's shared `taint_rules_swap` so connected MCP servers see
    /// them on the next scan call without a reconnect.
    ReloadTaintRules,
    /// `log_level` changed — swap the live tracing `EnvFilter`. Carries the
    /// new directive string (e.g. `"debug"`, `"librefang_kernel=trace,info"`)
    /// since the kernel doesn't keep the parsed filter around.
    ReloadLogLevel(String),
    /// `[[users]]` or `[tool_policy.groups]` changed — rebuild the RBAC
    /// `AuthManager` so per-user `tool_policy` / `memory_access` /
    /// `channel_tool_rules` edits take effect on the next tool call
    /// (RBAC M3, #3054). Without this action, design decision #7
    /// ("invalidate per-user permission cache on config reload") is
    /// silently violated — the dashboard reports "applied" while the
    /// previous policy is still being enforced.
    ReloadAuth,
}

// ---------------------------------------------------------------------------
// ReloadPlan — the output of diffing two configs
// ---------------------------------------------------------------------------

/// A categorized plan for applying config changes.
///
/// After building a plan via [`build_reload_plan`], callers inspect
/// `restart_required` to decide whether a full restart is needed or
/// the `hot_actions` can be applied in-place.
#[derive(Debug, Clone)]
pub struct ReloadPlan {
    /// Whether a full restart is needed.
    pub restart_required: bool,
    /// Human-readable reasons why restart is required.
    pub restart_reasons: Vec<String>,
    /// Actions that can be hot-reloaded without restart.
    pub hot_actions: Vec<HotAction>,
    /// Fields that changed but are no-ops (informational only).
    pub noop_changes: Vec<String>,
}

impl ReloadPlan {
    /// Whether any changes were detected at all.
    pub fn has_changes(&self) -> bool {
        self.restart_required || !self.hot_actions.is_empty() || !self.noop_changes.is_empty()
    }

    /// Whether the plan can be applied without restart.
    pub fn is_hot_reloadable(&self) -> bool {
        !self.restart_required
    }

    /// Log a human-readable summary of the plan.
    pub fn log_summary(&self) {
        if !self.has_changes() {
            info!("config reload: no changes detected");
            return;
        }
        if self.restart_required {
            warn!(
                "config reload: restart required — {}",
                self.restart_reasons.join("; ")
            );
        }
        for action in &self.hot_actions {
            info!("config reload: hot-reload action queued — {action:?}");
        }
        for noop in &self.noop_changes {
            info!("config reload: no-op change — {noop}");
        }
    }
}

// ---------------------------------------------------------------------------
// build_reload_plan
// ---------------------------------------------------------------------------

/// Compare JSON-serialized forms of a field. Returns `true` when the
/// serialized representations differ (or if one side fails to serialize).
fn field_changed<T: serde::Serialize>(old: &T, new: &T) -> bool {
    let old_json = serde_json::to_string(old).ok();
    let new_json = serde_json::to_string(new).ok();
    old_json != new_json
}

/// Runtime capabilities the planner needs to know about so it can correctly
/// classify changes whose hot-reload feasibility depends on which optional
/// hooks the embedding binary wired up at boot.
///
/// Today the only such hook is the log-level reloader (only the CLI daemon
/// path installs it; embedded callers like the desktop server start the same
/// kernel without it). Without consulting this struct, `build_reload_plan`
/// would always mark `log_level` changes as hot-reloadable — and in
/// embedded contexts the kernel would just warn-and-no-op while the
/// dashboard reported success. See Codex P2-2 #3200.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReloadCapabilities {
    /// `true` if a [`crate::log_reload::LogLevelReloader`] has been installed
    /// on the kernel via `set_log_reloader`. When `false`, `log_level`
    /// changes are routed to `restart_required` instead of `hot_actions`.
    pub log_reloader_installed: bool,
}

/// Diff two configurations and produce a reload plan.
///
/// Backward-compatibility wrapper that assumes every optional reloader is
/// installed — i.e. matches the original CLI daemon path. New call sites
/// (especially anything embedded that lacks the log reloader) should prefer
/// [`build_reload_plan_with_caps`].
///
/// The plan categorizes every detected change into one of three buckets:
///
/// 1. **restart_required** — the change touches something that cannot be
///    patched at runtime (e.g. the listen address or database path).
/// 2. **hot_actions** — the change can be applied without restarting.
/// 3. **noop_changes** — the change is informational; no action needed.
pub fn build_reload_plan(old: &KernelConfig, new: &KernelConfig) -> ReloadPlan {
    build_reload_plan_with_caps(
        old,
        new,
        ReloadCapabilities {
            log_reloader_installed: true,
        },
    )
}

/// Diff two configurations against a known set of [`ReloadCapabilities`].
///
/// Use this from the kernel hot-reload path so changes whose feasibility
/// depends on optional hooks (currently `log_level`) get demoted to
/// `restart_required` when the hook isn't wired — preventing the
/// dashboard from being told "applied" while the live filter never moved.
pub fn build_reload_plan_with_caps(
    old: &KernelConfig,
    new: &KernelConfig,
    caps: ReloadCapabilities,
) -> ReloadPlan {
    let mut plan = ReloadPlan {
        restart_required: false,
        restart_reasons: Vec::new(),
        hot_actions: Vec::new(),
        noop_changes: Vec::new(),
    };

    // ----- Restart-required fields -----

    if old.api_listen != new.api_listen {
        plan.restart_required = true;
        plan.restart_reasons.push(format!(
            "api_listen changed: {} -> {}",
            old.api_listen, new.api_listen
        ));
    }

    if old.api_key != new.api_key {
        plan.noop_changes
            .push("api_key changed (effective immediately via config swap)".to_string());
    }

    if old.dashboard_user != new.dashboard_user
        || old.dashboard_pass != new.dashboard_pass
        || old.dashboard_pass_hash != new.dashboard_pass_hash
    {
        plan.hot_actions.push(HotAction::UpdateDashboardCredentials);
    }

    if old.network_enabled != new.network_enabled {
        plan.restart_required = true;
        plan.restart_reasons
            .push("network_enabled changed".to_string());
    }

    // Network config (shared_secret, listen_addresses, etc.)
    if field_changed(&old.network, &new.network) {
        plan.restart_required = true;
        plan.restart_reasons
            .push("network config changed".to_string());
    }

    // Memory config (requires restarting SQLite connections)
    if field_changed(&old.memory, &new.memory) {
        plan.restart_required = true;
        plan.restart_reasons
            .push("memory config changed".to_string());
    }

    // Proxy config — hot-reloadable: re-export env vars and flush driver cache.
    if field_changed(&old.proxy, &new.proxy) {
        plan.hot_actions.push(HotAction::ReloadProxy);
    }

    // Default model — hot-reloadable (just swap config fields, new agents pick it up)
    if field_changed(&old.default_model, &new.default_model) {
        plan.hot_actions.push(HotAction::UpdateDefaultModel);
    }

    // Home/data directory changes
    if old.home_dir != new.home_dir {
        plan.restart_required = true;
        plan.restart_reasons.push(format!(
            "home_dir changed: {:?} -> {:?}",
            old.home_dir, new.home_dir
        ));
    }
    if old.data_dir != new.data_dir {
        plan.restart_required = true;
        plan.restart_reasons.push(format!(
            "data_dir changed: {:?} -> {:?}",
            old.data_dir, new.data_dir
        ));
    }

    // Stable prefix mode — hot-reloaded via ArcSwap config, effective on next message.
    if old.stable_prefix_mode != new.stable_prefix_mode {
        plan.noop_changes.push(format!(
            "stable_prefix_mode: {} -> {} (effective on next message)",
            old.stable_prefix_mode, new.stable_prefix_mode
        ));
    }

    // Vault config (encryption key derivation)
    if field_changed(&old.vault, &new.vault) {
        plan.restart_required = true;
        plan.restart_reasons
            .push("vault config changed".to_string());
    }

    // ----- Hot-reloadable fields -----

    if field_changed(&old.channels, &new.channels) {
        plan.hot_actions.push(HotAction::ReloadChannels);
    }

    if field_changed(&old.skills, &new.skills) {
        plan.hot_actions.push(HotAction::ReloadSkills);
    }

    if old.usage_footer != new.usage_footer {
        plan.hot_actions.push(HotAction::UpdateUsageFooter);
    }

    if field_changed(&old.web, &new.web) {
        plan.hot_actions.push(HotAction::ReloadWebConfig);
    }

    if field_changed(&old.browser, &new.browser) {
        plan.hot_actions.push(HotAction::ReloadBrowserConfig);
    }

    if field_changed(&old.approval, &new.approval) {
        plan.hot_actions.push(HotAction::UpdateApprovalPolicy);
    }

    if old.max_cron_jobs != new.max_cron_jobs {
        plan.hot_actions.push(HotAction::UpdateCronConfig);
    }

    if field_changed(&old.webhook_triggers, &new.webhook_triggers) {
        plan.hot_actions.push(HotAction::UpdateWebhookConfig);
    }

    if field_changed(&old.extensions, &new.extensions) {
        plan.hot_actions.push(HotAction::ReloadExtensions);
    }

    if field_changed(&old.mcp_servers, &new.mcp_servers) {
        plan.hot_actions.push(HotAction::ReloadMcpServers);
    }

    // Top-level [[taint_rules]] registry — hot-reloadable via the shared
    // `taint_rules_swap`. Tracked separately from `mcp_servers` because
    // operators commonly tune rule sets without touching MCP server config.
    if old.taint_rules != new.taint_rules {
        plan.hot_actions.push(HotAction::ReloadTaintRules);
    }

    if field_changed(&old.a2a, &new.a2a) {
        plan.hot_actions.push(HotAction::ReloadA2aConfig);
    }

    if field_changed(&old.fallback_providers, &new.fallback_providers) {
        plan.hot_actions.push(HotAction::ReloadFallbackProviders);
    }

    if field_changed(&old.provider_urls, &new.provider_urls)
        || field_changed(&old.provider_regions, &new.provider_regions)
    {
        plan.hot_actions.push(HotAction::ReloadProviderUrls);
    }

    if field_changed(&old.tool_policy, &new.tool_policy) {
        plan.hot_actions.push(HotAction::UpdateToolPolicy);
    }

    // RBAC M3 (#3054): invalidate the AuthManager when any field that
    // feeds it changes — `[[users]]` (role / channel_bindings /
    // tool_policy / channel_tool_rules / tool_categories /
    // memory_access) or the tool group catalogue used for category
    // resolution. Without this, a `/api/config/reload` after a policy
    // edit is a silent no-op.
    if field_changed(&old.users, &new.users)
        || field_changed(&old.tool_policy.groups, &new.tool_policy.groups)
    {
        plan.hot_actions.push(HotAction::ReloadAuth);
    }

    if field_changed(&old.proactive_memory, &new.proactive_memory) {
        plan.hot_actions.push(HotAction::UpdateProactiveMemory);
    }

    if field_changed(&old.sanitize, &new.sanitize) {
        plan.noop_changes.push(
            "sanitize config changed (effective on next message via config swap)".to_string(),
        );
    }

    if field_changed(&old.provider_api_keys, &new.provider_api_keys) {
        plan.hot_actions.push(HotAction::ReloadProviderApiKeys);
    }

    // ----- No-op fields -----

    if old.log_level != new.log_level {
        if caps.log_reloader_installed {
            plan.hot_actions
                .push(HotAction::ReloadLogLevel(new.log_level.clone()));
        } else {
            // No reloader wired (embedded callers without the CLI's
            // log_filter slot). Demote to restart_required so the
            // dashboard reports an honest "needs restart" instead of
            // a false "applied" — see Codex P2-2 #3200.
            plan.restart_required = true;
            plan.restart_reasons.push(format!(
                "log_level: {} -> {} (no log reloader installed; restart required)",
                old.log_level, new.log_level
            ));
        }
    }

    if old.language != new.language {
        plan.noop_changes.push(format!(
            "language: {} -> {} (effective on next message)",
            old.language, new.language
        ));
    }

    if old.mode != new.mode {
        plan.noop_changes.push(format!(
            "mode: {:?} -> {:?} (effective on next message)",
            old.mode, new.mode
        ));
    }

    plan
}

// ---------------------------------------------------------------------------
// validate_config_for_reload
// ---------------------------------------------------------------------------

/// Validate a new config before applying it.
///
/// Returns `Ok(())` if the config passes basic sanity checks, or `Err` with
/// a list of human-readable error messages.
pub fn validate_config_for_reload(config: &KernelConfig) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if config.api_listen.is_empty() {
        errors.push("api_listen cannot be empty".to_string());
    }

    if config.max_cron_jobs > 10_000 {
        errors.push("max_cron_jobs exceeds reasonable limit (10000)".to_string());
    }

    // Validate approval policy
    if let Err(e) = config.approval.validate() {
        errors.push(format!("approval policy: {e}"));
    }

    // Network config: if network is enabled, shared_secret must be set
    if config.network_enabled && config.network.shared_secret.is_empty() {
        errors.push("network_enabled is true but network.shared_secret is empty".to_string());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// should_reload — convenience helper for the reload mode
// ---------------------------------------------------------------------------

/// Given the configured [`ReloadMode`] and a [`ReloadPlan`], decide whether
/// the caller should apply hot actions.
///
/// Returns `true` if hot-reload actions should be applied.
pub fn should_apply_hot(mode: ReloadMode, plan: &ReloadPlan) -> bool {
    match mode {
        ReloadMode::Off => false,
        ReloadMode::Restart => false, // caller must do a full restart
        ReloadMode::Hot => !plan.hot_actions.is_empty(),
        ReloadMode::Hybrid => !plan.hot_actions.is_empty(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::config::KernelConfig;

    /// Helper: create a default config for diffing.
    fn default_cfg() -> KernelConfig {
        KernelConfig::default()
    }

    // -----------------------------------------------------------------------
    // Plan detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_changes_detected() {
        let a = default_cfg();
        let b = default_cfg();
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.has_changes());
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.is_empty());
        assert!(plan.noop_changes.is_empty());
    }

    #[test]
    fn test_api_listen_requires_restart() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.api_listen = "0.0.0.0:8080".to_string();
        let plan = build_reload_plan(&a, &b);
        assert!(plan.restart_required);
        assert!(plan
            .restart_reasons
            .iter()
            .any(|r| r.contains("api_listen")));
    }

    #[test]
    fn test_api_key_hot_reloaded() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.api_key = "super-secret-key".to_string();
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "api_key should be hot-reloaded via config swap"
        );
        assert!(plan.noop_changes.iter().any(|r| r.contains("api_key")));
    }

    #[test]
    fn test_network_requires_restart() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.network_enabled = true;
        let plan = build_reload_plan(&a, &b);
        assert!(plan.restart_required);
        assert!(plan
            .restart_reasons
            .iter()
            .any(|r| r.contains("network_enabled")));
    }

    #[test]
    fn test_network_config_requires_restart() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.network.shared_secret = "new-secret".to_string();
        let plan = build_reload_plan(&a, &b);
        assert!(plan.restart_required);
        assert!(plan
            .restart_reasons
            .iter()
            .any(|r| r.contains("network config")));
    }

    #[test]
    fn test_memory_config_requires_restart() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.memory.consolidation_threshold = 99_999;
        let plan = build_reload_plan(&a, &b);
        assert!(plan.restart_required);
        assert!(plan
            .restart_reasons
            .iter()
            .any(|r| r.contains("memory config")));
    }

    #[test]
    fn test_default_model_hot_reloadable() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.default_model.model = "gpt-4".to_string();
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "default_model should be hot-reloadable"
        );
        assert!(plan.hot_actions.contains(&HotAction::UpdateDefaultModel));
    }

    #[test]
    fn test_stable_prefix_mode_hot_reloaded() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.stable_prefix_mode = true;
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "stable_prefix_mode should be hot-reloaded via config swap"
        );
        assert!(plan
            .noop_changes
            .iter()
            .any(|r| r.contains("stable_prefix_mode")));
    }

    #[test]
    fn test_proxy_config_hot_reloaded() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.proxy.http_proxy = Some("http://proxy.example.com:8080".to_string());
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required, "proxy should be hot-reloaded");
        assert!(plan.hot_actions.contains(&HotAction::ReloadProxy));
    }

    // -----------------------------------------------------------------------
    // Hot-reload tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_channels_hot_reload() {
        let a = default_cfg();
        let mut b = default_cfg();
        // Change the channels config by adding a Telegram config
        b.channels.telegram =
            librefang_types::config::OneOrMany(vec![librefang_types::config::TelegramConfig {
                bot_token_env: "TG_TOKEN".to_string(),
                ..Default::default()
            }]);
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::ReloadChannels));
    }

    #[test]
    fn test_usage_footer_hot_reload() {
        use librefang_types::config::UsageFooterMode;
        let a = default_cfg();
        let mut b = default_cfg();
        b.usage_footer = UsageFooterMode::Off;
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::UpdateUsageFooter));
    }

    #[test]
    fn test_max_cron_jobs_hot_reload() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.max_cron_jobs = 1000;
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::UpdateCronConfig));
    }

    #[test]
    fn test_extensions_hot_reload() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.extensions.reconnect_max_attempts = 20;
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::ReloadExtensions));
    }

    #[test]
    fn test_skills_hot_reload_load_user_toggle() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.skills.load_user = false;
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(
            plan.hot_actions.contains(&HotAction::ReloadSkills),
            "disabling load_user should trigger ReloadSkills"
        );
    }

    #[test]
    fn test_skills_hot_reload_extra_dirs() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.skills
            .extra_dirs
            .push(std::path::PathBuf::from("/tmp/my-skills"));
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(
            plan.hot_actions.contains(&HotAction::ReloadSkills),
            "adding extra_dirs should trigger ReloadSkills"
        );
    }

    #[test]
    fn test_skills_no_reload_when_unchanged() {
        let a = default_cfg();
        let b = default_cfg();
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.hot_actions.contains(&HotAction::ReloadSkills),
            "identical skills config must not push ReloadSkills"
        );
    }

    #[test]
    fn test_provider_urls_hot_reload() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.provider_urls
            .insert("ollama".to_string(), "http://10.0.0.5:11434/v1".to_string());
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::ReloadProviderUrls));
    }

    #[test]
    fn test_tool_policy_hot_reload() {
        use librefang_types::tool_policy::{PolicyEffect, ToolPolicyRule};
        let a = default_cfg();
        let mut b = default_cfg();
        b.tool_policy.global_rules.push(ToolPolicyRule {
            pattern: "shell_*".to_string(),
            effect: PolicyEffect::Deny,
        });
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::UpdateToolPolicy));
    }

    // -----------------------------------------------------------------------
    // Mixed changes
    // -----------------------------------------------------------------------

    #[test]
    fn test_mixed_changes() {
        use librefang_types::config::UsageFooterMode;
        let a = default_cfg();
        let mut b = default_cfg();
        // Restart-required
        b.api_listen = "0.0.0.0:9999".to_string();
        // Hot-reloadable
        b.usage_footer = UsageFooterMode::Tokens;
        b.max_cron_jobs = 100;
        b.log_level = "debug".to_string();

        let plan = build_reload_plan(&a, &b);
        assert!(plan.restart_required);
        assert!(plan.has_changes());
        // Hot actions are still collected even if restart is required,
        // so the caller knows what will need re-initialization after restart.
        assert!(plan.hot_actions.contains(&HotAction::UpdateUsageFooter));
        assert!(plan.hot_actions.contains(&HotAction::UpdateCronConfig));
        assert!(plan
            .hot_actions
            .contains(&HotAction::ReloadLogLevel("debug".to_string())));
    }

    // -----------------------------------------------------------------------
    // No-op changes
    // -----------------------------------------------------------------------

    #[test]
    fn test_noop_changes() {
        use librefang_types::config::KernelMode;
        let a = default_cfg();
        let mut b = default_cfg();
        b.language = "de".to_string();
        b.mode = KernelMode::Dev;

        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.is_empty());
        assert_eq!(plan.noop_changes.len(), 2);
        assert!(plan.noop_changes.iter().any(|c| c.contains("language")));
        assert!(plan.noop_changes.iter().any(|c| c.contains("mode")));
    }

    #[test]
    fn test_log_level_hot_reloaded() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.log_level = "debug".to_string();

        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required, "log_level should be hot-reloadable");
        assert!(plan
            .hot_actions
            .contains(&HotAction::ReloadLogLevel("debug".to_string())));
    }

    #[test]
    fn test_log_level_demoted_to_restart_when_no_reloader_installed() {
        // Codex P2-2 #3200: embedded callers (e.g. desktop server) boot
        // the same kernel without wiring a LogLevelReloader. Without
        // capability-aware planning, the dashboard would receive a
        // false "applied, no restart needed" response while the live
        // filter never moved. Assert that demoting to restart_required
        // is the active behaviour when the reloader is absent.
        let a = default_cfg();
        let mut b = default_cfg();
        b.log_level = "debug".to_string();

        let plan = build_reload_plan_with_caps(
            &a,
            &b,
            ReloadCapabilities {
                log_reloader_installed: false,
            },
        );
        assert!(
            plan.restart_required,
            "log_level change must require restart when no reloader is installed"
        );
        assert!(
            !plan
                .hot_actions
                .iter()
                .any(|a| matches!(a, HotAction::ReloadLogLevel(_))),
            "ReloadLogLevel must NOT be queued as a hot action without a reloader"
        );
        assert!(
            plan.restart_reasons.iter().any(|r| r.contains("log_level")),
            "restart_reasons should explain the log_level demotion: {:?}",
            plan.restart_reasons
        );
    }

    // -----------------------------------------------------------------------
    // has_changes / is_hot_reloadable helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_has_changes() {
        // No changes
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![],
            noop_changes: vec![],
        };
        assert!(!plan.has_changes());

        // Only noop
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![],
            noop_changes: vec!["language: en -> de".to_string()],
        };
        assert!(plan.has_changes());

        // Only hot
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![HotAction::UpdateCronConfig],
            noop_changes: vec![],
        };
        assert!(plan.has_changes());

        // Only restart
        let plan = ReloadPlan {
            restart_required: true,
            restart_reasons: vec!["api_listen changed".to_string()],
            hot_actions: vec![],
            noop_changes: vec![],
        };
        assert!(plan.has_changes());
    }

    #[test]
    fn test_is_hot_reloadable() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![HotAction::ReloadChannels],
            noop_changes: vec![],
        };
        assert!(plan.is_hot_reloadable());

        let plan = ReloadPlan {
            restart_required: true,
            restart_reasons: vec!["api_listen changed".to_string()],
            hot_actions: vec![HotAction::ReloadChannels],
            noop_changes: vec![],
        };
        assert!(!plan.is_hot_reloadable());
    }

    // -----------------------------------------------------------------------
    // Validation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_config_for_reload_valid() {
        let config = default_cfg();
        assert!(validate_config_for_reload(&config).is_ok());
    }

    #[test]
    fn test_validate_config_for_reload_invalid() {
        // Empty api_listen
        let mut config = default_cfg();
        config.api_listen = String::new();
        let err = validate_config_for_reload(&config).unwrap_err();
        assert!(err.iter().any(|e| e.contains("api_listen")));

        // Excessive max_cron_jobs
        let mut config = default_cfg();
        config.max_cron_jobs = 100_000;
        let err = validate_config_for_reload(&config).unwrap_err();
        assert!(err.iter().any(|e| e.contains("max_cron_jobs")));
    }

    #[test]
    fn test_validate_network_enabled_no_secret() {
        let mut config = default_cfg();
        config.network_enabled = true;
        config.network.shared_secret = String::new();
        let err = validate_config_for_reload(&config).unwrap_err();
        assert!(err.iter().any(|e| e.contains("shared_secret")));
    }

    // -----------------------------------------------------------------------
    // should_apply_hot
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_apply_hot_off() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![HotAction::ReloadChannels],
            noop_changes: vec![],
        };
        assert!(!should_apply_hot(ReloadMode::Off, &plan));
    }

    #[test]
    fn test_should_apply_hot_restart_mode() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![HotAction::ReloadChannels],
            noop_changes: vec![],
        };
        assert!(!should_apply_hot(ReloadMode::Restart, &plan));
    }

    #[test]
    fn test_should_apply_hot_hybrid() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![HotAction::ReloadChannels],
            noop_changes: vec![],
        };
        assert!(should_apply_hot(ReloadMode::Hybrid, &plan));
        assert!(should_apply_hot(ReloadMode::Hot, &plan));
    }

    #[test]
    fn test_should_apply_hot_empty() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![],
            noop_changes: vec![],
        };
        assert!(!should_apply_hot(ReloadMode::Hybrid, &plan));
    }
}
