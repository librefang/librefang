//! Cluster pulled out of mod.rs in #4713 phase 3d.
//!
//! Hosts the agent-spawn surface: public entry points
//! (`spawn_agent`, `spawn_agent_with_source`,
//! `spawn_agent_with_parent`) plus the private
//! `spawn_agent_with_parent_and_source` / `spawn_agent_inner` core that
//! wires manifest validation, registry insert, scheduler bootstrap, and
//! channel registration; and the signed-manifest verification helpers
//! (`verify_signed_manifest`, `trusted_manifest_signer_keys`).
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Spawn a new agent from a manifest, optionally linking to a parent agent.
    pub fn spawn_agent(&self, manifest: AgentManifest) -> KernelResult<AgentId> {
        self.spawn_agent_with_source(manifest, None)
    }

    /// Spawn a new agent from a manifest and record its source TOML path.
    pub fn spawn_agent_with_source(
        &self,
        manifest: AgentManifest,
        source_toml_path: Option<PathBuf>,
    ) -> KernelResult<AgentId> {
        self.spawn_agent_with_parent_and_source(manifest, None, source_toml_path)
    }

    /// Spawn a new agent with an optional parent for lineage tracking.
    pub fn spawn_agent_with_parent(
        &self,
        manifest: AgentManifest,
        parent: Option<AgentId>,
    ) -> KernelResult<AgentId> {
        self.spawn_agent_with_parent_and_source(manifest, parent, None)
    }

    /// Spawn a new agent with optional parent and source TOML path.
    fn spawn_agent_with_parent_and_source(
        &self,
        manifest: AgentManifest,
        parent: Option<AgentId>,
        source_toml_path: Option<PathBuf>,
    ) -> KernelResult<AgentId> {
        self.spawn_agent_inner(manifest, parent, source_toml_path, None)
    }

    /// Spawn a new agent with all options including a predetermined ID.
    pub(crate) fn spawn_agent_inner(
        &self,
        manifest: AgentManifest,
        parent: Option<AgentId>,
        source_toml_path: Option<PathBuf>,
        predetermined_id: Option<AgentId>,
    ) -> KernelResult<AgentId> {
        let name = manifest.name.clone();

        // SECURITY (#3533): reject manifest `module` strings that escape
        // the LibreFang home dir before any further work. See
        // `validate_manifest_module_path` for the full rationale and the
        // sibling enforcement points (boot restore, hot reload,
        // update_manifest).
        validate_manifest_module_path(&manifest, &name)?;

        // tool_exec backend (#3332): if the manifest pins a backend
        // override, the matching subtable must exist on the global
        // config — otherwise the agent would fail at the first tool
        // call, not at spawn. We do this before locking the registry so
        // a misconfigured override never advances past spawn. Lost
        // during the kernel/mod split; restored here.
        if let Some(override_kind) = manifest.tool_exec_backend {
            if let Err(e) = self
                .config
                .load()
                .tool_exec
                .validate_override(override_kind)
            {
                return Err(KernelError::LibreFang(LibreFangError::InvalidInput(
                    format!(
                        "agent {name:?} tool_exec_backend = {:?} but {e}",
                        override_kind.as_str()
                    ),
                )));
            }
        }

        // Use a deterministic agent ID derived from the agent name so the
        // same agent gets the same UUID across daemon restarts. This preserves
        // session history associations in SQLite. Child agents spawned at
        // runtime still use random IDs (via predetermined_id = None + parent).
        //
        // Refs #4614 — canonical UUID registry: for top-level agents
        // (`parent.is_none()`), consult `agent_identities` first. If a prior
        // spawn already registered a UUID for this name, reuse it verbatim
        // — even if the v5 derivation later changes (namespace bump, name
        // normalisation tweak), the agent's history stays reachable. If no
        // entry exists yet, fall back to the historical
        // `AgentId::from_name(&name)` derivation and atomically register
        // it as the canonical UUID for this name (first-spawn wins).
        let agent_id = predetermined_id.unwrap_or_else(|| {
            if parent.is_none() {
                if let Some(existing) = self.agent_identities.get(&name) {
                    debug!(
                        agent = %name,
                        id = %existing,
                        "Reusing canonical UUID from agent_identities registry (#4614)"
                    );
                    existing
                } else {
                    let derived = AgentId::from_name(&name);
                    let recorded = self.agent_identities.register_if_absent(&name, derived);
                    if recorded != derived {
                        // Someone else won the race; honor their UUID.
                        debug!(
                            agent = %name,
                            chosen = %recorded,
                            derived = %derived,
                            "agent_identities: lost register race, honoring existing entry"
                        );
                    }
                    recorded
                }
            } else {
                AgentId::new()
            }
        });

        // Restore the most recent session for this agent if one exists in the
        // database, so conversation history survives daemon restarts.
        let session_id = self
            .memory
            .get_agent_session_ids(agent_id)
            .ok()
            .and_then(|ids| ids.into_iter().next())
            .unwrap_or_default();

        // SECURITY: If this spawn is linked to a running parent agent,
        // enforce that the child's capabilities are a subset of the
        // parent's. The `spawn_agent` tool runner and WASM host-call
        // paths already call `spawn_agent_checked` which runs the same
        // check, but pushing it down here closes every future code path
        // that routes through `spawn_agent_with_parent` (channel
        // handlers, LLM routing, workflow engines, bulk spawn, …) by
        // default instead of relying on each caller to remember the
        // wrapper. Top-level spawns (HTTP API, boot-time assistant,
        // channel bootstrap) pass `parent = None` and are unaffected —
        // they're an owner action, not a privilege inheritance.
        if let Some(parent_id) = parent {
            if let Some(parent_entry) = self.registry.get(parent_id) {
                let parent_caps = manifest_to_capabilities(&parent_entry.manifest);
                let child_caps = manifest_to_capabilities(&manifest);
                if let Err(violation) = librefang_types::capability::validate_capability_inheritance(
                    &parent_caps,
                    &child_caps,
                ) {
                    warn!(
                        agent = %name,
                        parent = %parent_id,
                        %violation,
                        "Rejecting child spawn — requested capabilities exceed parent"
                    );
                    return Err(KernelError::LibreFang(
                        librefang_types::error::LibreFangError::Internal(format!(
                            "Privilege escalation denied: {violation}"
                        )),
                    ));
                }
            } else {
                warn!(
                    agent = %name,
                    parent = %parent_id,
                    "Parent agent is not registered — rejecting child spawn to fail closed"
                );
                return Err(KernelError::LibreFang(
                    librefang_types::error::LibreFangError::Internal(format!(
                        "Privilege escalation denied: parent agent {parent_id} is not registered"
                    )),
                ));
            }
        }

        info!(agent = %name, id = %agent_id, parent = ?parent, "Spawning agent");

        // Create the backing session now; prompt injection happens after
        // registration so agent-scoped metadata is visible.
        let mut session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::LibreFang)?;

        // Inherit kernel exec_policy as fallback if agent manifest doesn't have one.
        // Exception: if the agent declares shell_exec in capabilities.tools, promote
        // to Full mode so the tool actually works rather than silently being blocked.
        let cfg = self.config.load();
        let mut manifest = manifest;
        if manifest.exec_policy.is_none() {
            if manifest
                .capabilities
                .tools
                .iter()
                .any(|t| t == "shell_exec" || t == "*")
            {
                manifest.exec_policy = Some(librefang_types::config::ExecPolicy {
                    mode: librefang_types::config::ExecSecurityMode::Full,
                    ..cfg.exec_policy.clone()
                });
            } else {
                manifest.exec_policy = Some(cfg.exec_policy.clone());
            }
        }
        info!(agent = %name, id = %agent_id, exec_mode = ?manifest.exec_policy.as_ref().map(|p| &p.mode), "Agent exec_policy resolved");

        // Normalize empty provider/model to "default" so the intent is preserved in DB.
        // Resolution to concrete values happens at execute_llm_agent time, ensuring
        // provider changes take effect immediately without re-spawning agents.
        {
            let is_default_provider =
                manifest.model.provider.is_empty() || manifest.model.provider == "default";
            let is_default_model =
                manifest.model.model.is_empty() || manifest.model.model == "default";
            if is_default_provider && is_default_model {
                manifest.model.provider = "default".to_string();
                manifest.model.model = "default".to_string();
            }
        }

        // Normalize: strip provider prefix from model name if present
        let normalized = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
        if normalized != manifest.model.model {
            manifest.model.model = normalized;
        }

        // Apply global budget defaults to agent resource quotas
        apply_budget_defaults(&self.budget_config(), &mut manifest.resources);

        // Create workspace directory for the agent.
        // Hand agents set a relative workspace path (hands/<hand>/<role>) resolved
        // against the workspaces root. Standalone agents go to workspaces/agents/<name>.
        let workspaces_root = if manifest.workspace.is_some() {
            cfg.effective_workspaces_dir()
        } else {
            cfg.effective_agent_workspaces_dir()
        };
        let workspace_dir = resolve_workspace_dir(
            &workspaces_root,
            manifest.workspace.clone(),
            &name,
            agent_id,
        )?;
        ensure_workspace(&workspace_dir)?;
        migrate_identity_files(&workspace_dir);
        let resolved_workspaces = ensure_named_workspaces(
            &cfg.effective_workspaces_dir(),
            &manifest.workspaces,
            &cfg.allowed_mount_roots,
        );
        if manifest.generate_identity_files {
            generate_identity_files(&workspace_dir, &manifest, &resolved_workspaces);
        }
        manifest.workspace = Some(workspace_dir);

        // Register capabilities
        let caps = manifest_to_capabilities(&manifest);
        self.capabilities.grant(agent_id, caps);

        // Register with scheduler
        self.scheduler
            .register(agent_id, manifest.resources.clone());

        // Create registry entry
        let tags = manifest.tags.clone();
        let is_hand = tags.iter().any(|t| t.starts_with("hand:"));
        let entry = AgentEntry {
            id: agent_id,
            name: manifest.name.clone(),
            manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent,
            children: vec![],
            session_id,
            source_toml_path,
            tags,
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand,
            ..Default::default()
        };
        self.registry
            .register(entry.clone())
            .map_err(KernelError::LibreFang)?;

        // Inject reset/context prompts only after the agent is registered so
        // agent-scoped injections and tag-gated global injections are visible.
        self.inject_reset_prompt(&mut session, agent_id);

        // Fire external session:start hook for the newly created session.
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": session.id.0.to_string(),
            }),
        );

        // Update parent's children list
        if let Some(parent_id) = parent {
            self.registry.add_child(parent_id, agent_id);
        }

        // Persist agent to SQLite so it survives restarts
        self.memory
            .save_agent(&entry)
            .map_err(KernelError::LibreFang)?;

        info!(agent = %name, id = %agent_id, "Agent spawned");

        // SECURITY: Record agent spawn in audit trail
        self.metering.audit_log.record(
            agent_id.to_string(),
            librefang_runtime::audit::AuditAction::AgentSpawn,
            format!("name={name}, parent={parent:?}"),
            "ok",
        );

        // For proactive agents spawned at runtime, auto-register triggers.
        // Skip any pattern already present (e.g. reloaded from trigger_jobs.json on restart).
        if let ScheduleMode::Proactive { conditions } = &entry.manifest.schedule {
            let mut registered = false;
            for condition in conditions {
                if let Some(pattern) = background::parse_condition(condition) {
                    if self
                        .workflows
                        .triggers
                        .agent_has_pattern(agent_id, &pattern)
                    {
                        continue;
                    }
                    let prompt = format!(
                        "[PROACTIVE ALERT] Condition '{condition}' matched: {{{{event}}}}. \
                         Review and take appropriate action. Agent: {name}"
                    );
                    self.workflows
                        .triggers
                        .register(agent_id, pattern, prompt, 0);
                    registered = true;
                }
            }
            if registered {
                if let Err(e) = self.workflows.triggers.persist() {
                    warn!(agent = %name, "Failed to persist proactive triggers: {e}");
                }
            }
        }

        // Publish lifecycle event (triggers evaluated synchronously on the event)
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id,
                name: name.clone(),
            }),
        );
        // Evaluate triggers synchronously (we can't await in a sync fn, so just evaluate)
        let (triggered, trigger_state_mutated) = self
            .workflows
            .triggers
            .evaluate_with_resolver(&event, |id| self.registry.get(id).map(|e| e.name.clone()));
        if !triggered.is_empty() || trigger_state_mutated {
            if let Err(e) = self.workflows.triggers.persist() {
                warn!("Failed to persist trigger jobs after spawn event: {e}");
            }
        }

        Ok(agent_id)
    }

    /// Verify a signed manifest envelope (Ed25519 + SHA-256).
    ///
    /// Call this before `spawn_agent` when a `SignedManifest` JSON is provided
    /// alongside the TOML. Returns the verified manifest TOML string on success.
    ///
    /// Rejects envelopes whose `signer_public_key` is not listed in
    /// `KernelConfig.trusted_manifest_signers`. An empty trust list is
    /// treated as "no manifests are trusted" and fails closed — otherwise
    /// a self-signed attacker envelope is indistinguishable from a
    /// legitimate one and would silently spawn with attacker-declared
    /// capabilities.
    pub fn verify_signed_manifest(&self, signed_json: &str) -> KernelResult<String> {
        let signed: librefang_types::manifest_signing::SignedManifest =
            serde_json::from_str(signed_json).map_err(|e| {
                KernelError::LibreFang(librefang_types::error::LibreFangError::Config(format!(
                    "Invalid signed manifest JSON: {e}"
                )))
            })?;

        let trusted = self.trusted_manifest_signer_keys()?;
        signed.verify_with_trusted_keys(&trusted).map_err(|e| {
            KernelError::LibreFang(librefang_types::error::LibreFangError::Config(format!(
                "Manifest signature verification failed: {e}"
            )))
        })?;
        info!(signer = %signed.signer_id, hash = %signed.content_hash, "Signed manifest verified");
        Ok(signed.manifest)
    }

    /// Decode `KernelConfig.trusted_manifest_signers` (hex-encoded Ed25519
    /// public keys) into the `[u8; 32]` form expected by
    /// `SignedManifest::verify_with_trusted_keys`. Invalid entries are
    /// rejected — we'd rather fail closed than silently skip malformed
    /// trust anchors.
    fn trusted_manifest_signer_keys(&self) -> KernelResult<Vec<[u8; 32]>> {
        let cfg = self.config.load();
        let mut keys = Vec::with_capacity(cfg.trusted_manifest_signers.len());
        for entry in &cfg.trusted_manifest_signers {
            let bytes = hex::decode(entry.trim()).map_err(|e| {
                KernelError::LibreFang(librefang_types::error::LibreFangError::Config(format!(
                    "trusted_manifest_signers entry {entry:?} is not valid hex: {e}"
                )))
            })?;
            let fixed: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
                KernelError::LibreFang(librefang_types::error::LibreFangError::Config(format!(
                    "trusted_manifest_signers entry {entry:?} is {} bytes, expected 32",
                    v.len()
                )))
            })?;
            keys.push(fixed);
        }
        Ok(keys)
    }
}
