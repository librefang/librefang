//! Agent registry — tracks all agents, their state, and indexes.

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use librefang_types::agent::{AgentEntry, AgentId, AgentMode, AgentState};
use librefang_types::error::{LibreFangError, LibreFangResult};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Capacity of the registry-change broadcast channel (#3513).
///
/// Subscribers only need to learn "something changed" — not what changed —
/// so the buffer can be small. A lagging receiver is treated as a signal
/// to re-fetch the full state, which is exactly what we want.
const CHANGE_CHANNEL_CAPACITY: usize = 16;

/// Registry of all agents in the kernel.
pub struct AgentRegistry {
    /// Primary index: agent ID → entry.
    ///
    /// Values are stored as `Arc<AgentEntry>` so reads (`list_arcs`,
    /// `is_auto_dream_enabled`, etc.) can hand out cheap pointer clones
    /// instead of deep-cloning the embedded `AgentManifest` (12+ Vecs /
    /// HashMaps). Mutators take the DashMap shard lock and use
    /// `Arc::make_mut` to get a `&mut AgentEntry`: when no readers hold
    /// the Arc this is in-place; when readers do hold it (e.g. a
    /// dashboard handler is mid-iteration over a `list_arcs` snapshot)
    /// `make_mut` clones the entry once and replaces the slot, leaving
    /// the readers' snapshot intact. See #3569.
    agents: DashMap<AgentId, Arc<AgentEntry>>,
    /// Name index: human-readable name → agent ID.
    name_index: DashMap<String, AgentId>,
    /// Tag index: tag → list of agent IDs.
    tag_index: DashMap<String, Vec<AgentId>>,
    /// Broadcast that fires after every successful registry mutation (#3513).
    ///
    /// Replaces the per-WS-client 5s polling loop in `librefang-api/src/ws.rs`
    /// with an event-driven push: every dashboard tab subscribes once and only
    /// rebuilds the agent snapshot when an actual mutation occurred.
    /// Capacity is intentionally small — losing a tick is benign because
    /// receivers re-snapshot from the registry on every signal anyway, and
    /// `RecvError::Lagged` is treated as "send a fresh snapshot".
    changed_tx: broadcast::Sender<()>,
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        let (changed_tx, _) = broadcast::channel(CHANGE_CHANNEL_CAPACITY);
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
            tag_index: DashMap::new(),
            changed_tx,
        }
    }

    /// Subscribe to registry-change events (#3513).
    ///
    /// Each successful mutator (`register`, `remove`, `set_state`, `set_mode`,
    /// `update_*`, `mark_*`, `schedule_session_wipe`, `add_child`, `touch`,
    /// `mark_processed_message`) publishes one `()` after the mutation
    /// completes. Subscribers should re-snapshot the registry on each recv,
    /// and treat `RecvError::Lagged` the same way (snapshot, then keep
    /// listening).
    pub fn subscribe_changes(&self) -> broadcast::Receiver<()> {
        self.changed_tx.subscribe()
    }

    /// Publish a "registry changed" event. Ignores send failures: when no
    /// receivers are connected `send` returns `Err`, which is the expected
    /// state for headless operation (no dashboard, no WS).
    fn notify_changed(&self) {
        let _ = self.changed_tx.send(());
    }

    /// Mutate an existing entry in place under the DashMap shard lock.
    ///
    /// Internally values are `Arc<AgentEntry>`; mutators need `&mut AgentEntry`.
    /// This helper applies `Arc::make_mut` so:
    /// - if no readers hold the Arc, the mutation is in place;
    /// - if readers do (a dashboard handler is mid-iteration over
    ///   `list_arcs`), the entry is cloned once and the slot is replaced,
    ///   leaving the snapshot the readers hold untouched.
    ///
    /// Returns `AgentNotFound` if the agent isn't registered. The closure's
    /// return value is propagated to the caller.
    fn with_entry_mut<R, F>(&self, id: AgentId, f: F) -> LibreFangResult<R>
    where
        F: FnOnce(&mut AgentEntry) -> R,
    {
        let mut slot = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        let inner = Arc::make_mut(slot.value_mut());
        Ok(f(inner))
    }

    /// Register a new agent.
    ///
    /// Publication ordering is load-bearing for concurrent lookups (#3338):
    /// the entry is inserted into `agents` **before** the name binding is
    /// finalized in `name_index`. A concurrent `find_by_name(name)` either
    /// sees no name binding at all, or — once `vacant.insert(id)` runs —
    /// resolves to an `id` that is already addressable via `agents.get(id)`.
    /// This closes the spawn-before-publish gap that surfaced as flaky load
    /// tests in `crates/librefang-api/tests/load_test.rs`.
    pub fn register(&self, entry: AgentEntry) -> LibreFangResult<()> {
        let id = entry.id;
        // Use atomic entry() API to avoid TOCTOU race between contains_key
        // and insert. The Vacant guard holds the DashMap shard lock for
        // this name across the agents/tag publish below, so no concurrent
        // register for the same name can race past the duplicate check.
        match self.name_index.entry(entry.name.clone()) {
            Entry::Occupied(_) => {
                return Err(LibreFangError::AgentAlreadyExists(entry.name));
            }
            Entry::Vacant(vacant) => {
                // Publish the agent entry and tag index entries BEFORE
                // binding the name to the id. Any concurrent reader that
                // resolves the name once `vacant.insert(id)` returns is
                // guaranteed to find the entry under `id`.
                let tags = entry.tags.clone();
                self.agents.insert(id, Arc::new(entry));
                for tag in &tags {
                    self.tag_index.entry(tag.clone()).or_default().push(id);
                }
                vacant.insert(id);
            }
        }
        self.notify_changed();
        Ok(())
    }

    /// Get an agent entry by ID.
    ///
    /// Returns an owned `AgentEntry` for backward compatibility — callers that
    /// hold the value across awaits or move-out continue to work. The clone
    /// happens once at the boundary; internal storage stays `Arc<AgentEntry>`
    /// so a follow-up `list_arcs()` from the same handler costs only a pointer
    /// copy. Hot paths that only need a read view should prefer building on
    /// `list_arcs()` instead.
    pub fn get(&self, id: AgentId) -> Option<AgentEntry> {
        self.agents.get(&id).map(|e| (**e.value()).clone())
    }

    /// Cheap-read accessor: hand back the stored `Arc<AgentEntry>` without
    /// deep-cloning. Hot paths (skill-workshop after-turn hook, metrics
    /// scrapes) call this once per turn per agent and only need read
    /// access. Mirrors [`Self::list_arcs`] but for a single id; identical
    /// snapshot semantics — subsequent registry mutations create copy-on-
    /// write replacements via `Arc::make_mut` and are not visible through
    /// previously returned Arcs.
    pub fn get_arc(&self, id: AgentId) -> Option<Arc<AgentEntry>> {
        self.agents.get(&id).map(|e| Arc::clone(e.value()))
    }

    /// Find an agent by name.
    pub fn find_by_name(&self, name: &str) -> Option<AgentEntry> {
        self.name_index
            .get(name)
            .and_then(|id| self.agents.get(id.value()).map(|e| (**e.value()).clone()))
    }

    /// Touch the agent's `last_active` timestamp without changing any other field.
    /// Used to prevent heartbeat false-positives during long-running operations.
    pub fn touch(&self, id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&id) {
            Arc::make_mut(entry.value_mut()).last_active = chrono::Utc::now();
        }
    }

    /// Flip the sticky `has_processed_message` flag and bump `last_active`.
    ///
    /// Called from the real message-dispatch path
    /// (`execute_llm_agent`) — never from administrative bookkeeping. This
    /// is what the heartbeat monitor checks to distinguish "agent that has
    /// genuinely been alive" from "agent that was spawned and never used".
    /// Idempotent: once `true`, repeated calls only refresh `last_active`.
    pub fn mark_processed_message(&self, id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&id) {
            let inner = Arc::make_mut(entry.value_mut());
            inner.has_processed_message = true;
            inner.last_active = chrono::Utc::now();
        }
    }

    /// Update agent state.
    pub fn set_state(&self, id: AgentId, state: AgentState) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.state = state;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update agent operational mode.
    pub fn set_mode(&self, id: AgentId, mode: AgentMode) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.mode = mode;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Remove an agent from the registry.
    ///
    /// Tear-down ordering mirrors `register` to keep concurrent
    /// `find_by_name` lookups monotone (#3338): the name binding is dropped
    /// **before** the entry is removed from `agents`, so a reader that still
    /// resolves the name will find the entry, and a reader that observes
    /// the name as gone will not be handed a stale id pointing at a
    /// vanished entry.
    pub fn remove(&self, id: AgentId) -> LibreFangResult<AgentEntry> {
        // Snapshot name + tags without removing yet so we can drop the
        // name binding first.
        let (name, tags) = {
            let snap = self
                .agents
                .get(&id)
                .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
            (snap.name.clone(), snap.tags.clone())
        };
        // Drop the name binding first — but only if it still resolves to
        // this id, otherwise a concurrent rename could have re-bound the
        // name to a different agent.
        self.name_index
            .remove_if(&name, |_, mapped_id| *mapped_id == id);
        // Now retract the entry. If a racing remove already took it,
        // surface AgentNotFound rather than silently succeeding.
        let (_, entry_arc) = self
            .agents
            .remove(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        for tag in &tags {
            if let Some(mut ids) = self.tag_index.get_mut(tag) {
                ids.retain(|&agent_id| agent_id != id);
            }
        }
        self.notify_changed();
        // Try to unwrap the Arc to avoid a final clone when nobody else holds
        // a reference. If outstanding `list_arcs()` snapshots still hold the
        // Arc (a dashboard mid-render), fall back to cloning.
        Ok(Arc::try_unwrap(entry_arc).unwrap_or_else(|arc| (*arc).clone()))
    }

    /// List all agents, sorted by name for deterministic ordering.
    ///
    /// Returns owned `AgentEntry` values for backward compatibility — every
    /// entry is deep-cloned out of the registry's internal `Arc<AgentEntry>`.
    /// Hot paths that only need read access should call [`Self::list_arcs`]
    /// instead, which returns `Arc<AgentEntry>` clones (pointer-only).
    pub fn list(&self) -> Vec<AgentEntry> {
        let mut entries: Vec<AgentEntry> =
            self.agents.iter().map(|e| (**e.value()).clone()).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// List all agents as `Arc<AgentEntry>`, sorted by name.
    ///
    /// Cheap: each element is a pointer clone of the registry's internally
    /// stored `Arc<AgentEntry>` — no `AgentEntry::clone` happens (#3569).
    /// Prefer this over `list()` for read-only views (dashboard refreshes,
    /// per-LLM-turn prompt construction, metrics scrapes). Callers must
    /// treat the entries as immutable snapshots; subsequent registry
    /// mutations create copy-on-write replacements via `Arc::make_mut` and
    /// will not be visible through previously returned Arcs.
    pub fn list_arcs(&self) -> Vec<Arc<AgentEntry>> {
        let mut entries: Vec<Arc<AgentEntry>> =
            self.agents.iter().map(|e| Arc::clone(e.value())).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Projects `(name, state_debug, model)` per agent, sorted by name for prompt-cache stability.
    pub fn peer_agents_summary(&self) -> Vec<(String, String, String)> {
        let mut entries: Vec<(String, String, String)> = self
            .agents
            .iter()
            .map(|e| {
                let v = e.value();
                (
                    v.name.clone(),
                    format!("{:?}", v.state),
                    v.manifest.model.model.clone(),
                )
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// Add a child agent ID to a parent's children list.
    pub fn add_child(&self, parent_id: AgentId, child_id: AgentId) {
        let mutated = self
            .with_entry_mut(parent_id, |entry| entry.children.push(child_id))
            .is_ok();
        if mutated {
            self.notify_changed();
        }
    }

    /// Count of registered agents.
    pub fn count(&self) -> usize {
        self.agents.len()
    }

    /// Update an agent's session ID (for session reset).
    pub fn update_session_id(
        &self,
        id: AgentId,
        new_session_id: librefang_types::agent::SessionId,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.session_id = new_session_id;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's workspace path.
    pub fn update_workspace(
        &self,
        id: AgentId,
        workspace: Option<std::path::PathBuf>,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.workspace = workspace;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's source TOML path.
    pub fn update_source_toml_path(
        &self,
        id: AgentId,
        source_toml_path: Option<std::path::PathBuf>,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.source_toml_path = source_toml_path;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Replace an agent's manifest wholesale. The caller is responsible for
    /// preserving runtime-only fields (workspace, tags) and invalidating any
    /// caches that depend on the manifest. Used by `reload_agent_from_disk`.
    pub fn replace_manifest(
        &self,
        id: AgentId,
        manifest: librefang_types::agent::AgentManifest,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest = manifest;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's visual identity (emoji, avatar, color).
    pub fn update_identity(
        &self,
        id: AgentId,
        identity: librefang_types::agent::AgentIdentity,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.identity = identity;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's model configuration.
    pub fn update_model(&self, id: AgentId, new_model: String) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.model.model = new_model;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's model AND provider together.
    pub fn update_model_and_provider(
        &self,
        id: AgentId,
        new_model: String,
        new_provider: String,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.model.model = new_model;
            entry.manifest.model.provider = new_provider;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's model, provider, and connection hints together.
    pub fn update_model_provider_config(
        &self,
        id: AgentId,
        new_model: String,
        new_provider: String,
        api_key_env: Option<String>,
        base_url: Option<String>,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.model.model = new_model;
            entry.manifest.model.provider = new_provider;
            entry.manifest.model.api_key_env = api_key_env;
            entry.manifest.model.base_url = base_url;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's max_tokens (response length limit).
    pub fn update_max_tokens(&self, id: AgentId, max_tokens: u32) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.model.max_tokens = max_tokens;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's sampling temperature.
    pub fn update_temperature(&self, id: AgentId, temperature: f32) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.model.temperature = temperature;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's web search augmentation mode.
    pub fn update_web_search_augmentation(
        &self,
        id: AgentId,
        mode: librefang_types::agent::WebSearchAugmentationMode,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.web_search_augmentation = mode;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's fallback model chain.
    pub fn update_fallback_models(
        &self,
        id: AgentId,
        fallback_models: Vec<librefang_types::agent::FallbackModel>,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.fallback_models = fallback_models;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's skill allowlist.
    pub fn update_skills(&self, id: AgentId, skills: Vec<String>) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.skills = skills;
            entry.manifest.skills_disabled = false;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's MCP server allowlist.
    pub fn update_mcp_servers(&self, id: AgentId, servers: Vec<String>) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.mcp_servers = servers;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's declared tools and/or allowlist/blocklist in a
    /// single registry lock. Fields left as `None` are not modified.
    pub fn update_tool_config(
        &self,
        id: AgentId,
        capabilities_tools: Option<Vec<String>>,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            if let Some(ct) = capabilities_tools {
                entry.manifest.capabilities.tools = ct;
            }
            if let Some(al) = allowlist {
                entry.manifest.tool_allowlist = al;
            }
            if let Some(bl) = blocklist {
                entry.manifest.tool_blocklist = bl;
            }
            entry.manifest.tools_disabled = false;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Rollback helper for [`Self::update_skills`]: restores both the skill
    /// allowlist AND the `skills_disabled` flag. `update_skills` always sets
    /// `skills_disabled = false` (deliberate "re-enable on update" semantics),
    /// so a rollback that only restored `skills` would silently leave the flag
    /// flipped on a failed DB persist (#3499).
    pub fn restore_skills_state(
        &self,
        id: AgentId,
        skills: Vec<String>,
        skills_disabled: bool,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.skills = skills;
            entry.manifest.skills_disabled = skills_disabled;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Rollback helper for [`Self::update_tool_config`]: restores tool fields
    /// AND the `tools_disabled` flag. `update_tool_config` always sets
    /// `tools_disabled = false`; a rollback that only restored the lists would
    /// silently leave the flag flipped on a failed DB persist (#3499).
    pub fn restore_tool_state(
        &self,
        id: AgentId,
        capabilities_tools: Vec<String>,
        allowlist: Vec<String>,
        blocklist: Vec<String>,
        tools_disabled: bool,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.capabilities.tools = capabilities_tools;
            entry.manifest.tool_allowlist = allowlist;
            entry.manifest.tool_blocklist = blocklist;
            entry.manifest.tools_disabled = tools_disabled;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's system prompt (hot-swap, takes effect on next message).
    pub fn update_system_prompt(&self, id: AgentId, new_prompt: String) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.model.system_prompt = new_prompt;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's name (also updates the name index).
    pub fn update_name(&self, id: AgentId, new_name: String) -> LibreFangResult<()> {
        // #4980 nit: reject renames into the reserved `_operator:`
        // namespace — synthetic operator-node step-result names would
        // collide with the real agent and make run history ambiguous.
        // `spawn_agent_inner` enforces the same rule at create time;
        // the rename path needs its own gate because it bypasses spawn.
        librefang_types::agent::validate_agent_name(&new_name)?;

        // Use atomic entry() API to avoid TOCTOU race between contains_key and insert.
        match self.name_index.entry(new_name.clone()) {
            Entry::Occupied(_) => {
                return Err(LibreFangError::AgentAlreadyExists(new_name));
            }
            Entry::Vacant(vacant) => {
                vacant.insert(id);
            }
        }
        let old_name = match self.with_entry_mut(id, |entry| {
            let prev = entry.name.clone();
            entry.name = new_name.clone();
            entry.manifest.name = new_name.clone();
            entry.last_active = chrono::Utc::now();
            prev
        }) {
            Ok(prev) => prev,
            Err(e) => {
                // Roll back the name index insertion if agent not found.
                self.name_index.remove(&new_name);
                return Err(e);
            }
        };
        self.name_index.remove(&old_name);
        self.notify_changed();
        Ok(())
    }

    /// Update an agent's description.
    pub fn update_description(&self, id: AgentId, new_desc: String) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.description = new_desc;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Toggle the agent's auto-dream opt-in flag. The auto-dream scheduler
    /// reads this on every tick, so the change takes effect without restart.
    /// In-memory only — persisting to the agent manifest file is a separate
    /// concern (matches the pattern of `update_system_prompt`).
    pub fn update_auto_dream_enabled(&self, id: AgentId, enabled: bool) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.manifest.auto_dream_enabled = enabled;
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Cheap read-only check for the auto-dream opt-in flag, without cloning
    /// the agent entry. Used on the hot path of the `AgentLoopEnd` hook,
    /// which fires for every turn of every agent and would otherwise pay
    /// the `AgentEntry` + manifest clone cost (several KB of Strings/Vecs
    /// per turn) just to read one bool. Missing agent → `false`, matching
    /// the "not enrolled" behaviour expected by callers.
    pub fn is_auto_dream_enabled(&self, id: AgentId) -> bool {
        self.agents
            .get(&id)
            .map(|e| e.value().manifest.auto_dream_enabled)
            .unwrap_or(false)
    }

    /// Update an agent's resource quota (budget limits).
    pub fn update_resources(
        &self,
        id: AgentId,
        hourly: Option<f64>,
        daily: Option<f64>,
        monthly: Option<f64>,
        tokens_per_hour: Option<u64>,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            if let Some(v) = hourly {
                entry.manifest.resources.max_cost_per_hour_usd = v;
            }
            if let Some(v) = daily {
                entry.manifest.resources.max_cost_per_day_usd = v;
            }
            if let Some(v) = monthly {
                entry.manifest.resources.max_cost_per_month_usd = v;
            }
            if let Some(v) = tokens_per_hour {
                entry.manifest.resources.max_llm_tokens_per_hour = Some(v);
            }
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Mark an agent's onboarding as complete.
    pub fn mark_onboarding_complete(&self, id: AgentId) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.onboarding_completed = true;
            entry.onboarding_completed_at = Some(chrono::Utc::now());
            entry.last_active = chrono::Utc::now();
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Update the session auto-reset state flags on an agent entry.
    ///
    /// Called after a policy-driven session reset or a manual reset:
    /// - `force_session_wipe` is cleared (the forced-wipe has been applied).
    /// - `resume_pending` is cleared.
    /// - `reset_reason` records why the reset happened.
    ///
    /// Does **not** update `last_active` — that is a separate concern.
    pub fn update_session_reset_state(
        &self,
        id: AgentId,
        reason: librefang_types::config::SessionResetReason,
    ) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.force_session_wipe = false;
            entry.resume_pending = false;
            entry.reset_reason = Some(reason);
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Schedule a forced session wipe so the next invocation performs a hard
    /// reset (cleared message history; session_id is preserved).
    /// Used by operator action or stuck-loop recovery.
    ///
    /// Named `schedule_session_wipe` to avoid confusion with
    /// `suspend_agent()` / `AgentState::Suspended`.
    pub fn schedule_session_wipe(&self, id: AgentId) -> LibreFangResult<()> {
        self.with_entry_mut(id, |entry| {
            entry.force_session_wipe = true;
        })?;
        self.notify_changed();
        Ok(())
    }

    /// Mark an agent's session as `resume_pending` after an interrupted
    /// restart.  Ignored when `force_session_wipe` is already set (hard-wipe wins).
    pub fn mark_resume_pending(&self, id: AgentId) -> LibreFangResult<()> {
        let mutated = self.with_entry_mut(id, |entry| {
            if !entry.force_session_wipe {
                entry.resume_pending = true;
                true
            } else {
                false
            }
        })?;
        if mutated {
            self.notify_changed();
        }
        Ok(())
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use librefang_types::agent::*;

    fn test_entry(name: &str) -> AgentEntry {
        AgentEntry {
            id: AgentId::new(),
            name: name.to_string(),
            manifest: AgentManifest {
                name: name.to_string(),
                description: "test".to_string(),
                author: "test".to_string(),
                module: "test".to_string(),
                ..Default::default()
            },
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: vec![],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        }
    }

    #[test]
    fn test_register_and_get() {
        let registry = AgentRegistry::new();
        let entry = test_entry("test-agent");
        let id = entry.id;
        registry.register(entry).unwrap();
        assert!(registry.get(id).is_some());
    }

    #[test]
    fn test_find_by_name() {
        let registry = AgentRegistry::new();
        let entry = test_entry("my-agent");
        registry.register(entry).unwrap();
        assert!(registry.find_by_name("my-agent").is_some());
    }

    #[test]
    fn test_duplicate_name() {
        let registry = AgentRegistry::new();
        registry.register(test_entry("dup")).unwrap();
        assert!(registry.register(test_entry("dup")).is_err());
    }

    #[test]
    fn test_remove() {
        let registry = AgentRegistry::new();
        let entry = test_entry("removable");
        let id = entry.id;
        registry.register(entry).unwrap();
        registry.remove(id).unwrap();
        assert!(registry.get(id).is_none());
    }

    #[test]
    fn test_update_skills_reenables_disabled_skills() {
        let registry = AgentRegistry::new();
        let mut entry = test_entry("skills-disabled");
        entry.manifest.skills_disabled = true;
        let id = entry.id;
        registry.register(entry).unwrap();

        registry
            .update_skills(id, vec!["review".to_string()])
            .expect("update should succeed");

        let updated = registry.get(id).expect("agent should exist");
        assert_eq!(updated.manifest.skills, vec!["review".to_string()]);
        assert!(
            !updated.manifest.skills_disabled,
            "updating skills should re-enable skill resolution"
        );
    }

    #[test]
    fn test_update_tool_config_reenables_disabled_tools() {
        let registry = AgentRegistry::new();
        let mut entry = test_entry("tools-disabled");
        entry.manifest.tools_disabled = true;
        let id = entry.id;
        registry.register(entry).unwrap();

        registry
            .update_tool_config(id, None, Some(vec!["file_read".to_string()]), None)
            .expect("update should succeed");

        let updated = registry.get(id).expect("agent should exist");
        assert_eq!(
            updated.manifest.tool_allowlist,
            vec!["file_read".to_string()]
        );
        assert!(
            !updated.manifest.tools_disabled,
            "updating tool filters should re-enable tool resolution"
        );
    }

    #[test]
    fn test_list_returns_deterministic_order() {
        let registry = AgentRegistry::new();
        // Insert in reverse alphabetical order
        registry.register(test_entry("zeta")).unwrap();
        registry.register(test_entry("alpha")).unwrap();
        registry.register(test_entry("mu")).unwrap();

        let names: Vec<String> = registry.list().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    /// #3569: `list_arcs()` must hand out cheap pointer clones — every Arc
    /// returned must alias the same allocation as the registry's internal
    /// slot, so `Arc::ptr_eq` against a re-fetched snapshot confirms no
    /// `AgentEntry::clone` happened on the read path.
    #[test]
    fn test_list_arcs_does_not_deep_clone() {
        let registry = AgentRegistry::new();
        registry.register(test_entry("alpha")).unwrap();
        registry.register(test_entry("beta")).unwrap();

        let snap1 = registry.list_arcs();
        let snap2 = registry.list_arcs();
        assert_eq!(snap1.len(), 2);
        assert_eq!(snap2.len(), 2);
        // Two consecutive snapshots must alias the same Arc allocations —
        // proves the registry stored an Arc and `list_arcs` only pointer-cloned.
        for (a, b) in snap1.iter().zip(snap2.iter()) {
            assert!(
                Arc::ptr_eq(a, b),
                "list_arcs deep-cloned `{}`; expected pointer-clone only",
                a.name
            );
        }
        // Strong count = 1 internal slot + 2 in snap1 + 2 in snap2 = 5 per agent.
        // The exact count is incidental; what matters is it's >1, which is only
        // possible if `list_arcs` shared the Arc instead of allocating a new one.
        for arc in &snap1 {
            assert!(
                Arc::strong_count(arc) >= 3,
                "expected Arc shared with registry + sibling snapshot, got strong_count={}",
                Arc::strong_count(arc)
            );
        }
    }

    /// #3569: mutations under `Arc::make_mut` must give snapshot semantics —
    /// a `list_arcs()` taken before a mutation must continue to observe the
    /// pre-mutation values, while a fresh `list_arcs()` after the mutation
    /// observes the new ones. This is the contract that lets dashboard
    /// handlers iterate a snapshot without locks while the kernel keeps
    /// updating agents underneath them.
    #[test]
    fn test_list_arcs_snapshot_isolation_under_mutation() {
        let registry = AgentRegistry::new();
        registry.register(test_entry("frozen")).unwrap();
        let id = registry.list_arcs()[0].id;

        // Take a pre-mutation snapshot.
        let before = registry.list_arcs();
        let before_model = before[0].manifest.model.model.clone();

        // Mutate — Arc::make_mut should fork the entry because `before`
        // still holds the old Arc.
        registry
            .update_model(id, "claude-sonnet-4-7".to_string())
            .unwrap();

        // Old snapshot is untouched.
        assert_eq!(before[0].manifest.model.model, before_model);
        assert_ne!(before_model, "claude-sonnet-4-7");
        // New snapshot reflects the mutation.
        let after = registry.list_arcs();
        assert_eq!(after[0].manifest.model.model, "claude-sonnet-4-7");
        // The two snapshots are distinct allocations now.
        assert!(
            !Arc::ptr_eq(&before[0], &after[0]),
            "make_mut should have forked the entry; snapshots aliased the same Arc"
        );
    }

    #[test]
    fn test_peer_agents_summary_is_sorted_and_projects_three_fields() {
        let registry = AgentRegistry::new();
        registry.register(test_entry("zeta")).unwrap();
        registry.register(test_entry("alpha")).unwrap();
        registry.register(test_entry("mu")).unwrap();

        let summary = registry.peer_agents_summary();
        let names: Vec<&str> = summary.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
        for (name, state_debug, model) in &summary {
            assert!(!name.is_empty());
            assert!(!state_debug.is_empty());
            assert!(!model.is_empty());
        }
    }

    #[test]
    fn test_update_temperature() {
        let registry = AgentRegistry::new();
        let entry = test_entry("temp-agent");
        let id = entry.id;
        registry.register(entry).unwrap();

        // Default temperature is 0.7
        let before = registry.get(id).unwrap();
        let old_active = before.last_active;
        assert!((before.manifest.model.temperature - 0.7).abs() < f32::EPSILON);

        // Wait a tiny bit so last_active changes
        std::thread::sleep(std::time::Duration::from_millis(1));

        registry.update_temperature(id, 1.5).unwrap();

        let after = registry.get(id).unwrap();
        assert!((after.manifest.model.temperature - 1.5).abs() < f32::EPSILON);
        assert!(after.last_active > old_active);
    }

    #[test]
    fn test_update_auto_dream_enabled_toggles_flag() {
        let registry = AgentRegistry::new();
        let entry = test_entry("dreamer");
        let id = entry.id;
        registry.register(entry).unwrap();

        // Starts false (manifest default).
        assert!(!registry.get(id).unwrap().manifest.auto_dream_enabled);

        registry.update_auto_dream_enabled(id, true).unwrap();
        assert!(registry.get(id).unwrap().manifest.auto_dream_enabled);

        registry.update_auto_dream_enabled(id, false).unwrap();
        assert!(!registry.get(id).unwrap().manifest.auto_dream_enabled);
    }

    #[test]
    fn test_is_auto_dream_enabled_tracks_flag() {
        // Lightweight bool-only accessor must agree with the clone-based
        // `get().manifest.auto_dream_enabled` path in all three states.
        let registry = AgentRegistry::new();
        let entry = test_entry("dreamer-fast");
        let id = entry.id;
        registry.register(entry).unwrap();
        assert!(!registry.is_auto_dream_enabled(id));

        registry.update_auto_dream_enabled(id, true).unwrap();
        assert!(registry.is_auto_dream_enabled(id));

        registry.update_auto_dream_enabled(id, false).unwrap();
        assert!(!registry.is_auto_dream_enabled(id));
    }

    #[test]
    fn test_is_auto_dream_enabled_missing_agent_is_false() {
        // Missing agent must return false rather than panic — the auto-dream
        // hook fires for every turn and cannot distinguish a killed agent
        // from an opted-out one at that layer.
        let registry = AgentRegistry::new();
        let bogus = AgentId::new();
        assert!(!registry.is_auto_dream_enabled(bogus));
    }

    /// Regression for #3338 / #3817: under concurrent register+remove,
    /// `find_by_name` must be atomic — it must never return `None` because
    /// publication of the entry hasn't caught up with the name binding,
    /// nor return a half-cloned/torn entry. The contract is: when a
    /// register call returns `Ok`, every subsequent `find_by_name(name)`
    /// either returns `Some(entry)` for that name or returns `None`
    /// because the agent is genuinely gone.
    ///
    /// Before the fix, `register` inserted into `name_index` before
    /// `agents`, and `remove` retracted `agents` before `name_index`. A
    /// concurrent `find_by_name` could observe the name resolved to an id
    /// while the corresponding entry was either not yet published or
    /// already gone — the entire row would silently come back as `None`.
    /// This is the spawn-before-publish gap referenced in the issue.
    #[test]
    fn find_by_name_is_atomic_under_concurrent_register_and_remove() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        // Two-phase deterministic driver. Earlier iterations:
        //   v1 (#4393): wall-clock 100ms slice + `lookups > 1_000`. Flaked
        //     on Ubuntu shard 1 under load (reader couldn't squeeze 1k
        //     iterations in time).
        //   v2 (#4673): fixed 5_000 writer cycles + `hits > 0`. Removed
        //     the wall-clock dependency but introduced a probabilistic
        //     vacuous-pass: on a fast runner that gives the writer
        //     priority (observed on macOS, #4704), the writer can drain
        //     all 5_000 register/remove cycles before the reader thread
        //     ever schedules — `hits` stays 0 and the assertion fires.
        //
        // v3 (this): split into two phases.
        //   Phase 1 — establish that the reader actually ran. Writer
        //     registers "racy" once and waits on `phase1_done` until
        //     the reader has observed the entry at least once. No race
        //     here: "racy" is permanently in the registry, so the
        //     reader's first hit is structural, not probabilistic.
        //   Phase 2 — race phase. Writer cycles register/remove; reader
        //     keeps polling. This is what actually exercises the
        //     torn-read invariant.
        // After this, `hits > 0` is guaranteed (Phase 1 sets it),
        // `torn == 0` is the real invariant under test, and the test
        // is no longer schedule-dependent.
        const WRITER_CYCLES: usize = 5_000;

        let registry = Arc::new(AgentRegistry::new());
        let stop = Arc::new(AtomicBool::new(false));
        let phase1_done = Arc::new(AtomicBool::new(false));
        // Count cases where `find_by_name` returned `Some` but the entry's
        // name disagreed with the lookup key (impossible if registry is
        // self-consistent; would catch torn reads).
        let torn = Arc::new(AtomicUsize::new(0));
        let lookups = Arc::new(AtomicUsize::new(0));
        let hits = Arc::new(AtomicUsize::new(0));

        let writer = {
            let registry = Arc::clone(&registry);
            let stop = Arc::clone(&stop);
            let phase1_done = Arc::clone(&phase1_done);
            thread::spawn(move || {
                // Phase 1: register "racy" once, wait for reader to observe.
                let entry = test_entry("racy");
                let id = entry.id;
                registry
                    .register(entry)
                    .expect("phase 1: initial register should succeed");
                while !phase1_done.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
                registry.remove(id).ok();

                // Phase 2: race cycles.
                for _ in 0..WRITER_CYCLES {
                    let entry = test_entry("racy");
                    let id = entry.id;
                    if registry.register(entry).is_ok() {
                        registry.remove(id).ok();
                    }
                }
                stop.store(true, Ordering::Release);
            })
        };

        let reader = {
            let registry = Arc::clone(&registry);
            let stop = Arc::clone(&stop);
            let phase1_done = Arc::clone(&phase1_done);
            let torn = Arc::clone(&torn);
            let lookups = Arc::clone(&lookups);
            let hits = Arc::clone(&hits);
            thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    lookups.fetch_add(1, Ordering::Relaxed);
                    if let Some(found) = registry.find_by_name("racy") {
                        hits.fetch_add(1, Ordering::Relaxed);
                        // Release-store so the writer's Acquire-load on
                        // phase1_done sees the same memory state where
                        // we incremented `hits`. Cheap to do every hit;
                        // the writer only checks during Phase 1.
                        phase1_done.store(true, Ordering::Release);
                        if found.name != "racy" {
                            torn.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
        };

        writer.join().unwrap();
        reader.join().unwrap();

        // Reader ran (vacuous on a truly broken scheduler, but if Phase 1
        // signalled `phase1_done` the reader must have observed at least
        // once — both this and the `hits > 0` assertion below should hold).
        assert!(
            lookups.load(Ordering::Relaxed) >= 1,
            "reader thread did not run a single iteration before the writer finished"
        );
        assert!(
            hits.load(Ordering::Relaxed) > 0,
            "reader never observed the agent — Phase 1 was supposed to make this \
             structural; if this fires the registry's find_by_name is broken or \
             the phase1 hand-off is wrong"
        );
        assert_eq!(
            torn.load(Ordering::Relaxed),
            0,
            "find_by_name returned an entry whose name does not match the lookup key — \
             register/remove ordering exposes a torn read across name_index and agents"
        );
    }

    #[test]
    fn test_update_auto_dream_enabled_missing_agent_errors() {
        let registry = AgentRegistry::new();
        let bogus = AgentId::new();
        let result = registry.update_auto_dream_enabled(bogus, true);
        assert!(matches!(
            result,
            Err(librefang_types::error::LibreFangError::AgentNotFound(_))
        ));
    }

    /// #3513: every successful mutation must publish a change event so
    /// dashboard WebSockets can re-snapshot without polling. Verifies the
    /// broadcast channel fires on `register` (and that the receiver wakes up).
    #[tokio::test]
    async fn change_broadcast_fires_on_register() {
        let registry = AgentRegistry::new();
        let mut rx = registry.subscribe_changes();

        registry.register(test_entry("first")).unwrap();

        // The receiver must wake up promptly; bound the wait so a missed
        // broadcast surfaces as a test failure rather than a hang.
        let recv = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
        assert!(
            matches!(recv, Ok(Ok(()))),
            "expected change event, got {recv:?}"
        );
    }

    /// #3513: a burst of mutations must all be observable by a subscriber.
    /// `Lagged` is acceptable — subscribers handle it by re-snapshotting —
    /// what is not acceptable is a silent miss with no signal at all.
    #[tokio::test]
    async fn change_broadcast_handles_burst_of_mutations() {
        let registry = AgentRegistry::new();
        let mut rx = registry.subscribe_changes();

        // Fire 5 in rapid succession.
        for i in 0..5 {
            registry
                .register(test_entry(&format!("burst-{i}")))
                .unwrap();
        }

        // Drain — we must observe at least one event (Ok or Lagged), and
        // total events seen plus any lag must cover all 5 mutations or
        // signal lag explicitly.
        let mut got_ok = 0usize;
        let mut got_lagged = false;
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(())) => got_ok += 1,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    got_lagged = true;
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Err(_) => break, // timeout — channel quiesced
            }
        }

        assert!(
            got_ok > 0 || got_lagged,
            "subscriber received no signal at all from a burst of 5 mutations"
        );
        if !got_lagged {
            assert_eq!(
                got_ok, 5,
                "expected 5 change events without lag, got {got_ok}"
            );
        }
    }

    /// #3513: subscribe_changes() must hand out independent receivers so
    /// multiple WS clients can all listen.
    #[tokio::test]
    async fn change_broadcast_supports_multiple_subscribers() {
        let registry = AgentRegistry::new();
        let mut rx_a = registry.subscribe_changes();
        let mut rx_b = registry.subscribe_changes();

        registry.register(test_entry("multi")).unwrap();

        let a = tokio::time::timeout(std::time::Duration::from_secs(1), rx_a.recv()).await;
        let b = tokio::time::timeout(std::time::Duration::from_secs(1), rx_b.recv()).await;
        assert!(matches!(a, Ok(Ok(()))), "rx_a expected event, got {a:?}");
        assert!(matches!(b, Ok(Ok(()))), "rx_b expected event, got {b:?}");
    }

    /// #3513: mutators other than register must also publish — otherwise a
    /// dashboard tab opened after agent creation would never observe model
    /// changes, state transitions, etc.
    #[tokio::test]
    async fn change_broadcast_fires_on_state_and_model_updates() {
        let registry = AgentRegistry::new();
        let entry = test_entry("mutable");
        let id = entry.id;
        registry.register(entry).unwrap();

        // Subscribe AFTER register so we observe only post-subscribe mutations.
        let mut rx = registry.subscribe_changes();

        registry
            .set_state(id, AgentState::Running)
            .expect("set_state");
        let s1 = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
        assert!(matches!(s1, Ok(Ok(()))), "set_state should fire event");

        registry
            .update_model(id, "claude-sonnet-4-7".to_string())
            .expect("update_model");
        let s2 = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
        assert!(matches!(s2, Ok(Ok(()))), "update_model should fire event");

        registry.remove(id).expect("remove");
        let s3 = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
        assert!(matches!(s3, Ok(Ok(()))), "remove should fire event");
    }
}
