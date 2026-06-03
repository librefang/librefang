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
    /// Credential pool configuration changed — rebuild pools.
    ReloadCredentialPools,
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
    /// previous policy is still being enforced. Supersedes the M6
    /// `ReloadUsers` action that only rebuilt the channel-binding index.
    ReloadAuth,
    /// `[queue.concurrency]` changed — resize the global lane semaphores
    /// so a smaller `trigger_lane` actually rate-limits new work (#3628).
    /// Per-agent caps are NOT touched — see
    /// `docs/architecture/trigger-dispatch-concurrency.md` for why.
    UpdateQueueConcurrency,
    /// `[budget]` changed — push the new global cost / token caps into the
    /// metering subsystem's `budget_config` swap so the next LLM call gates
    /// against the operator's edit (#4797). Without this action, edits to
    /// `[budget]` in `config.toml` only take effect on the next boot — the
    /// in-memory `BudgetConfig` is constructed once from `KernelConfig.budget`
    /// at boot time and never re-read.
    UpdateBudget,
    /// `[external_auth]` (or any `[[external_auth.providers]]` entry)
    /// changed in a way that affects IdP identity — flush the OIDC
    /// discovery + JWKS caches owned by `librefang-api::oauth`.
    ///
    /// Without this action, swapping IdPs at runtime leaves the
    /// previous provider's JWKS in cache for up to 1h (the cache TTL).
    /// Tokens issued by the new IdP fail JWT signature validation
    /// against the stale keys → 401 until the natural eviction.
    /// Caches are keyed by `issuer_url` / `jwks_uri`; a new IdP means
    /// a new key, so the stale entries would never be hit anyway,
    /// but they bloat memory until TTL. The fast eviction also
    /// matters when an operator rotates `issuer_url` back to a value
    /// the cache already holds with stale keys.
    ReloadExternalAuth,
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

/// Decide whether two `[external_auth]` snapshots disagree on a field
/// that affects which OIDC discovery document / JWKS keyset is
/// canonical — i.e. whether the existing API-side caches should be
/// flushed.
///
/// The full `ExternalAuthConfig` carries operator-facing knobs
/// (`session_ttl_secs`, `allowed_domains`, `require_email_verified`,
/// `redirect_url`, scopes, audience) that are read directly from the
/// live config at each request and never cached by the OIDC layer.
/// Triggering cache eviction on those edits would force a network
/// round-trip on the next login for no behavioural change, so we
/// narrow the trigger set to the fields that actually key into the
/// caches.
///
/// IdP-identity fields:
///   - top-level `enabled` (toggling auth off then on should rebuild
///     fresh — a quiesced provider may have rotated keys),
///   - top-level `issuer_url` (discovery cache key in single-provider
///     mode),
///   - per-provider `id` set (a renamed provider effectively rebinds
///     a different IdP under the same handle),
///   - per-provider `issuer_url` and `jwks_uri` (cache keys in
///     multi-provider mode).
fn external_auth_idp_changed(
    old: &librefang_types::config::ExternalAuthConfig,
    new: &librefang_types::config::ExternalAuthConfig,
) -> bool {
    if old.enabled != new.enabled || old.issuer_url != new.issuer_url {
        return true;
    }
    // Multi-provider: compare the (id, issuer_url, jwks_uri) tuples.
    // Length difference alone is conclusive; otherwise zip-compare so
    // a reordering of the providers list — which can legitimately
    // change route precedence — does not trigger eviction unless an
    // IdP-identity field also moved.
    if old.providers.len() != new.providers.len() {
        return true;
    }
    // Build a `(id -> (issuer_url, jwks_uri))` map for each side and
    // diff; order-insensitive so a pure reordering doesn't churn the
    // cache. A renamed `id` shows up as a removed entry + an added
    // entry under the new name → diff returns `true`, which is the
    // correct behaviour (the route handle now points at a different
    // logical IdP slot).
    let to_map = |cfg: &librefang_types::config::ExternalAuthConfig| {
        cfg.providers
            .iter()
            .map(|p| (p.id.clone(), (p.issuer_url.clone(), p.jwks_uri.clone())))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    to_map(old) != to_map(new)
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

    // Memory wiki config (#3329) — the vault is constructed once at
    // boot and held in `LibreFangKernel.wiki_vault`; toggling
    // `enabled`, switching `mode` / `render_mode`, or pointing
    // `vault_path` somewhere else cannot be picked up without a
    // rebuild. Mark restart-required so an operator gets a loud signal
    // instead of a silent no-op.
    if field_changed(&old.memory_wiki, &new.memory_wiki) {
        plan.restart_required = true;
        plan.restart_reasons
            .push("memory_wiki config changed".to_string());
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

    if field_changed(&old.sidecar_channels, &new.sidecar_channels) {
        // Reuses the same hot action — `mesh.channel_adapters.clear()`
        // forces channel_bridge to re-init from `kernel.config_ref()`,
        // which already iterates `sidecar_channels` on every init pass.
        if !plan.hot_actions.contains(&HotAction::ReloadChannels) {
            plan.hot_actions.push(HotAction::ReloadChannels);
        }
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

    if field_changed(&old.credential_pools, &new.credential_pools) {
        plan.hot_actions.push(HotAction::ReloadCredentialPools);
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

    // #3628 — resize the lane semaphores when `[queue.concurrency]`
    // changes. Without this the new caps are written into `self.config`
    // but the live semaphores were sized at boot and never updated.
    if field_changed(&old.queue.concurrency, &new.queue.concurrency) {
        plan.hot_actions.push(HotAction::UpdateQueueConcurrency);
    }

    // #4797 — `[budget]` is held in `MeteringSubsystem.budget_config` (an
    // ArcSwap snapshot built at boot from `KernelConfig.budget`), separate
    // from `self.config`. A bare config swap leaves the metering snapshot
    // pointed at the boot-time budget, so edits to `max_hourly_usd` etc.
    // silently no-op until restart. Push `UpdateBudget` to RCU the snapshot.
    if field_changed(&old.budget, &new.budget) {
        plan.hot_actions.push(HotAction::UpdateBudget);
    }

    // `[external_auth]` IdP identity changed — flush the OIDC discovery
    // + JWKS caches that `librefang-api::oauth` keeps as module-level
    // `LazyLock`s, keyed by `issuer_url` / `jwks_uri`. Without this, a
    // hot-reload that swaps in a different identity provider would
    // leave the previous IdP's JWKS in cache for up to 1h, and any
    // re-binding of an issuer URL to a new keyset (key rotation +
    // reconfigure in one step) would 401 every token until the
    // natural cache TTL expires.
    //
    // Only fields that actually affect the cached entries should
    // trigger eviction: changing `session_ttl_secs` or `allowed_domains`
    // doesn't invalidate any cached key, and firing the action on
    // every edit would waste a network round-trip on the next login.
    // See the helper for the exact field set.
    if external_auth_idp_changed(&old.external_auth, &new.external_auth) {
        plan.hot_actions.push(HotAction::ReloadExternalAuth);
    } else if field_changed(&old.external_auth, &new.external_auth) {
        // Non-IdP edits only (session_ttl_secs, allowed_domains, redirect_url,
        // scopes, audience, require_email_verified). The OAuth layer reads
        // these live from the ArcSwap config on every request (`oauth.rs`:
        // `config_ref()` / `config_snapshot()`), so the bare config swap makes
        // them effective on the next request — no restart, and no cache
        // eviction (none of these key into the OIDC discovery / JWKS caches).
        plan.noop_changes.push(
            "external_auth config changed (effective on next request via config swap)".to_string(),
        );
    }

    if field_changed(&old.sanitize, &new.sanitize) {
        plan.noop_changes.push(
            "sanitize config changed (effective on next message via config swap)".to_string(),
        );
    }

    // (M6 had a separate `ReloadUsers` action here; collapsed into M3's
    // `ReloadAuth` above since `auth.reload(&users, &tool_groups)` does
    // the strict superset of what `replace_users` did.)

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

    // ----- Backfilled field coverage (#config-reload-coverage) -----
    //
    // Every `KernelConfig` field reaches one of the three branches below or
    // one of the hand-tuned branches above. The
    // `every_config_field_is_reload_classified` test enumerates the struct
    // via `KernelConfig::known_top_level_fields()` and fails if a field is
    // missing from BOTH this function's coverage and
    // [`classified_reload_fields`]. Keep the two in sync.
    //
    // Classification rules used here (see the doc that drove this backfill):
    //   * RESTART  — the value is captured once at boot / server
    //                construction (into a kernel field, the axum router, a
    //                background task, or a cached LLM driver) and there is no
    //                hot action wired to rebuild that consumer. A bare config
    //                swap would silently no-op, so we demand a restart.
    //   * NOOP     — the value is read live from `config_ref()` /
    //                `self.config.load()` on every message / request, so the
    //                ArcSwap config swap performed by `reload_config` makes
    //                the change effective on the next use with no extra work.
    // When the live-read path could not be verified, the field is classed
    // RESTART (the safe default) rather than guessed into NOOP/HotReload.

    // Helper: record a restart-required change for `field` when it differs.
    // Scoped in its own block so the mutable borrow of `plan` ends at the
    // closing brace — the `noop` closure below can then re-borrow `plan`
    // without a `drop()` (clippy flags `drop()` on a non-`Drop` closure).
    {
        let mut restart_if_changed = |changed: bool, field: &str| {
            if changed {
                plan.restart_required = true;
                plan.restart_reasons
                    .push(format!("{field} changed (restart required)"));
            }
        };

        // -- RESTART: boot- / server-captured, no hot action wired --
        restart_if_changed(old.config_version != new.config_version, "config_version");
        restart_if_changed(old.cors_origin != new.cors_origin, "cors_origin");
        restart_if_changed(old.trusted_hosts != new.trusted_hosts, "trusted_hosts");
        restart_if_changed(
            old.trusted_proxies != new.trusted_proxies,
            "trusted_proxies",
        );
        restart_if_changed(
            old.trust_forwarded_for != new.trust_forwarded_for,
            "trust_forwarded_for",
        );
        restart_if_changed(
            old.allowed_mount_roots != new.allowed_mount_roots,
            "allowed_mount_roots",
        );
        restart_if_changed(
            old.require_auth_for_reads != new.require_auth_for_reads,
            "require_auth_for_reads",
        );
        restart_if_changed(
            old.external_auth_proxy != new.external_auth_proxy,
            "external_auth_proxy",
        );
        restart_if_changed(
            field_changed(&old.channel_role_mapping, &new.channel_role_mapping),
            "channel_role_mapping",
        );
        restart_if_changed(old.include != new.include, "include");
        restart_if_changed(
            field_changed(&old.exec_policy, &new.exec_policy),
            "exec_policy",
        );
        restart_if_changed(field_changed(&old.bindings, &new.bindings), "bindings");
        restart_if_changed(field_changed(&old.tool_exec, &new.tool_exec), "tool_exec");
        restart_if_changed(
            field_changed(&old.auth_profiles, &new.auth_profiles),
            "auth_profiles",
        );
        restart_if_changed(field_changed(&old.vertex_ai, &new.vertex_ai), "vertex_ai");
        restart_if_changed(
            field_changed(&old.azure_openai, &new.azure_openai),
            "azure_openai",
        );
        restart_if_changed(field_changed(&old.oauth, &new.oauth), "oauth");
        restart_if_changed(
            field_changed(
                &old.provider_request_timeout_secs,
                &new.provider_request_timeout_secs,
            ),
            "provider_request_timeout_secs",
        );
        restart_if_changed(
            field_changed(&old.provider_max_retries, &new.provider_max_retries),
            "provider_max_retries",
        );
        restart_if_changed(
            field_changed(&old.provider_proxy_urls, &new.provider_proxy_urls),
            "provider_proxy_urls",
        );
        restart_if_changed(
            old.local_probe_interval_secs != new.local_probe_interval_secs,
            "local_probe_interval_secs",
        );
        restart_if_changed(
            field_changed(&old.health_check, &new.health_check),
            "health_check",
        );
        restart_if_changed(field_changed(&old.heartbeat, &new.heartbeat), "heartbeat");
        restart_if_changed(field_changed(&old.plugins, &new.plugins), "plugins");
        restart_if_changed(field_changed(&old.registry, &new.registry), "registry");
        restart_if_changed(
            field_changed(&old.rate_limit, &new.rate_limit),
            "rate_limit",
        );
        restart_if_changed(old.strict_config != new.strict_config, "strict_config");
        restart_if_changed(
            field_changed(&old.parallel_tools, &new.parallel_tools),
            "parallel_tools",
        );
        restart_if_changed(
            old.workflow_stale_timeout_minutes != new.workflow_stale_timeout_minutes,
            "workflow_stale_timeout_minutes",
        );
        restart_if_changed(
            old.workflow_default_total_timeout_secs != new.workflow_default_total_timeout_secs,
            "workflow_default_total_timeout_secs",
        );
        restart_if_changed(
            field_changed(&old.background, &new.background),
            "background",
        );
        restart_if_changed(old.log_dir != new.log_dir, "log_dir");
        restart_if_changed(old.workspaces_dir != new.workspaces_dir, "workspaces_dir");
        restart_if_changed(field_changed(&old.llm, &new.llm), "llm");
        restart_if_changed(field_changed(&old.reload, &new.reload), "reload");
        restart_if_changed(
            old.max_request_body_bytes != new.max_request_body_bytes,
            "max_request_body_bytes",
        );
        restart_if_changed(
            old.max_upload_size_bytes != new.max_upload_size_bytes,
            "max_upload_size_bytes",
        );
        restart_if_changed(
            old.max_concurrent_bg_llm != new.max_concurrent_bg_llm,
            "max_concurrent_bg_llm",
        );
        // NOTE: `external_auth` is intentionally NOT classified here. It is
        // fully hot-reloadable, so its classification lives in the hand-tuned
        // branch above: an IdP-identity change (enabled / issuer_url /
        // per-provider issuer+jwks) queues `ReloadExternalAuth` (#5594) to
        // evict the JWKS/discovery caches, and every other field is read live
        // from the ArcSwap config by the OAuth layer (`oauth.rs`:
        // `config_ref()` / `config_snapshot()`), so a non-IdP edit is a no-op
        // that takes effect on the next request — never a restart. #5652 gated
        // the backfill's restart on `!external_auth_idp_changed(...)` to clear
        // the double-classification; this drops the restart for non-IdP edits
        // entirely since the live-read path makes it unnecessary.
        restart_if_changed(
            field_changed(&old.auto_dream, &new.auto_dream),
            "auto_dream",
        );
        // The RL-export hook reads `config.rl_export` live per turn, but the
        // target (W&B / Tinker / Atropos credentials and base URLs) is an
        // egress-security surface; classify conservatively as restart so an
        // operator's intent to redirect / disable the export takes effect
        // through the documented restart path rather than mid-session.
        restart_if_changed(field_changed(&old.rl_export, &new.rl_export), "rl_export");
        restart_if_changed(field_changed(&old.audit, &new.audit), "audit");
        restart_if_changed(field_changed(&old.telemetry, &new.telemetry), "telemetry");
        restart_if_changed(
            field_changed(&old.context_engine, &new.context_engine),
            "context_engine",
        );
        restart_if_changed(field_changed(&old.session, &new.session), "session");
        restart_if_changed(
            field_changed(&old.task_board, &new.task_board),
            "task_board",
        );
        restart_if_changed(field_changed(&old.broadcast, &new.broadcast), "broadcast");
        restart_if_changed(
            field_changed(&old.auto_reply, &new.auto_reply),
            "auto_reply",
        );
        restart_if_changed(field_changed(&old.canvas, &new.canvas), "canvas");
        restart_if_changed(old.update_channel != new.update_channel, "update_channel");
        restart_if_changed(field_changed(&old.inbox, &new.inbox), "inbox");
        restart_if_changed(
            field_changed(&old.prompt_intelligence, &new.prompt_intelligence),
            "prompt_intelligence",
        );
        restart_if_changed(field_changed(&old.docker, &new.docker), "docker");
        restart_if_changed(
            field_changed(&old.trusted_manifest_signers, &new.trusted_manifest_signers),
            "trusted_manifest_signers",
        );
        // `terminal` is read live per-request for `max_windows`, but the tmux
        // wiring (`tmux_enabled` / `tmux_binary_path`) is captured once at
        // server construction (server.rs). Conservative: restart-required.
        restart_if_changed(field_changed(&old.terminal, &new.terminal), "terminal");
    }

    // -- NOOP: read live from `config_ref()` / `self.config.load()` per
    //    message or per request; the ArcSwap config swap makes the change
    //    effective on the next use with no explicit reapply action. --
    {
        let mut noop_if_changed = |changed: bool, field: &str| {
            if changed {
                plan.noop_changes.push(format!(
                    "{field} changed (effective on next message/request)"
                ));
            }
        };

        noop_if_changed(
            old.agent_max_iterations != new.agent_max_iterations,
            "agent_max_iterations",
        );
        noop_if_changed(
            old.max_history_messages != new.max_history_messages,
            "max_history_messages",
        );
        noop_if_changed(
            old.max_agent_call_depth != new.max_agent_call_depth,
            "max_agent_call_depth",
        );
        noop_if_changed(
            old.tool_timeout_secs != new.tool_timeout_secs,
            "tool_timeout_secs",
        );
        noop_if_changed(
            field_changed(&old.tool_timeouts, &new.tool_timeouts),
            "tool_timeouts",
        );
        noop_if_changed(field_changed(&old.thinking, &new.thinking), "thinking");
        noop_if_changed(field_changed(&old.triggers, &new.triggers), "triggers");
        noop_if_changed(
            field_changed(&old.notification, &new.notification),
            "notification",
        );
        noop_if_changed(field_changed(&old.tts, &new.tts), "tts");
        noop_if_changed(field_changed(&old.media, &new.media), "media");
        // The hands marketplace install handler reads `hands.registry_allowed_hosts`
        // live from `config_snapshot()` on every request, so a swap is effective
        // on the next install with no explicit reapply action.
        noop_if_changed(field_changed(&old.hands, &new.hands), "hands");
        noop_if_changed(field_changed(&old.links, &new.links), "links");
        noop_if_changed(field_changed(&old.privacy, &new.privacy), "privacy");
        noop_if_changed(field_changed(&old.pairing, &new.pairing), "pairing");
        noop_if_changed(
            field_changed(&old.gateway_compression, &new.gateway_compression),
            "gateway_compression",
        );
        noop_if_changed(
            field_changed(&old.tool_results, &new.tool_results),
            "tool_results",
        );
        noop_if_changed(
            field_changed(&old.tool_invoke, &new.tool_invoke),
            "tool_invoke",
        );
        noop_if_changed(
            field_changed(&old.default_routing, &new.default_routing),
            "default_routing",
        );
        noop_if_changed(old.prompt_caching != new.prompt_caching, "prompt_caching");
        noop_if_changed(
            field_changed(&old.prompt_cache, &new.prompt_cache),
            "prompt_cache",
        );
        noop_if_changed(
            field_changed(&old.compaction, &new.compaction),
            "compaction",
        );
        noop_if_changed(old.qwen_code_path != new.qwen_code_path, "qwen_code_path");
        noop_if_changed(
            old.cron_session_max_tokens != new.cron_session_max_tokens,
            "cron_session_max_tokens",
        );
        noop_if_changed(
            old.cron_session_max_messages != new.cron_session_max_messages,
            "cron_session_max_messages",
        );
        noop_if_changed(
            old.cron_session_warn_fraction != new.cron_session_warn_fraction,
            "cron_session_warn_fraction",
        );
        noop_if_changed(
            old.cron_session_warn_total_tokens != new.cron_session_warn_total_tokens,
            "cron_session_warn_total_tokens",
        );
        noop_if_changed(
            old.cron_session_compaction_mode != new.cron_session_compaction_mode,
            "cron_session_compaction_mode",
        );
        noop_if_changed(
            old.cron_session_compaction_keep_recent != new.cron_session_compaction_keep_recent,
            "cron_session_compaction_keep_recent",
        );
    }

    plan
}

// ---------------------------------------------------------------------------
// Reload-classification coverage (drift guard)
// ---------------------------------------------------------------------------

/// `#[serde(alias = …)]` names on `KernelConfig` top-level fields.
///
/// `KernelConfig::known_top_level_fields()` derives its list from the
/// schemars schema and folds in these aliases (see
/// `librefang_types::config::validation`). The aliases are NOT real struct
/// fields, so the coverage test must exclude them before comparing against
/// the set of fields `build_reload_plan` classifies. Keep in sync with the
/// `alias = "…"` attributes on the `KernelConfig` struct.
pub const KERNEL_CONFIG_FIELD_ALIASES: &[&str] = &[
    "listen_addr",     // alias for api_listen
    "approval_policy", // alias for approval
];

/// The exhaustive set of `KernelConfig` field names that
/// [`build_reload_plan`] inspects and classifies (RequiresRestart /
/// HotReload / Ignore).
///
/// This is a literal mirror of every field touched in
/// `build_reload_plan_with_caps`. The
/// `every_config_field_is_reload_classified` test asserts that this set is a
/// superset of every real `KernelConfig` field, so a newly-added field that
/// is not also wired into `build_reload_plan` fails the build instead of
/// silently no-op-ing on `POST /api/config/reload`.
///
/// **When you add a field to `KernelConfig`:** add a branch to
/// `build_reload_plan_with_caps` AND its name here. The test will remind you
/// if you forget.
pub fn classified_reload_fields() -> std::collections::BTreeSet<&'static str> {
    [
        // -- hand-tuned branches at the top of build_reload_plan --
        "api_listen",
        "api_key",
        "dashboard_user",
        "dashboard_pass",
        "dashboard_pass_hash",
        "network_enabled",
        "network",
        "memory",
        "memory_wiki",
        "proxy",
        "default_model",
        "home_dir",
        "data_dir",
        "stable_prefix_mode",
        "vault",
        "channels",
        "sidecar_channels",
        "skills",
        "usage_footer",
        "web",
        "browser",
        "approval",
        "max_cron_jobs",
        "webhook_triggers",
        "extensions",
        "mcp_servers",
        "taint_rules",
        "a2a",
        "fallback_providers",
        "credential_pools",
        "provider_urls",
        "provider_regions",
        "tool_policy",
        "users",
        "proactive_memory",
        "queue",
        "budget",
        "sanitize",
        "provider_api_keys",
        "log_level",
        "language",
        "mode",
        // hot-reloadable: ReloadExternalAuth on IdP-identity change, noop
        // (live-read) otherwise — see the hand-tuned branch above.
        "external_auth",
        // -- backfilled RESTART branches --
        "config_version",
        "cors_origin",
        "trusted_hosts",
        "trusted_proxies",
        "trust_forwarded_for",
        "allowed_mount_roots",
        "require_auth_for_reads",
        "external_auth_proxy",
        "channel_role_mapping",
        "include",
        "exec_policy",
        "bindings",
        "tool_exec",
        "auth_profiles",
        "vertex_ai",
        "azure_openai",
        "oauth",
        "provider_request_timeout_secs",
        "provider_max_retries",
        "provider_proxy_urls",
        "local_probe_interval_secs",
        "health_check",
        "heartbeat",
        "plugins",
        "registry",
        "rate_limit",
        "strict_config",
        "parallel_tools",
        "workflow_stale_timeout_minutes",
        "workflow_default_total_timeout_secs",
        "background",
        "log_dir",
        "workspaces_dir",
        "llm",
        "reload",
        "max_request_body_bytes",
        "max_upload_size_bytes",
        "max_concurrent_bg_llm",
        "auto_dream",
        "rl_export",
        "audit",
        "telemetry",
        "context_engine",
        "session",
        "task_board",
        "broadcast",
        "auto_reply",
        "canvas",
        "update_channel",
        "inbox",
        "prompt_intelligence",
        "docker",
        "trusted_manifest_signers",
        "terminal",
        // -- backfilled NOOP branches --
        "agent_max_iterations",
        "max_history_messages",
        "max_agent_call_depth",
        "tool_timeout_secs",
        "tool_timeouts",
        "thinking",
        "triggers",
        "notification",
        "tts",
        "media",
        "hands",
        "links",
        "privacy",
        "pairing",
        "gateway_compression",
        "tool_results",
        "tool_invoke",
        "default_routing",
        "prompt_caching",
        "prompt_cache",
        "compaction",
        "qwen_code_path",
        "cron_session_max_tokens",
        "cron_session_max_messages",
        "cron_session_warn_fraction",
        "cron_session_warn_total_tokens",
        "cron_session_compaction_mode",
        "cron_session_compaction_keep_recent",
    ]
    .into_iter()
    .collect()
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

/// Given the configured [`ReloadMode`] and a [`ReloadPlan`], decide whether
/// the freshly-validated config should be swapped into the live `ArcSwap`
/// (and dependent snapshots — raw `config.toml`, taint rules, aux client —
/// refreshed).
///
/// This is broader than [`should_apply_hot`]: in `Hot` / `Hybrid` modes it is
/// `true` whenever the plan carries *any* effective change, not only
/// `hot_actions`. A large class of fields is classified as `noop_changes`
/// precisely because they are read live from `config.load()` on each
/// message/request and rely on the swap itself to take effect (e.g.
/// `max_history_messages`, `agent_max_iterations`, `compaction`,
/// `prompt_caching`, default `language` / `mode`, `sanitize`). Gating the
/// swap on `hot_actions` alone made those edits silently no-op while the
/// reload response reported success.
///
/// `Off` / `Restart` modes still return `false` — the operator expects no
/// runtime change until a full restart.
pub fn should_store_config(mode: ReloadMode, plan: &ReloadPlan) -> bool {
    match mode {
        ReloadMode::Off | ReloadMode::Restart => false,
        ReloadMode::Hot | ReloadMode::Hybrid => {
            !plan.hot_actions.is_empty() || !plan.noop_changes.is_empty()
        }
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
        // Witness rotation history: dingtalk → whatsapp → webhook →
        // google_chat → here (`file_download_max_bytes`), the only
        // non-`OneOrMany` field still on `ChannelsConfig` after all
        // in-process channels migrated to sidecars. The assertion
        // is on the ReloadChannels hot action firing for ANY change
        // to the `channels` block, not on any adapter-specific
        // shape. (`sidecar_channels` is covered by the next test.)
        b.channels.file_download_max_bytes = a.channels.file_download_max_bytes + 1;
        let plan = build_reload_plan(&a, &b);
        assert!(!plan.restart_required);
        assert!(plan.hot_actions.contains(&HotAction::ReloadChannels));
    }

    /// Sidecar channels participate in the same hot-reload action as the
    /// in-process channels block. Without this the dashboard's
    /// "configure → save → telegram comes up" flow stays dark until
    /// daemon restart because `mesh.channel_adapters` is never cleared.
    /// `SidecarChannelConfig` has no `Default`, so build via JSON (mirrors
    /// the `sidecar_telegram()` helper in `channels_routes_test.rs`).
    #[test]
    fn sidecar_channels_change_triggers_reload_channels_action() {
        use librefang_types::config::SidecarChannelConfig;
        let a = default_cfg();
        let mut b = default_cfg();
        let sidecar: SidecarChannelConfig = serde_json::from_value(serde_json::json!({
            "name": "telegram",
            "command": "python3",
            "args": ["-m", "librefang.sidecar.adapters.telegram"],
            "channel_type": "telegram",
        }))
        .expect("valid SidecarChannelConfig");
        b.sidecar_channels = vec![sidecar];
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "sidecar_channels edits must be hot-reloadable"
        );
        assert!(
            plan.hot_actions.contains(&HotAction::ReloadChannels),
            "expected ReloadChannels in {:?}",
            plan.hot_actions,
        );
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

    /// Regression for #4797: changes to `[budget]` must produce an
    /// `UpdateBudget` hot action so the metering subsystem swaps in the
    /// new caps. Without this, edits to `max_hourly_usd` etc. in
    /// `config.toml` are written into `self.config` but the live
    /// `MeteringSubsystem.budget_config` ArcSwap stays pointed at the
    /// boot-time budget — the operator's edit silently no-ops until
    /// daemon restart.
    #[test]
    fn test_budget_hot_reload_emits_update_action() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.budget.max_hourly_usd = 12.34;
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "budget edits must be hot-reloadable"
        );
        assert!(
            plan.hot_actions.contains(&HotAction::UpdateBudget),
            "expected UpdateBudget in {:?}",
            plan.hot_actions,
        );
    }

    /// Regression for #3628: changes to `[queue.concurrency]` must produce
    /// an `UpdateQueueConcurrency` hot action so the lane semaphores get
    /// resized. Without this the new caps were stored on `self.config` but
    /// the live semaphores remained sized at boot.
    #[test]
    fn test_queue_concurrency_hot_reload_emits_resize_action() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.queue.concurrency.trigger_lane = a.queue.concurrency.trigger_lane.saturating_add(4);
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "queue.concurrency must be hot-reloadable"
        );
        assert!(
            plan.hot_actions
                .contains(&HotAction::UpdateQueueConcurrency),
            "expected UpdateQueueConcurrency in {:?}",
            plan.hot_actions,
        );
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
    // External auth — IdP-identity changes must evict OAuth caches (refs
    // `docs/issues/jwks-cache-no-reload-evict.md`). The positive and
    // negative tests together pin the "only on real IdP swap" contract.
    // -----------------------------------------------------------------------

    #[test]
    fn test_external_auth_issuer_url_change_evicts_oauth_caches() {
        let a = default_cfg();
        let mut b = default_cfg();
        b.external_auth.enabled = true;
        b.external_auth.issuer_url = "https://idp-b.example.com".to_string();
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.restart_required,
            "external_auth is hot-reloadable; restart should not be required"
        );
        assert!(
            plan.hot_actions.contains(&HotAction::ReloadExternalAuth),
            "issuer_url change must queue ReloadExternalAuth so stale JWKS \
             from the previous IdP is evicted before the next token \
             validation: actions={:?}",
            plan.hot_actions
        );
    }

    #[test]
    fn test_external_auth_provider_jwks_uri_change_evicts_oauth_caches() {
        use librefang_types::config::OidcProvider;
        let mut a = default_cfg();
        let mut b = default_cfg();
        a.external_auth.enabled = true;
        b.external_auth.enabled = true;
        a.external_auth.providers.push(OidcProvider {
            id: "corp".to_string(),
            display_name: "Corp SSO".to_string(),
            issuer_url: "https://idp-a.example.com".to_string(),
            auth_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_uri: "https://idp-a.example.com/.well-known/jwks.json".to_string(),
            client_id: "client".to_string(),
            client_secret_env: "LIBREFANG_OAUTH_CLIENT_SECRET".to_string(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".to_string(),
            scopes: vec!["openid".to_string()],
            allowed_domains: vec![],
            audience: String::new(),
            require_email_verified: None,
        });
        b.external_auth.providers.push(OidcProvider {
            id: "corp".to_string(),
            display_name: "Corp SSO".to_string(),
            // Same id, but rebound to a different IdP — the most
            // dangerous shape because the route handle is stable but
            // the cached keyset is now stale.
            issuer_url: "https://idp-b.example.com".to_string(),
            auth_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_uri: "https://idp-b.example.com/.well-known/jwks.json".to_string(),
            client_id: "client".to_string(),
            client_secret_env: "LIBREFANG_OAUTH_CLIENT_SECRET".to_string(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".to_string(),
            scopes: vec!["openid".to_string()],
            allowed_domains: vec![],
            audience: String::new(),
            require_email_verified: None,
        });
        let plan = build_reload_plan(&a, &b);
        assert!(
            plan.hot_actions.contains(&HotAction::ReloadExternalAuth),
            "per-provider issuer/jwks rebind must queue ReloadExternalAuth: \
             actions={:?}",
            plan.hot_actions
        );
    }

    #[test]
    fn test_external_auth_unrelated_field_does_not_evict_caches() {
        // session_ttl_secs, allowed_domains, require_email_verified,
        // scopes — none of these change which OIDC document or JWKS
        // is canonical, so they must NOT churn the cache.
        let mut a = default_cfg();
        let mut b = default_cfg();
        a.external_auth.enabled = true;
        b.external_auth.enabled = true;
        a.external_auth.issuer_url = "https://idp.example.com".to_string();
        b.external_auth.issuer_url = "https://idp.example.com".to_string();
        a.external_auth.session_ttl_secs = 3_600;
        b.external_auth.session_ttl_secs = 7_200;
        a.external_auth.allowed_domains = vec!["a.example.com".to_string()];
        b.external_auth.allowed_domains =
            vec!["a.example.com".to_string(), "b.example.com".to_string()];
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.hot_actions.contains(&HotAction::ReloadExternalAuth),
            "non-IdP-identity edits must NOT trigger cache eviction \
             (would force a needless OIDC round-trip on next login): \
             actions={:?}",
            plan.hot_actions
        );
        // These knobs are read live from the ArcSwap config by the OAuth layer
        // (`oauth.rs`), so a config swap applies them on the next request — no
        // restart required. #5652 left non-IdP edits classified as restart;
        // this pins them to a no-op instead.
        assert!(
            !plan.restart_required,
            "non-IdP external_auth edits are live-read; restart must not be \
             required: reasons={:?}",
            plan.restart_reasons
        );
        assert!(
            plan.noop_changes
                .iter()
                .any(|n| n.contains("external_auth")),
            "non-IdP external_auth edit should record a no-op change so the \
             operator gets honest 'applied on next request' feedback: \
             noop_changes={:?}",
            plan.noop_changes
        );
    }

    #[test]
    fn test_external_auth_provider_reorder_does_not_evict_caches() {
        // The providers list controls route precedence. Reordering it
        // is a legitimate operator action (e.g. promote SSO over
        // GitHub) that does not change any IdP's signing keys.
        use librefang_types::config::OidcProvider;
        let p = |id: &str, issuer: &str| OidcProvider {
            id: id.to_string(),
            display_name: id.to_string(),
            issuer_url: issuer.to_string(),
            auth_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_uri: format!("{issuer}/.well-known/jwks.json"),
            client_id: "client".to_string(),
            client_secret_env: "LIBREFANG_OAUTH_CLIENT_SECRET".to_string(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".to_string(),
            scopes: vec!["openid".to_string()],
            allowed_domains: vec![],
            audience: String::new(),
            require_email_verified: None,
        };
        let mut a = default_cfg();
        let mut b = default_cfg();
        a.external_auth.enabled = true;
        b.external_auth.enabled = true;
        a.external_auth.providers = vec![
            p("google", "https://accounts.google.com"),
            p("corp", "https://idp.example.com"),
        ];
        b.external_auth.providers = vec![
            p("corp", "https://idp.example.com"),
            p("google", "https://accounts.google.com"),
        ];
        let plan = build_reload_plan(&a, &b);
        assert!(
            !plan.hot_actions.contains(&HotAction::ReloadExternalAuth),
            "pure provider reorder must not evict caches: actions={:?}",
            plan.hot_actions
        );
    }

    #[test]
    fn test_external_auth_disable_evicts_caches() {
        // Disabling auth then re-enabling later is a legitimate
        // hot-reload sequence. We treat the disable as IdP-identity
        // change so that when the operator re-enables it, the first
        // login fetches fresh keys (the original IdP may have rotated
        // its signing keys while auth was off).
        let mut a = default_cfg();
        let mut b = default_cfg();
        a.external_auth.enabled = true;
        a.external_auth.issuer_url = "https://idp.example.com".to_string();
        b.external_auth.enabled = false;
        b.external_auth.issuer_url = "https://idp.example.com".to_string();
        let plan = build_reload_plan(&a, &b);
        assert!(
            plan.hot_actions.contains(&HotAction::ReloadExternalAuth),
            "toggling external_auth.enabled must queue cache eviction: \
             actions={:?}",
            plan.hot_actions
        );
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

    // -----------------------------------------------------------------------
    // should_store_config — the config swap must fire on noop-only changes too
    // (read-live fields like max_history_messages rely on the ArcSwap swap)
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_store_config_noop_only_swaps_in_hot_hybrid() {
        // Plan with NO hot actions, only a read-live ("noop") field change.
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![],
            noop_changes: vec!["max_history_messages".to_string()],
        };
        // `should_apply_hot` is false (no hot actions) — that was the bug:
        // gating the swap on it left read-live edits unapplied.
        assert!(!should_apply_hot(ReloadMode::Hot, &plan));
        assert!(!should_apply_hot(ReloadMode::Hybrid, &plan));
        // `should_store_config` is true — the new config is swapped in so the
        // read-live field takes effect on the next message/request.
        assert!(should_store_config(ReloadMode::Hot, &plan));
        assert!(should_store_config(ReloadMode::Hybrid, &plan));
    }

    #[test]
    fn test_should_store_config_off_restart_never_swap() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![HotAction::ReloadChannels],
            noop_changes: vec!["max_history_messages".to_string()],
        };
        // Off / Restart must not apply runtime changes even with pending diffs.
        assert!(!should_store_config(ReloadMode::Off, &plan));
        assert!(!should_store_config(ReloadMode::Restart, &plan));
    }

    #[test]
    fn test_should_store_config_empty_plan_no_swap() {
        let plan = ReloadPlan {
            restart_required: false,
            restart_reasons: vec![],
            hot_actions: vec![],
            noop_changes: vec![],
        };
        assert!(!should_store_config(ReloadMode::Hot, &plan));
        assert!(!should_store_config(ReloadMode::Hybrid, &plan));
    }

    // -----------------------------------------------------------------------
    // Reload-classification coverage (drift guard)
    // -----------------------------------------------------------------------

    /// Every real `KernelConfig` field must be classified by
    /// `build_reload_plan`. Without this, a contributor who adds a config
    /// field but forgets to wire it into the reload planner ships a silent
    /// no-op on `POST /api/config/reload` — the documented default failure
    /// mode (see `docs/issues/config-reload-coverage.md`).
    ///
    /// The struct field set is enumerated via
    /// `KernelConfig::known_top_level_fields()`, which is derived at runtime
    /// from the schemars JSON Schema and therefore sees every field
    /// regardless of `#[serde(skip_serializing_if = …)]` — the
    /// `serde_json::to_value(&default)` approach would miss the ~20 fields
    /// that serialize-skip at their default value (e.g. `trusted_hosts`,
    /// `agent_max_iterations`). Serde aliases that the schema folds in
    /// (`listen_addr`, `approval_policy`) are not real fields and are
    /// excluded.
    #[test]
    fn every_config_field_is_reload_classified() {
        let aliases: std::collections::BTreeSet<&str> =
            super::KERNEL_CONFIG_FIELD_ALIASES.iter().copied().collect();
        let fields: std::collections::BTreeSet<&str> = KernelConfig::known_top_level_fields()
            .iter()
            .copied()
            .filter(|f| !aliases.contains(f))
            .collect();

        let covered = super::classified_reload_fields();

        let missing: Vec<&str> = fields.difference(&covered).copied().collect();
        assert!(
            missing.is_empty(),
            "KernelConfig fields not classified in build_reload_plan: {missing:?}\n\
             Add a branch to `build_reload_plan_with_caps` (RequiresRestart / \
             HotReload / Ignore) AND the field name to `classified_reload_fields()`."
        );

        // Catch the inverse drift too: a name in `classified_reload_fields()`
        // that no longer exists on the struct (renamed / removed field) would
        // otherwise rot silently. The alias-folded schema list is the source
        // of truth for what's real.
        let known: std::collections::BTreeSet<&str> = KernelConfig::known_top_level_fields()
            .iter()
            .copied()
            .collect();
        let stale: Vec<&str> = covered
            .iter()
            .copied()
            .filter(|f| !known.contains(f) && !aliases.contains(f))
            .collect();
        assert!(
            stale.is_empty(),
            "`classified_reload_fields()` lists names that are not \
             KernelConfig fields (renamed/removed?): {stale:?}"
        );
    }

    /// The ops-facing reference table in `docs/operations/config-reload.md`
    /// must list exactly the same set of fields that
    /// [`super::classified_reload_fields`] classifies. The doc is
    /// hand-transcribed from `build_reload_plan`, so without this guard a
    /// classification change (or a newly-added field) could land in the code
    /// while the doc silently rots — defeating the doc's stated purpose of
    /// being the canonical "does this hot-reload?" answer.
    ///
    /// The doc lists each field as the first column of a markdown table row,
    /// `| `field_name` | ... |`. We parse those backtick-wrapped leading
    /// tokens and compare the set to `classified_reload_fields()` in both
    /// directions.
    #[test]
    fn doc_reload_table_matches_classified_reload_fields() {
        // CARGO_MANIFEST_DIR = <repo>/crates/librefang-kernel
        let doc_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/operations/config-reload.md");
        let doc = std::fs::read_to_string(&doc_path).unwrap_or_else(|e| {
            panic!("failed to read {}: {e}", doc_path.display());
        });

        // Collect the first-column backtick token of every table row.
        let mut doc_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for line in doc.lines() {
            let line = line.trim_start();
            let Some(rest) = line.strip_prefix("| `") else {
                continue;
            };
            // Token runs until the closing backtick. Field names are
            // `[a-z0-9_]+`; anything else (legend rows, prose) won't match.
            let Some(end) = rest.find('`') else { continue };
            let token = &rest[..end];
            if !token.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                doc_fields.insert(token.to_string());
            }
        }

        let covered: std::collections::BTreeSet<String> = super::classified_reload_fields()
            .iter()
            .map(|s| s.to_string())
            .collect();

        let missing_from_doc: Vec<&String> = covered.difference(&doc_fields).collect();
        assert!(
            missing_from_doc.is_empty(),
            "fields classified in build_reload_plan but absent from \
             docs/operations/config-reload.md: {missing_from_doc:?}\n\
             Add a table row for each in the doc."
        );

        let extra_in_doc: Vec<&String> = doc_fields.difference(&covered).collect();
        assert!(
            extra_in_doc.is_empty(),
            "docs/operations/config-reload.md lists field names that are not \
             classified in build_reload_plan (renamed/removed?): {extra_in_doc:?}"
        );
    }
}
