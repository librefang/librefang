//! Agent registry — tracks all agents, their state, and indexes.

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use librefang_types::agent::{AgentEntry, AgentId, AgentMode, AgentState};
use librefang_types::error::{LibreFangError, LibreFangResult};

/// Registry of all agents in the kernel.
pub struct AgentRegistry {
    /// Primary index: agent ID → entry.
    agents: DashMap<AgentId, AgentEntry>,
    /// Name index: human-readable name → agent ID.
    name_index: DashMap<String, AgentId>,
    /// Tag index: tag → list of agent IDs.
    tag_index: DashMap<String, Vec<AgentId>>,
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
            tag_index: DashMap::new(),
        }
    }

    /// Register a new agent.
    pub fn register(&self, entry: AgentEntry) -> LibreFangResult<()> {
        let id = entry.id;
        // Use atomic entry() API to avoid TOCTOU race between contains_key and insert.
        match self.name_index.entry(entry.name.clone()) {
            Entry::Occupied(_) => {
                return Err(LibreFangError::AgentAlreadyExists(entry.name));
            }
            Entry::Vacant(vacant) => {
                vacant.insert(id);
            }
        }
        for tag in &entry.tags {
            self.tag_index.entry(tag.clone()).or_default().push(id);
        }
        self.agents.insert(id, entry);
        Ok(())
    }

    /// Get an agent entry by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentEntry> {
        self.agents.get(&id).map(|e| e.value().clone())
    }

    /// Find an agent by name.
    pub fn find_by_name(&self, name: &str) -> Option<AgentEntry> {
        self.name_index
            .get(name)
            .and_then(|id| self.agents.get(id.value()).map(|e| e.value().clone()))
    }

    /// Touch the agent's `last_active` timestamp without changing any other field.
    /// Used to prevent heartbeat false-positives during long-running operations.
    pub fn touch(&self, id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&id) {
            entry.last_active = chrono::Utc::now();
        }
    }

    /// Update agent state.
    pub fn set_state(&self, id: AgentId, state: AgentState) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.state = state;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update agent operational mode.
    pub fn set_mode(&self, id: AgentId, mode: AgentMode) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.mode = mode;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Remove an agent from the registry.
    pub fn remove(&self, id: AgentId) -> LibreFangResult<AgentEntry> {
        let (_, entry) = self
            .agents
            .remove(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        self.name_index.remove(&entry.name);
        for tag in &entry.tags {
            if let Some(mut ids) = self.tag_index.get_mut(tag) {
                ids.retain(|&agent_id| agent_id != id);
            }
        }
        Ok(entry)
    }

    /// List all agents, sorted by name for deterministic ordering.
    pub fn list(&self) -> Vec<AgentEntry> {
        let mut entries: Vec<AgentEntry> = self.agents.iter().map(|e| e.value().clone()).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Add a child agent ID to a parent's children list.
    pub fn add_child(&self, parent_id: AgentId, child_id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&parent_id) {
            entry.children.push(child_id);
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
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.session_id = new_session_id;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's workspace path.
    pub fn update_workspace(
        &self,
        id: AgentId,
        workspace: Option<std::path::PathBuf>,
    ) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.workspace = workspace;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's source TOML path.
    pub fn update_source_toml_path(
        &self,
        id: AgentId,
        source_toml_path: Option<std::path::PathBuf>,
    ) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.source_toml_path = source_toml_path;
        entry.last_active = chrono::Utc::now();
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
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest = manifest;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's visual identity (emoji, avatar, color).
    pub fn update_identity(
        &self,
        id: AgentId,
        identity: librefang_types::agent::AgentIdentity,
    ) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.identity = identity;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model configuration.
    pub fn update_model(&self, id: AgentId, new_model: String) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model AND provider together.
    pub fn update_model_and_provider(
        &self,
        id: AgentId,
        new_model: String,
        new_provider: String,
    ) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.manifest.model.provider = new_provider;
        entry.last_active = chrono::Utc::now();
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
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.manifest.model.provider = new_provider;
        entry.manifest.model.api_key_env = api_key_env;
        entry.manifest.model.base_url = base_url;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's max_tokens (response length limit).
    pub fn update_max_tokens(&self, id: AgentId, max_tokens: u32) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.max_tokens = max_tokens;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's sampling temperature.
    pub fn update_temperature(&self, id: AgentId, temperature: f32) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.temperature = temperature;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's fallback model chain.
    pub fn update_fallback_models(
        &self,
        id: AgentId,
        fallback_models: Vec<librefang_types::agent::FallbackModel>,
    ) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.fallback_models = fallback_models;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's skill allowlist.
    pub fn update_skills(&self, id: AgentId, skills: Vec<String>) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.skills = skills;
        entry.manifest.skills_disabled = false;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's MCP server allowlist.
    pub fn update_mcp_servers(&self, id: AgentId, servers: Vec<String>) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.mcp_servers = servers;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's tool allowlist and blocklist.
    pub fn update_tool_filters(
        &self,
        id: AgentId,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        if let Some(al) = allowlist {
            entry.manifest.tool_allowlist = al;
        }
        if let Some(bl) = blocklist {
            entry.manifest.tool_blocklist = bl;
        }
        entry.manifest.tools_disabled = false;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's system prompt (hot-swap, takes effect on next message).
    pub fn update_system_prompt(&self, id: AgentId, new_prompt: String) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.system_prompt = new_prompt;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's name (also updates the name index).
    pub fn update_name(&self, id: AgentId, new_name: String) -> LibreFangResult<()> {
        // Use atomic entry() API to avoid TOCTOU race between contains_key and insert.
        match self.name_index.entry(new_name.clone()) {
            Entry::Occupied(_) => {
                return Err(LibreFangError::AgentAlreadyExists(new_name));
            }
            Entry::Vacant(vacant) => {
                vacant.insert(id);
            }
        }
        let mut entry = self.agents.get_mut(&id).ok_or_else(|| {
            // Roll back the name index insertion if agent not found.
            self.name_index.remove(&new_name);
            LibreFangError::AgentNotFound(id.to_string())
        })?;
        let old_name = entry.name.clone();
        entry.name = new_name.clone();
        entry.manifest.name = new_name;
        entry.last_active = chrono::Utc::now();
        drop(entry);
        self.name_index.remove(&old_name);
        Ok(())
    }

    /// Update an agent's description.
    pub fn update_description(&self, id: AgentId, new_desc: String) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.description = new_desc;
        entry.last_active = chrono::Utc::now();
        Ok(())
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
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
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
            entry.manifest.resources.max_llm_tokens_per_hour = v;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Mark an agent's onboarding as complete.
    pub fn mark_onboarding_complete(&self, id: AgentId) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.onboarding_completed = true;
        entry.onboarding_completed_at = Some(chrono::Utc::now());
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// List agents belonging to a specific account (tenant).
    ///
    /// Returns only agents whose `account_id` matches the given owner string.
    /// Unowned agents (`account_id: None`) are excluded. Results are sorted
    /// by name for deterministic ordering, matching `list()`.
    pub fn list_by_account(&self, owner: &str) -> Vec<AgentEntry> {
        let mut entries: Vec<AgentEntry> = self
            .agents
            .iter()
            .filter(|e| e.value().account_id.as_deref() == Some(owner))
            .map(|e| e.value().clone())
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Get an agent by ID, scoped to a specific account (tenant).
    ///
    /// Returns `Some(entry)` only if the agent exists AND its `account_id`
    /// matches the given owner. Returns `None` for cross-tenant access or
    /// if the agent is unowned (`account_id: None`).
    pub fn get_scoped(&self, id: AgentId, owner: &str) -> Option<AgentEntry> {
        self.agents
            .get(&id)
            .filter(|e| e.value().account_id.as_deref() == Some(owner))
            .map(|e| e.value().clone())
    }

    /// Assign an account (tenant) owner to an agent.
    ///
    /// When `account_id` is `Some`, the agent is bound to that tenant for
    /// subsequent access-control checks. When `None`, the agent remains
    /// unowned (legacy / admin behaviour).
    pub fn set_account_id(&self, id: AgentId, account_id: Option<String>) -> LibreFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| LibreFangError::AgentNotFound(id.to_string()))?;
        entry.account_id = account_id;
        entry.last_active = chrono::Utc::now();
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
            account_id: None,
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
    fn test_update_tool_filters_reenables_disabled_tools() {
        let registry = AgentRegistry::new();
        let mut entry = test_entry("tools-disabled");
        entry.manifest.tools_disabled = true;
        let id = entry.id;
        registry.register(entry).unwrap();

        registry
            .update_tool_filters(id, Some(vec!["file_read".to_string()]), None)
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

    // ── Multi-tenant: list_by_account ──────────────────────────

    fn test_entry_with_account(name: &str, account_id: Option<&str>) -> AgentEntry {
        let mut entry = test_entry(name);
        entry.account_id = account_id.map(|s| s.to_string());
        entry
    }

    #[test]
    fn test_list_by_account_filters_to_owner() {
        let registry = AgentRegistry::new();
        registry
            .register(test_entry_with_account("t1-agent", Some("tenant-1")))
            .unwrap();
        registry
            .register(test_entry_with_account("t2-agent", Some("tenant-2")))
            .unwrap();
        registry
            .register(test_entry_with_account("sys-agent", None))
            .unwrap();

        let t1_agents = registry.list_by_account("tenant-1");
        assert_eq!(t1_agents.len(), 1);
        assert_eq!(t1_agents[0].name, "t1-agent");
    }

    #[test]
    fn test_list_by_account_returns_empty_for_unknown_tenant() {
        let registry = AgentRegistry::new();
        registry
            .register(test_entry_with_account("t1-agent", Some("tenant-1")))
            .unwrap();

        let results = registry.list_by_account("tenant-999");
        assert!(results.is_empty());
    }

    // ── Multi-tenant: get_scoped ────────────────────────────

    #[test]
    fn test_get_scoped_returns_owned_agent() {
        let registry = AgentRegistry::new();
        let entry = test_entry_with_account("owned", Some("tenant-1"));
        let id = entry.id;
        registry.register(entry).unwrap();

        let result = registry.get_scoped(id, "tenant-1");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "owned");
    }

    #[test]
    fn test_get_scoped_cross_tenant_returns_none() {
        let registry = AgentRegistry::new();
        let entry = test_entry_with_account("secret", Some("tenant-1"));
        let id = entry.id;
        registry.register(entry).unwrap();

        // tenant-2 must NOT see tenant-1's agent
        let result = registry.get_scoped(id, "tenant-2");
        assert!(result.is_none(), "cross-tenant access must return None");
    }

    #[test]
    fn test_get_scoped_unowned_agent_returns_none() {
        let registry = AgentRegistry::new();
        let entry = test_entry_with_account("legacy", None);
        let id = entry.id;
        registry.register(entry).unwrap();

        // Scoped request for an unowned agent → None (prevents leaking legacy data)
        let result = registry.get_scoped(id, "tenant-1");
        assert!(
            result.is_none(),
            "scoped request should not see unowned agent"
        );
    }

    // ── Multi-tenant: set_account_id ────────────────────────

    #[test]
    fn test_set_account_id_assigns_owner() {
        let registry = AgentRegistry::new();
        let entry = test_entry("unowned");
        let id = entry.id;
        registry.register(entry).unwrap();

        registry
            .set_account_id(id, Some("tenant-1".to_string()))
            .unwrap();

        let updated = registry.get(id).unwrap();
        assert_eq!(updated.account_id, Some("tenant-1".to_string()));
    }

    #[test]
    fn test_set_account_id_clears_owner() {
        let registry = AgentRegistry::new();
        let entry = test_entry_with_account("owned", Some("tenant-1"));
        let id = entry.id;
        registry.register(entry).unwrap();

        registry.set_account_id(id, None).unwrap();

        let updated = registry.get(id).unwrap();
        assert_eq!(updated.account_id, None);
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
}
