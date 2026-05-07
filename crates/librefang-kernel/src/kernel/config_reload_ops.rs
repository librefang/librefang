//! Cluster pulled out of mod.rs in #4713 phase 3e/3.
//!
//! Hosts `reload_config` and its private companion `apply_hot_actions_inner`.
//! Together they implement the hot-reload pipeline: parse + validate the
//! on-disk `config.toml`, diff it against the live snapshot, run the
//! resulting `HotAction` list under the config-reload write lock, and
//! atomically swap the new config into place. Bundling the public entry
//! point with its only caller keeps the rolling-window invariants
//! (taint-rules first, then config; cache invalidations under the same
//! guard) reviewable in one file.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Reload configuration: read the config file, diff against current, and
    /// apply hot-reloadable actions. Returns the reload plan for API response.
    pub async fn reload_config(&self) -> Result<crate::config_reload::ReloadPlan, String> {
        let old_cfg = self.config.load();
        use crate::config_reload::{should_apply_hot, validate_config_for_reload};

        // Read and parse the on-disk config via the strict loader (#4664).
        // Unlike `crate::config::load_config`, `try_load_config` returns `Err`
        // on every failure mode (TOML syntax error, broken `include = [...]`
        // chain, migration failure, deserialize-shape mismatch) instead of
        // silently falling back to `KernelConfig::default()`. Without this,
        // the diff-and-apply path below would treat the defaults as the
        // operator's intent and wipe out their `default_model`,
        // `provider_api_keys`, channels, etc.
        //
        // Surfacing `Err` here lets the watcher's
        // `Err(e) => tracing::warn!("Config hot-reload failed: {e}")` branch
        // fire and the `POST /api/config/reload` handler return 400, both
        // without touching the live config. (Phase 3e/3 of #4713 originally
        // re-introduced the pre-#4664 tolerant path during the mod.rs split;
        // this restores the strict loader.)
        let config_path = self.home_dir_boot.join("config.toml");
        let mut new_config = crate::config::try_load_config(&config_path)
            .map_err(|e| format!("Config reload failed; live config unchanged: {e}"))?;

        // Clamp bounds on the new config before validating or applying.
        // Initial boot calls clamp_bounds() at kernel construction time,
        // so without this call the reload path would apply out-of-range
        // values (e.g. max_cron_jobs=0, timeouts=0) that the initial
        // startup path normally corrects.
        new_config.clamp_bounds();

        // Validate new config. Use the same `live config unchanged` wrapper
        // as the strict-loader path so every reload-rejection error carries
        // the operator-actionable pledge — both for log readability and so
        // the integration helper
        // `assert_reload_rejects_and_preserves_default_model` (api crate) can
        // assert reload-boundary semantics with one substring regardless of
        // which inner branch tripped.
        if let Err(errors) = validate_config_for_reload(&new_config) {
            return Err(format!(
                "Config reload failed; live config unchanged: validation: {}",
                errors.join("; ")
            ));
        }

        // Build the reload plan against the live capability set so changes
        // whose feasibility depends on optional reloaders get correctly
        // routed to `restart_required` when the reloader isn't installed
        // (e.g. embedded desktop boot doesn't wire the log reloader).
        let caps = crate::config_reload::ReloadCapabilities {
            log_reloader_installed: self.log_reloader.get().is_some(),
        };
        let plan = crate::config_reload::build_reload_plan_with_caps(&old_cfg, &new_config, caps);
        plan.log_summary();

        // Apply hot actions + store new config atomically under the same
        // write lock.  This prevents message handlers from seeing side effects
        // (cleared caches, updated overrides) while config_ref() still returns
        // the old config.
        //
        // Only store the new config when hot-reload is active (Hot / Hybrid).
        // In Off / Restart modes the user expects no runtime changes — they
        // must restart to pick up the new config.
        if should_apply_hot(old_cfg.reload.mode, &plan) {
            let _write_guard = self.config_reload_lock.write().await;
            self.apply_hot_actions_inner(&plan, &new_config);
            // Push the new `[[taint_rules]]` registry into the shared swap
            // BEFORE swapping `self.config`. Connected MCP servers read from
            // this swap on every scan; updating it now means the next tool
            // call inherits the new rules without restarting the server.
            // Order: taint_rules first, then config — that way no scanner
            // sees a window where `self.config.load().taint_rules` and the
            // `taint_rules_swap` snapshot disagree.
            //
            // The reload-plan diff (`build_reload_plan`) emits
            // `HotAction::ReloadTaintRules` whenever `[[taint_rules]]`
            // changes, so `should_apply_hot` reaches this branch on those
            // edits even when no other hot action fires.
            self.taint_rules_swap
                .store(std::sync::Arc::new(new_config.taint_rules.clone()));
            // Refresh the cached raw `config.toml` snapshot (#3722) so
            // skill config injection picks up `[skills.config.*]` edits
            // without needing the per-message hot path to re-read the
            // file. The strongly-typed `KernelConfig` does not preserve
            // this open-ended namespace, so we keep the raw value
            // separately.
            let refreshed_raw = load_raw_config_toml(&config_path);
            self.raw_config_toml
                .store(std::sync::Arc::new(refreshed_raw));
            let new_config_arc = std::sync::Arc::new(new_config);
            self.config.store(std::sync::Arc::clone(&new_config_arc));
            // Rebuild the auxiliary LLM client so `[llm.auxiliary]` edits
            // take effect on the next side-task call. ArcSwap atomically
            // replaces the live snapshot — concurrent callers that already
            // resolved a chain keep using their `Arc<dyn LlmDriver>` until
            // the call completes.
            self.aux_client.store(std::sync::Arc::new(
                librefang_runtime::aux_client::AuxClient::new(
                    new_config_arc,
                    Arc::clone(&self.default_driver),
                ),
            ));
        }

        Ok(plan)
    }

    /// Apply hot-reload actions to the running kernel.
    ///
    /// **Caller must hold `config_reload_lock` write guard** so that the
    /// config swap and side effects are atomic with respect to message handlers.
    fn apply_hot_actions_inner(
        &self,
        plan: &crate::config_reload::ReloadPlan,
        new_config: &librefang_types::config::KernelConfig,
    ) {
        use crate::config_reload::HotAction;

        for action in &plan.hot_actions {
            match action {
                HotAction::UpdateApprovalPolicy => {
                    info!("Hot-reload: updating approval policy");
                    self.approval_manager
                        .update_policy(new_config.approval.clone());
                }
                HotAction::UpdateCronConfig => {
                    info!(
                        "Hot-reload: updating cron config (max_jobs={})",
                        new_config.max_cron_jobs
                    );
                    self.cron_scheduler
                        .set_max_total_jobs(new_config.max_cron_jobs);
                }
                HotAction::ReloadProviderUrls => {
                    info!("Hot-reload: applying provider URL overrides");
                    // Invalidate cached LLM drivers — URLs/keys may have changed.
                    self.driver_cache.clear();
                    // Pre-compute everything outside the RCU closure: the closure
                    // may re-run on CAS retry, so all logging + region resolution
                    // happens here exactly once. Region resolution reads a
                    // snapshot — under contention the inputs are still consistent
                    // because they only depend on `new_config` + provider list.
                    let regions = new_config.provider_regions.clone();
                    let provider_urls = new_config.provider_urls.clone();
                    let proxy_urls = new_config.provider_proxy_urls.clone();
                    let region_urls: std::collections::BTreeMap<String, String> =
                        if regions.is_empty() {
                            std::collections::BTreeMap::new()
                        } else {
                            let snapshot = self.model_catalog.load();
                            let urls = snapshot.resolve_region_urls(&regions);
                            if !urls.is_empty() {
                                info!(
                                    "Hot-reload: applied {} provider region URL override(s)",
                                    urls.len()
                                );
                            }
                            let region_api_keys = snapshot.resolve_region_api_keys(&regions);
                            if !region_api_keys.is_empty() {
                                info!(
                                    "Hot-reload: {} region api_key override(s) detected \
                                 (takes effect on next driver init)",
                                    region_api_keys.len()
                                );
                            }
                            urls
                        };
                    self.model_catalog_update(|catalog| {
                        if !region_urls.is_empty() {
                            catalog.apply_url_overrides(&region_urls);
                        }
                        // Apply explicit provider_urls (higher priority, overwrites region URLs)
                        if !provider_urls.is_empty() {
                            catalog.apply_url_overrides(&provider_urls);
                        }
                        if !proxy_urls.is_empty() {
                            catalog.apply_proxy_url_overrides(&proxy_urls);
                        }
                    });
                    // Also update media driver cache with new provider URLs
                    self.media.media_drivers.update_provider_urls(provider_urls);
                }
                HotAction::UpdateDefaultModel => {
                    info!(
                        "Hot-reload: updating default model to {}/{}",
                        new_config.default_model.provider, new_config.default_model.model
                    );
                    // Invalidate cached drivers — the default provider may have changed.
                    self.driver_cache.clear();
                    let mut guard = self
                        .default_model_override
                        .write()
                        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
                    *guard = Some(new_config.default_model.clone());
                }
                HotAction::UpdateToolPolicy => {
                    info!(
                        "Hot-reload: updating tool policy ({} global rules, {} agent rules)",
                        new_config.tool_policy.global_rules.len(),
                        new_config.tool_policy.agent_rules.len(),
                    );
                    let mut guard = self
                        .tool_policy_override
                        .write()
                        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
                    *guard = Some(new_config.tool_policy.clone());
                }
                HotAction::UpdateProactiveMemory => {
                    info!("Hot-reload: updating proactive memory config");
                    if let Some(pm) = self.proactive_memory.get() {
                        pm.update_config(new_config.proactive_memory.clone());
                    }
                }
                HotAction::ReloadChannels => {
                    // Channel adapters are registered at bridge startup. Clear
                    // existing adapters so they are re-created with the new config
                    // on the next bridge cycle.
                    info!(
                        "Hot-reload: channel config updated — clearing {} adapter(s), \
                         will reinitialize on next bridge cycle",
                        self.channel_adapters.len()
                    );
                    self.channel_adapters.clear();
                }
                HotAction::ReloadSkills => {
                    self.reload_skills();
                }
                HotAction::UpdateUsageFooter => {
                    info!(
                        "Hot-reload: usage footer mode updated to {:?} \
                         (takes effect on next response)",
                        new_config.usage_footer
                    );
                }
                HotAction::ReloadWebConfig => {
                    info!(
                        "Hot-reload: web config updated (search_provider={:?}, \
                         cache_ttl={}min) — takes effect on next web tool invocation",
                        new_config.web.search_provider, new_config.web.cache_ttl_minutes
                    );
                }
                HotAction::ReloadBrowserConfig => {
                    info!(
                        "Hot-reload: browser config updated (headless={}) \
                         — new sessions will use updated config",
                        new_config.browser.headless
                    );
                }
                HotAction::UpdateWebhookConfig => {
                    let enabled = new_config
                        .webhook_triggers
                        .as_ref()
                        .map(|w| w.enabled)
                        .unwrap_or(false);
                    info!("Hot-reload: webhook trigger config updated (enabled={enabled})");
                }
                HotAction::ReloadExtensions => {
                    info!("Hot-reload: reloading MCP catalog");
                    // Atomic swap — readers in flight keep the old snapshot.
                    let count = self.mcp_catalog_reload(&new_config.home_dir);
                    info!("Hot-reload: reloaded {count} MCP catalog entry/entries");
                    // Effective MCP server list now == config.mcp_servers directly.
                    let new_mcp = new_config.mcp_servers.clone();
                    let mut effective = self
                        .effective_mcp_servers
                        .write()
                        .unwrap_or_else(|e| e.into_inner());
                    *effective = new_mcp;
                    info!(
                        "Hot-reload: effective MCP server list updated ({} total)",
                        effective.len()
                    );
                    // Bump MCP generation so tool list caches are invalidated
                    self.mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                HotAction::ReloadMcpServers => {
                    info!("Hot-reload: MCP server config updated");
                    let new_mcp = new_config.mcp_servers.clone();

                    // Snapshot the previous effective list so we can diff
                    // which entries actually changed. Existing connections
                    // hold a per-server `McpServerConfig` clone (including
                    // `taint_policy`/`taint_scanning`/`headers`/`env`/
                    // `transport`), so any field that is not behind a shared
                    // `ArcSwap` (only `taint_rule_sets` is) requires a
                    // disconnect+reconnect for the new value to reach
                    // in-flight tool calls. Without this, edits via PUT
                    // `/api/mcp/servers/{name}`, CLI `config.toml` edits,
                    // or any non-PATCH path would silently keep the old
                    // policy alive on already-connected servers.
                    let old_mcp = self
                        .effective_mcp_servers
                        .read()
                        .map(|s| s.clone())
                        .unwrap_or_default();

                    let new_by_name: std::collections::HashMap<&str, _> =
                        new_mcp.iter().map(|s| (s.name.as_str(), s)).collect();
                    let mut to_reconnect: Vec<String> = Vec::new();
                    for old_entry in &old_mcp {
                        match new_by_name.get(old_entry.name.as_str()) {
                            None => {
                                // Removed: stale connection still alive in
                                // `mcp_connections` until we evict it.
                                to_reconnect.push(old_entry.name.clone());
                            }
                            Some(new_entry) => {
                                // Modified: serialize-compare is robust
                                // against future field additions and avoids
                                // forcing `PartialEq` onto every nested
                                // config type (`McpTaintPolicy`,
                                // `McpOAuthConfig`, transport variants…).
                                let old_json = serde_json::to_string(old_entry).unwrap_or_default();
                                let new_json =
                                    serde_json::to_string(*new_entry).unwrap_or_default();
                                if old_json != new_json {
                                    to_reconnect.push(old_entry.name.clone());
                                }
                            }
                        }
                    }

                    let mut effective = self
                        .effective_mcp_servers
                        .write()
                        .unwrap_or_else(|e| e.into_inner());
                    // Diff the health registry against the new server set so
                    // removed servers stop being tracked and newly added ones
                    // enter the map immediately — otherwise `report_ok` /
                    // `report_error` are silent no-ops for those IDs and
                    // `/api/mcp/health` under-reports until a full restart.
                    let old_names: std::collections::HashSet<String> =
                        effective.iter().map(|s| s.name.clone()).collect();
                    let new_names: std::collections::HashSet<String> =
                        new_mcp.iter().map(|s| s.name.clone()).collect();
                    for name in old_names.difference(&new_names) {
                        self.mcp_health.unregister(name);
                    }
                    for name in new_names.difference(&old_names) {
                        self.mcp_health.register(name);
                    }
                    let count = new_mcp.len();
                    *effective = new_mcp;
                    drop(effective);

                    // Bump MCP generation so tool list caches are invalidated
                    self.mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if to_reconnect.is_empty() {
                        info!(
                            "Hot-reload: effective MCP server list rebuilt \
                             ({count} total, no reconnects needed)"
                        );
                    } else {
                        info!(
                            servers = ?to_reconnect,
                            "Hot-reload: effective MCP server list rebuilt \
                             ({count} total, {} server(s) need reconnection \
                             to apply config changes)",
                            to_reconnect.len()
                        );
                        // Fire-and-forget: `disconnect_mcp_server` drops the
                        // stale slot and `connect_mcp_servers` is idempotent
                        // (re-adds servers missing from `mcp_connections`
                        // using the now-updated effective list).
                        if let Some(weak) = self.self_handle.get() {
                            if let Some(kernel) = weak.upgrade() {
                                spawn_logged("mcp_reconnect", async move {
                                    for name in &to_reconnect {
                                        kernel.disconnect_mcp_server(name).await;
                                    }
                                    kernel.connect_mcp_servers().await;
                                });
                            } else {
                                tracing::warn!(
                                    server_count = to_reconnect.len(),
                                    "Hot-reload: kernel self-handle dropped \
                                     — MCP servers will keep stale config \
                                     until next restart"
                                );
                            }
                        }
                    }
                }
                HotAction::ReloadA2aConfig => {
                    info!(
                        "Hot-reload: A2A config updated — takes effect on next \
                         discovery/send operation"
                    );
                }
                HotAction::ReloadFallbackProviders => {
                    let count = new_config.fallback_providers.len();
                    info!("Hot-reload: fallback provider chain updated ({count} provider(s))");
                    // Invalidate cached LLM drivers so the new fallback chain
                    // is used when drivers are next constructed.
                    self.driver_cache.clear();
                }
                HotAction::ReloadProviderApiKeys => {
                    info!("Hot-reload: provider API keys changed — flushing driver cache");
                    self.driver_cache.clear();
                }
                HotAction::ReloadProxy => {
                    info!("Hot-reload: proxy config changed — reinitializing HTTP proxy env");
                    librefang_runtime::http_client::init_proxy(new_config.proxy.clone());
                    self.driver_cache.clear();
                }
                HotAction::UpdateDashboardCredentials => {
                    info!("Hot-reload: dashboard credentials updated — config swap is sufficient");
                }
                HotAction::ReloadAuth => {
                    info!(
                        "Hot-reload: rebuilding AuthManager ({} users, {} tool groups)",
                        new_config.users.len(),
                        new_config.tool_policy.groups.len(),
                    );
                    self.security
                        .auth
                        .reload(&new_config.users, &new_config.tool_policy.groups);
                    // Re-validate channel-role-mapping role strings on
                    // every reload so an operator who just edited the
                    // config and introduced a typo sees a WARN instead
                    // of silent default-deny on the next message.
                    let typos = crate::auth::validate_channel_role_mapping(
                        &new_config.channel_role_mapping,
                    );
                    if typos > 0 {
                        warn!(
                            "Hot-reload: channel_role_mapping has {typos} typo'd role \
                             string(s) — see WARN lines above"
                        );
                    }
                }
                HotAction::ReloadTaintRules => {
                    // Actual swap is performed by the caller (`reload_config`)
                    // after this match completes — this arm is informational
                    // only. Logging here keeps the action visible alongside
                    // every other hot reload in the audit trail.
                    info!(
                        "Hot-reload: [[taint_rules]] registry updated — \
                         next MCP scan will see new rule sets without reconnect"
                    );
                }
                HotAction::ReloadLogLevel(level) => match self.log_reloader.get() {
                    Some(reloader) => match reloader.reload(level) {
                        Ok(()) => info!("Hot-reload: log_level updated to {level}"),
                        Err(e) => warn!("Hot-reload: log_level update to {level} failed: {e}"),
                    },
                    None => warn!(
                        "Hot-reload: log_level changed to {level} but no reloader is installed; \
                         restart required for the new filter to take effect"
                    ),
                },
                HotAction::UpdateQueueConcurrency => {
                    use librefang_runtime::command_lane::Lane;
                    let cc = &new_config.queue.concurrency;
                    info!(
                        "Hot-reload: resizing lane semaphores (main={}, cron={}, subagent={}, trigger={})",
                        cc.main_lane, cc.cron_lane, cc.subagent_lane, cc.trigger_lane,
                    );
                    // Per-agent caps (cc.default_per_agent, agent.toml's
                    // max_concurrent_invocations) are NOT rebuilt — those
                    // semaphores are owned by individual agents. Operators
                    // need to respawn the agent for those to apply.
                    self.command_queue
                        .resize_lane(Lane::Main, cc.main_lane as u32);
                    self.command_queue
                        .resize_lane(Lane::Cron, cc.cron_lane as u32);
                    self.command_queue
                        .resize_lane(Lane::Subagent, cc.subagent_lane as u32);
                    self.command_queue
                        .resize_lane(Lane::Trigger, cc.trigger_lane as u32);
                }
            }
        }

        // Invalidate prompt metadata cache so next message picks up any
        // config-driven changes (workspace paths, skill config, etc.).
        self.prompt_metadata_cache.invalidate_all();

        // Invalidate the manifest cache so newly installed/removed
        // agents are picked up on the next routing call.
        router::invalidate_manifest_cache();
        router::invalidate_hand_route_cache();
    }
}
