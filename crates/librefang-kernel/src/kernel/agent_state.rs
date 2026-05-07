//! Cluster pulled out of mod.rs in #4713 phase 3d.
//!
//! Hosts dashboard-driven manifest mutation flows:
//! `persist_manifest_to_disk`, `set_agent_model`, `reload_agent_from_disk`,
//! `update_manifest`, `set_agent_skills`, `set_agent_mcp_servers`,
//! `set_agent_tool_filters`. Each public entry point applies the change
//! to the in-memory `AgentRegistry` entry, invalidates relevant prompt
//! caches, and persists the resulting manifest back to `agent.toml`.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Switch an agent's model.
    ///
    /// When `explicit_provider` is `Some`, that provider name is used as-is
    /// (respecting the user's custom configuration). When `None`, the provider
    /// is auto-detected from the model catalog or inferred from the model name,
    /// but only if the agent does NOT have a custom `base_url` configured.
    /// Agents with a custom `base_url` keep their current provider unless
    /// overridden explicitly — this prevents custom setups (e.g. Tencent,
    /// Azure, or other third-party endpoints) from being misidentified.
    /// Persist an agent's manifest to its `agent.toml` on disk so that
    /// dashboard-driven config changes (model, provider, fallback, etc.)
    /// survive a restart. The on-disk file lives at the entry's recorded
    /// `source_toml_path`, falling back to the canonical
    /// `<agent_workspaces_dir>/<safe_name>/agent.toml` when no source path
    /// is set.
    ///
    /// This is best-effort: a failure to write is logged but does not
    /// propagate as an error — the authoritative copy lives in SQLite.
    pub fn persist_manifest_to_disk(&self, agent_id: AgentId) {
        let Some(entry) = self.registry.get(agent_id) else {
            return;
        };
        let toml_path = match entry.source_toml_path.clone() {
            Some(p) => p,
            None => {
                let safe_name = safe_path_component(&entry.name, "agent");
                self.config
                    .load()
                    .effective_agent_workspaces_dir()
                    .join(safe_name)
                    .join("agent.toml")
            }
        };
        let dir = match toml_path.parent() {
            Some(d) => d.to_path_buf(),
            None => {
                warn!(agent = %entry.name, "Failed to derive parent dir for manifest persist");
                return;
            }
        };
        match toml::to_string_pretty(&entry.manifest) {
            Ok(toml_str) => {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    warn!(agent = %entry.name, "Failed to create agent dir for manifest persist: {e}");
                    return;
                }
                if let Err(e) = atomic_write_toml(&toml_path, &toml_str) {
                    warn!(agent = %entry.name, "Failed to persist manifest to disk: {e}");
                } else {
                    debug!(agent = %entry.name, path = %toml_path.display(), "Persisted manifest to disk");
                }
            }
            Err(e) => {
                warn!(agent = %entry.name, "Failed to serialize manifest to TOML: {e}");
            }
        }
    }

    pub fn set_agent_model(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
    ) -> KernelResult<()> {
        let provider = if let Some(ep) = explicit_provider {
            // User explicitly set the provider — use it as-is
            Some(ep.to_string())
        } else {
            // Check whether the agent has a custom base_url, which indicates
            // a user-configured provider endpoint. In that case, preserve the
            // current provider name instead of overriding it with auto-detection.
            let has_custom_url = self
                .registry
                .get(agent_id)
                .map(|e| e.manifest.model.base_url.is_some())
                .unwrap_or(false);

            if has_custom_url {
                // Keep the current provider — don't let auto-detection override
                // a deliberately configured custom endpoint.
                None
            } else {
                // No custom base_url: safe to auto-detect from catalog / model name
                let resolved_provider = self
                    .llm
                    .model_catalog
                    .load()
                    .find_model(model)
                    .map(|entry| entry.provider.clone());
                resolved_provider.or_else(|| infer_provider_from_model(model))
            }
        };

        // Strip the provider prefix from the model name (e.g. "openrouter/deepseek/deepseek-chat" → "deepseek/deepseek-chat")
        let normalized_model = if let Some(ref prov) = provider {
            strip_provider_prefix(model, prov)
        } else {
            model.to_string()
        };

        // Snapshot the full model state for rollback on DB persist failure (#3499).
        let prev_model_state = self.registry.get(agent_id).map(|e| {
            (
                e.manifest.model.model.clone(),
                e.manifest.model.provider.clone(),
                e.manifest.model.api_key_env.clone(),
                e.manifest.model.base_url.clone(),
            )
        });

        if let Some(provider) = provider {
            // When the provider changes, also clear any per-agent api_key_env
            // and base_url overrides — they belonged to the previous provider
            // and would route subsequent requests to the wrong endpoint with
            // the wrong credentials. resolve_driver falls back to the global
            // [provider_api_keys] / [provider_urls] tables (or convention) for
            // the new provider, which is what the user expects when picking a
            // model from the dashboard. When the provider is unchanged we
            // leave the override fields alone so that genuine per-agent
            // overrides on the same provider are preserved.
            let prev_provider = self
                .registry
                .get(agent_id)
                .map(|e| e.manifest.model.provider.clone());
            let provider_changed = prev_provider.as_deref() != Some(provider.as_str());
            if provider_changed {
                self.registry
                    .update_model_provider_config(
                        agent_id,
                        normalized_model.clone(),
                        provider.clone(),
                        None,
                        None,
                    )
                    .map_err(KernelError::LibreFang)?;
            } else {
                self.registry
                    .update_model_and_provider(agent_id, normalized_model.clone(), provider.clone())
                    .map_err(KernelError::LibreFang)?;
            }
            info!(agent_id = %agent_id, model = %normalized_model, provider = %provider, "Agent model+provider updated");
        } else {
            self.registry
                .update_model(agent_id, normalized_model.clone())
                .map_err(KernelError::LibreFang)?;
            info!(agent_id = %agent_id, model = %normalized_model, "Agent model updated (provider unchanged)");
        }

        // Persist the updated entry. On DB failure, roll back the in-memory model
        // mutation and propagate the error so the API caller sees a 500 instead of
        // silently drifting registry vs. disk (#3499).
        if let Some(entry) = self.registry.get(agent_id) {
            if let Err(e) = self.memory.save_agent(&entry) {
                if let Some((p_model, p_provider, p_api_key_env, p_base_url)) = prev_model_state {
                    let _ = self.registry.update_model_provider_config(
                        agent_id,
                        p_model,
                        p_provider,
                        p_api_key_env,
                        p_base_url,
                    );
                }
                return Err(KernelError::LibreFang(e));
            }
        }

        // Write updated manifest to agent.toml so changes survive restart (#996, #1018)
        self.persist_manifest_to_disk(agent_id);

        // Clear canonical session to prevent memory poisoning from old model's responses
        let _ = self.memory.delete_canonical_session(agent_id);
        debug!(agent_id = %agent_id, "Cleared canonical session after model switch");

        Ok(())
    }

    /// Reload an agent's manifest from its source agent.toml on disk.
    ///
    /// At boot the kernel reads agent.toml and syncs it into the in-memory
    /// registry, but runtime edits to the file are otherwise invisible until
    /// the next restart. This method re-reads the file, preserves
    /// runtime-only fields that TOML doesn't carry (workspace path, tags,
    /// current enabled state), replaces the in-memory manifest, persists it
    /// to the DB, and invalidates the tool cache so the updated skill / MCP
    /// allowlists take effect on the next message.
    pub fn reload_agent_from_disk(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let fallback_toml_path = {
            let safe_name = safe_path_component(&entry.name, "agent");
            self.config
                .load()
                .effective_agent_workspaces_dir()
                .join(safe_name)
                .join("agent.toml")
        };
        // Prefer stored source path when it still exists; otherwise fall back
        // to the canonical workspaces/agents/<name>/ location so entries with
        // a stale legacy source_toml_path self-heal after boot migration.
        let toml_path = match entry.source_toml_path.clone() {
            Some(p) if p.exists() => p,
            _ => fallback_toml_path,
        };

        if !toml_path.exists() {
            return Err(KernelError::LibreFang(LibreFangError::Internal(format!(
                "agent.toml not found at {}",
                toml_path.display()
            ))));
        }

        // `block_in_place` so this sync read does not park a tokio worker
        // thread for I/O while holding no locks.
        let toml_str = tokio::task::block_in_place(|| std::fs::read_to_string(&toml_path))
            .map_err(|e| {
                KernelError::LibreFang(LibreFangError::Internal(format!(
                    "Failed to read {}: {e}",
                    toml_path.display()
                )))
            })?;

        // Try the hand-extraction path FIRST, then fall back to flat AgentManifest.
        // See the boot loop for the rationale — AgentManifest::deserialize is lenient
        // enough to accept a hand.toml and silently produce a stub manifest with
        // the default "You are a helpful AI agent." system prompt.
        let mut disk_manifest: librefang_types::agent::AgentManifest =
            extract_manifest_from_hand_toml(&toml_str, &entry.name)
                .or_else(|| toml::from_str::<librefang_types::agent::AgentManifest>(&toml_str).ok())
                .ok_or_else(|| {
                    KernelError::LibreFang(LibreFangError::Internal(format!(
                        "Invalid TOML in {}: not an agent manifest or hand definition",
                        toml_path.display()
                    )))
                })?;

        // SECURITY (#3533): hot-reload is a separate code path from
        // spawn — without this check an operator (or anyone with TOML
        // write access) could swap a running agent's `module` for an
        // absolute / `..`-traversing host path and have the next
        // invocation exec it. Reject before touching the registry so
        // the previous (validated) manifest stays in effect.
        validate_manifest_module_path(&disk_manifest, &entry.name)?;

        // Preserve workspace if TOML leaves it unset — workspace is
        // populated at spawn time with the real directory path.
        if disk_manifest.workspace.is_none() {
            disk_manifest.workspace = entry.manifest.workspace.clone();
        }
        // Always preserve the name. Renaming would also need to update
        // `entry.name` and the registry's `name_index`, which reload does
        // not touch — a renamed manifest without those updates would
        // silently break `find_by_name` lookups. Use the rename API.
        disk_manifest.name = entry.manifest.name.clone();
        // Always preserve tags for the same reason: there is no runtime
        // API to update `entry.tags` or the registry's `tag_index`, both
        // of which are a snapshot taken at spawn time. Letting reload
        // change `manifest.tags` would desync manifest tags from the
        // tag index used by `find_by_tag()`.
        disk_manifest.tags = entry.manifest.tags.clone();

        self.registry
            .replace_manifest(agent_id, disk_manifest)
            .map_err(KernelError::LibreFang)?;

        if let Some(refreshed) = self.registry.get(agent_id) {
            // Re-grant capabilities in case caps/profile changed in the TOML.
            // Uses insert() so it replaces any existing grants for this agent.
            let caps = manifest_to_capabilities(&refreshed.manifest);
            self.capabilities.grant(agent_id, caps);
            // Refresh the scheduler's quota cache so changes to
            // `max_llm_tokens_per_hour` and friends take effect on the
            // next message instead of waiting for daemon restart.
            // Uses `update_quota` (not `register`) to preserve the
            // accumulated usage tracker — switching the limit shouldn't
            // wipe the running window. Issue #2317.
            self.scheduler
                .update_quota(agent_id, refreshed.manifest.resources.clone());
            let _ = self.memory.save_agent(&refreshed);
        }

        // Invalidate the per-agent tool cache so the new skill/MCP allowlist
        // takes effect on the next message. The skill-summary cache is keyed
        // by allowlist content so it self-invalidates when the list changes.
        self.prompt_metadata_cache.tools.remove(&agent_id);

        info!(agent_id = %agent_id, path = %toml_path.display(), "Reloaded agent manifest from disk");
        Ok(())
    }

    /// Apply a caller-supplied manifest to a running agent and persist it to
    /// disk.  This is the in-memory counterpart of `reload_agent_from_disk`:
    /// instead of reading the TOML file it accepts a pre-parsed manifest,
    /// replaces the registry entry, refreshes capabilities / quota / memory,
    /// invalidates the tool cache, and then persists the new state to
    /// `agent.toml` so the change survives a restart.
    ///
    /// The same invariants as `reload_agent_from_disk` are enforced:
    /// - `name` and `tags` are locked to the current values (use the rename /
    ///   tag APIs to change them)
    /// - `workspace` is preserved when the incoming manifest leaves it unset
    pub fn update_manifest(
        &self,
        agent_id: AgentId,
        mut new_manifest: librefang_types::agent::AgentManifest,
    ) -> KernelResult<()> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // SECURITY (#3533): same path-escape check as spawn / hot-reload.
        // Without it, any caller with `update_manifest` access could
        // swap a running agent's `module` to an arbitrary host script.
        validate_manifest_module_path(&new_manifest, &entry.name)?;

        // Preserve invariants that the registry indices depend on.
        if new_manifest.workspace.is_none() {
            new_manifest.workspace = entry.manifest.workspace.clone();
        }
        new_manifest.name = entry.manifest.name.clone();
        new_manifest.tags = entry.manifest.tags.clone();

        self.registry
            .replace_manifest(agent_id, new_manifest)
            .map_err(KernelError::LibreFang)?;

        if let Some(refreshed) = self.registry.get(agent_id) {
            let caps = manifest_to_capabilities(&refreshed.manifest);
            self.capabilities.grant(agent_id, caps);
            self.scheduler
                .update_quota(agent_id, refreshed.manifest.resources.clone());
            let _ = self.memory.save_agent(&refreshed);
        }

        // Invalidate the per-agent tool cache so skill/MCP allowlist changes
        // take effect on the next message.
        self.prompt_metadata_cache.tools.remove(&agent_id);

        // Persist to disk so the change survives a daemon restart.
        self.persist_manifest_to_disk(agent_id);

        info!(agent_id = %agent_id, "Applied and persisted updated agent manifest");
        Ok(())
    }

    /// Update an agent's skill allowlist. Empty = all skills (backward compat).
    pub fn set_agent_skills(&self, agent_id: AgentId, skills: Vec<String>) -> KernelResult<()> {
        // Validate skill names if allowlist is non-empty
        if !skills.is_empty() {
            let registry = self
                .skills
                .skill_registry
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let known = registry.skill_names();
            for name in &skills {
                if !known.contains(name) {
                    return Err(KernelError::LibreFang(LibreFangError::Internal(format!(
                        "Unknown skill: {name}"
                    ))));
                }
            }
        }

        // Snapshot previous skill list AND skills_disabled flag so we can roll
        // back the in-memory mutation if the DB persist fails (#3499 — previously
        // `let _ =` swallowed the error and left the registry drifted from disk).
        // Note: capture both fields because `update_skills` always sets
        // `skills_disabled = false`, so a rollback that only restored `skills`
        // would silently leave the disabled flag flipped on persist failure.
        let prev_skills_state = self
            .registry
            .get(agent_id)
            .map(|e| (e.manifest.skills.clone(), e.manifest.skills_disabled));

        self.registry
            .update_skills(agent_id, skills.clone())
            .map_err(KernelError::LibreFang)?;

        if let Some(entry) = self.registry.get(agent_id) {
            if let Err(e) = self.memory.save_agent(&entry) {
                if let Some((p_skills, p_disabled)) = prev_skills_state {
                    let _ = self
                        .registry
                        .restore_skills_state(agent_id, p_skills, p_disabled);
                }
                return Err(KernelError::LibreFang(e));
            }
        }

        // Invalidate cached tool list — skill allowlist change affects available tools
        self.prompt_metadata_cache.tools.remove(&agent_id);

        info!(agent_id = %agent_id, skills = ?skills, "Agent skills updated");
        Ok(())
    }

    /// Update an agent's MCP server allowlist. Empty = all servers (backward compat).
    pub fn set_agent_mcp_servers(
        &self,
        agent_id: AgentId,
        servers: Vec<String>,
    ) -> KernelResult<()> {
        // Validate server names if allowlist is non-empty
        if !servers.is_empty() {
            if let Ok(mcp_tools) = self.mcp.mcp_tools.lock() {
                let mut known_servers: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let configured_servers: Vec<String> = self
                    .mcp
                    .effective_mcp_servers
                    .read()
                    .map(|servers| servers.iter().map(|s| s.name.clone()).collect())
                    .unwrap_or_default();
                for tool in mcp_tools.iter() {
                    if let Some(s) = librefang_runtime::mcp::resolve_mcp_server_from_known(
                        &tool.name,
                        configured_servers.iter().map(String::as_str),
                    ) {
                        known_servers.insert(librefang_runtime::mcp::normalize_name(s));
                    }
                }
                for name in &servers {
                    let normalized = librefang_runtime::mcp::normalize_name(name);
                    if !known_servers.contains(&normalized) {
                        return Err(KernelError::LibreFang(LibreFangError::Internal(format!(
                            "Unknown MCP server: {name}"
                        ))));
                    }
                }
            }
        }

        // Snapshot previous MCP server allowlist for rollback on DB persist failure (#3499).
        let prev_servers = self
            .registry
            .get(agent_id)
            .map(|e| e.manifest.mcp_servers.clone());

        self.registry
            .update_mcp_servers(agent_id, servers.clone())
            .map_err(KernelError::LibreFang)?;

        if let Some(entry) = self.registry.get(agent_id) {
            if let Err(e) = self.memory.save_agent(&entry) {
                if let Some(p_servers) = prev_servers {
                    let _ = self.registry.update_mcp_servers(agent_id, p_servers);
                }
                return Err(KernelError::LibreFang(e));
            }
        }

        // Invalidate cached tool list — MCP server allowlist change affects available tools
        self.prompt_metadata_cache.tools.remove(&agent_id);

        info!(agent_id = %agent_id, servers = ?servers, "Agent MCP servers updated");
        Ok(())
    }

    /// Update an agent's tool allowlist and/or blocklist.
    pub fn set_agent_tool_filters(
        &self,
        agent_id: AgentId,
        capabilities_tools: Option<Vec<String>>,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> KernelResult<()> {
        if capabilities_tools.is_none() && allowlist.is_none() && blocklist.is_none() {
            return Ok(());
        }

        info!(
            agent_id = %agent_id,
            capabilities_tools = ?capabilities_tools,
            allowlist = ?allowlist,
            blocklist = ?blocklist,
            "Agent tool filters updated"
        );

        // Snapshot previous tool config + tools_disabled flag for rollback on
        // DB persist failure (#3499). Capture all four fields because
        // `update_tool_config` always sets `tools_disabled = false`, so a
        // rollback that only restored the lists would silently leave the
        // disabled flag flipped on persist failure.
        let prev_tool_state = self.registry.get(agent_id).map(|e| {
            (
                e.manifest.capabilities.tools.clone(),
                e.manifest.tool_allowlist.clone(),
                e.manifest.tool_blocklist.clone(),
                e.manifest.tools_disabled,
            )
        });

        self.registry
            .update_tool_config(agent_id, capabilities_tools, allowlist, blocklist)
            .map_err(KernelError::LibreFang)?;

        if let Some(entry) = self.registry.get(agent_id) {
            if let Err(e) = self.memory.save_agent(&entry) {
                if let Some((p_caps, p_allow, p_block, p_disabled)) = prev_tool_state {
                    let _ = self
                        .registry
                        .restore_tool_state(agent_id, p_caps, p_allow, p_block, p_disabled);
                }
                return Err(KernelError::LibreFang(e));
            }
        }

        self.persist_manifest_to_disk(agent_id);

        // Invalidate cached tool list — tool filter change affects available tools
        self.prompt_metadata_cache.tools.remove(&agent_id);

        Ok(())
    }
}
