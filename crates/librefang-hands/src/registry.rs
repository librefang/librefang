//! Hand registry — manages hand definitions and active instances.

use crate::{
    HandDefinition, HandError, HandInstance, HandRequirement, HandResult, HandSettingType,
    HandStatus, RequirementType,
};
use dashmap::DashMap;
use librefang_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

/// Wrapper struct for HAND.toml files that use the documented `[hand]` section format.
#[derive(Debug, Clone, Deserialize)]
struct HandTomlWrapper {
    hand: HandDefinition,
}

/// Resolve the agents registry directory from a home directory.
/// Returns `Some(path)` if `{home_dir}/registry/agents/` exists and is a directory.
fn resolve_agents_dir(home_dir: &Path) -> Option<std::path::PathBuf> {
    let dir = home_dir.join("registry").join("agents");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// Parse a HAND.toml into a HandDefinition with its skill content attached.
///
/// Accepts both formats:
/// - Flat format: fields at top level
/// - Wrapped format (shown in docs): fields under `[hand]` section
pub fn parse_hand_toml(
    toml_content: &str,
    skill_content: &str,
    agent_skill_content: HashMap<String, String>,
) -> Result<HandDefinition, HandError> {
    parse_hand_toml_with_agents_dir(toml_content, skill_content, agent_skill_content, None)
}

/// Parse HAND.toml with optional agent template resolution.
///
/// When `agents_dir` is provided, agents with a `base` field are resolved:
/// the base agent template is loaded from `{agents_dir}/{base}/agent.toml`,
/// and the hand's inline fields are deep-merged on top (hand wins).
pub fn parse_hand_toml_with_agents_dir(
    toml_content: &str,
    skill_content: &str,
    agent_skill_content: HashMap<String, String>,
    agents_dir: Option<&std::path::Path>,
) -> Result<HandDefinition, HandError> {
    let mut def: HandDefinition = if agents_dir.is_some() {
        // Use the filesystem-aware parser so `base` template references are
        // resolved during agent construction (before AgentManifest parsing).
        crate::parse_hand_definition(toml_content, agents_dir)
            .or_else(|flat_err| {
                tracing::warn!("Flat parse failed for hand: {flat_err}");
                // Try wrapped format: fields under [hand] section.
                // Extract the [hand] sub-table and re-serialize so that
                // parse_hand_definition can resolve `base` templates with agents_dir.
                let top: toml::Value = toml::from_str(toml_content)
                    .map_err(|e| format!("Wrapped parse also failed: {e}"))?;
                let hand_value = top
                    .get("hand")
                    .ok_or_else(|| "Wrapped parse also failed: no [hand] section".to_string())?;
                let hand_toml = toml::to_string(hand_value)
                    .map_err(|e| format!("Failed to re-serialize [hand] section: {e}"))?;
                crate::parse_hand_definition(&hand_toml, agents_dir)
                    .map_err(|e| format!("Wrapped parse also failed: {e}"))
            })
            .map_err(|e: String| HandError::TomlParse(e))?
    } else {
        // No agents_dir — use standard serde path (no base resolution).
        toml::from_str::<HandDefinition>(toml_content)
            .or_else(|flat_err| {
                tracing::warn!("Flat parse failed for hand: {flat_err}");
                toml::from_str::<HandTomlWrapper>(toml_content).map(|w| w.hand)
            })
            .map_err(|e| HandError::TomlParse(e.to_string()))?
    };

    if !skill_content.is_empty() {
        def.skill_content = Some(skill_content.to_string());
    }
    if !agent_skill_content.is_empty() {
        def.agent_skill_content = agent_skill_content;
    }
    Ok(def)
}

/// Scan a directory for per-agent skill files matching `SKILL-{role}.md`.
///
/// Returns a map from lowercase role name to file content.
fn scan_agent_skill_files(dir: &Path) -> HashMap<String, String> {
    let mut skills = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_str().unwrap_or_default().to_string();
            if let Some(role) = file_name
                .strip_prefix("SKILL-")
                .and_then(|rest| rest.strip_suffix(".md"))
            {
                if !role.is_empty() {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        if !content.is_empty() {
                            skills.insert(role.to_lowercase(), content);
                        }
                    }
                }
            }
        }
    }
    skills
}

/// Scan `home_dir/registry/hands/` for subdirectories containing HAND.toml.
///
/// Returns `(hand_id, toml_content, shared_skill_content, per_agent_skill_content)`.
/// Per-agent skill files follow the pattern `SKILL-{role}.md` (e.g. `SKILL-pm.md`).
fn scan_hands_dir(home_dir: &Path) -> Vec<(String, String, String, HashMap<String, String>)> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();

    let dirs = [home_dir.join("registry").join("hands")];

    for hands_dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(hands_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let id = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if !seen.insert(id.clone()) {
                    continue;
                }
                let toml_path = path.join("HAND.toml");
                let skill_path = path.join("SKILL.md");
                if !toml_path.exists() {
                    continue;
                }
                let toml = match std::fs::read_to_string(&toml_path) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(path = %toml_path.display(), error = %e, "Failed to read HAND.toml");
                        continue;
                    }
                };
                let skill = std::fs::read_to_string(&skill_path).unwrap_or_default();
                let agent_skills = scan_agent_skill_files(&path);

                results.push((id, toml, skill, agent_skills));
            }
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Entry from persisted hand state used during daemon restart.
#[derive(Debug, Clone)]
pub struct HandStateEntry {
    pub hand_id: String,
    pub config: HashMap<String, serde_json::Value>,
    pub old_agent_ids: BTreeMap<String, AgentId>,
    pub coordinator_role: Option<String>,
    pub status: HandStatus,
    /// The original instance UUID, used to regenerate deterministic agent IDs
    /// that match the pre-restart values.
    pub instance_id: Option<Uuid>,
}

// ─── Settings availability types ────────────────────────────────────────────

/// Availability status of a single setting option.
#[derive(Debug, Clone, Serialize)]
pub struct SettingOptionStatus {
    pub value: String,
    pub label: String,
    pub provider_env: Option<String>,
    pub binary: Option<String>,
    pub available: bool,
}

/// Setting with per-option availability info (for API responses).
#[derive(Debug, Clone, Serialize)]
pub struct SettingStatus {
    pub key: String,
    pub label: String,
    pub description: String,
    pub setting_type: HandSettingType,
    pub default: String,
    pub options: Vec<SettingOptionStatus>,
}

/// The Hand registry — stores definitions and tracks active instances.
pub struct HandRegistry {
    /// All known hand definitions, keyed by hand_id.
    definitions: DashMap<String, HandDefinition>,
    /// Active hand instances, keyed by instance UUID.
    instances: DashMap<Uuid, HandInstance>,
    /// Serializes activate/deactivate to prevent race conditions where two
    /// concurrent requests both pass the "already active" check.
    activate_lock: Mutex<()>,
    /// Guards concurrent writes to hand_state.json.
    persist_lock: Mutex<()>,
}

impl HandRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            definitions: DashMap::new(),
            instances: DashMap::new(),
            activate_lock: Mutex::new(()),
            persist_lock: Mutex::new(()),
        }
    }

    /// Persist hand state to disk so it survives restarts.
    ///
    /// Persists both Active and Paused instances so their state is not lost
    /// across daemon restarts. Error-state instances are also persisted so
    /// the user can see what went wrong after a restart.
    pub fn persist_state(&self, path: &std::path::Path) -> HandResult<()> {
        let _guard = self
            .persist_lock
            .lock()
            .map_err(|e| HandError::Config(format!("persist lock poisoned: {e}")))?;
        let entries: Vec<serde_json::Value> = self
            .instances
            .iter()
            .filter(|e| !matches!(e.status, HandStatus::Inactive))
            .map(|e| {
                serde_json::json!({
                    "hand_id": e.hand_id,
                    "instance_id": e.instance_id.to_string(),
                    "config": e.config,
                    "agent_ids": e.agent_ids,
                    "coordinator_role": e.coordinator_role,
                    "status": e.status,
                })
            })
            .collect();
        let wrapper = serde_json::json!({
            "version": 3,
            "instances": entries,
        });
        let json = serde_json::to_string_pretty(&wrapper)
            .map_err(|e| HandError::Config(format!("serialize hand state: {e}")))?;
        std::fs::write(path, json)
            .map_err(|e| HandError::Config(format!("write hand state: {e}")))?;
        Ok(())
    }

    /// Load persisted hand state and re-activate hands.
    /// Returns list of (hand_id, config, old_agent_ids, status) that should be restored.
    /// The `old_agent_ids` are the agent UUIDs from before the restart, used to
    /// reassign cron jobs to newly spawned agents (issue #402).
    pub fn load_state(path: &std::path::Path) -> Vec<HandStateEntry> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };

        // Try v3/v2 format (with version field), then fall back to v1 (bare array).
        let entries: Vec<serde_json::Value> =
            if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(&data) {
                if wrapper.get("version").is_some() {
                    match wrapper.get("instances").cloned() {
                        Some(serde_json::Value::Array(arr)) => arr,
                        _ => {
                            warn!("Hand state file has no instances array");
                            return Vec::new();
                        }
                    }
                } else if let serde_json::Value::Array(arr) = wrapper {
                    // v1 format (bare array)
                    arr
                } else {
                    warn!("Hand state file has unrecognized format");
                    return Vec::new();
                }
            } else {
                warn!("Failed to parse hand state file as JSON");
                return Vec::new();
            };

        entries
            .into_iter()
            .filter_map(|e| {
                let hand_id = e["hand_id"].as_str()?.to_string();
                let status = e
                    .get("status")
                    .and_then(|v| serde_json::from_value::<HandStatus>(v.clone()).ok())
                    .unwrap_or(HandStatus::Active);

                match &status {
                    HandStatus::Active | HandStatus::Paused => {}
                    HandStatus::Error(message) => {
                        info!(
                            hand = %hand_id,
                            error = %message,
                            "Skipping errored hand from persisted state"
                        );
                        return None;
                    }
                    HandStatus::Inactive => {
                        info!(hand = %hand_id, "Skipping inactive hand from persisted state");
                        return None;
                    }
                }

                let config: HashMap<String, serde_json::Value> =
                    serde_json::from_value(e["config"].clone()).unwrap_or_default();

                // v3: agent_ids as BTreeMap<String, AgentId>
                // v2: agent_id as single AgentId → convert to {"main": id}
                // v1: agent_id as single AgentId → same conversion
                let old_agent_ids: BTreeMap<String, AgentId> =
                    if let Some(ids_val) = e.get("agent_ids") {
                        serde_json::from_value(ids_val.clone()).unwrap_or_default()
                    } else if let Some(id_val) = e.get("agent_id") {
                        if let Ok(id) = serde_json::from_value::<AgentId>(id_val.clone()) {
                            let mut map = BTreeMap::new();
                            map.insert("main".to_string(), id);
                            map
                        } else {
                            BTreeMap::new()
                        }
                    } else {
                        BTreeMap::new()
                    };
                let coordinator_role = HandInstance::normalize_coordinator_role(
                    &old_agent_ids,
                    e.get("coordinator_role").and_then(|v| v.as_str()),
                );

                let instance_id = e
                    .get("instance_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok());

                Some(HandStateEntry {
                    hand_id,
                    config,
                    old_agent_ids,
                    coordinator_role,
                    status,
                    instance_id,
                })
            })
            .collect()
    }

    /// Load hand definitions from disk. Returns (added, updated) counts.
    pub fn reload_from_disk(&self, home_dir: &std::path::Path) -> (usize, usize) {
        let fresh = scan_hands_dir(home_dir);
        let agents_dir = resolve_agents_dir(home_dir);
        let agents_dir_opt = agents_dir.as_deref();
        let mut added = 0usize;
        let mut updated = 0usize;
        for (id, toml_content, skill_content, agent_skill_content) in fresh {
            match parse_hand_toml_with_agents_dir(
                &toml_content,
                &skill_content,
                agent_skill_content,
                agents_dir_opt,
            ) {
                Ok(def) => {
                    if self.definitions.contains_key(&def.id) {
                        updated += 1;
                    } else {
                        added += 1;
                    }
                    self.definitions.insert(def.id.clone(), def);
                }
                Err(e) => {
                    warn!(hand = %id, error = %e, "Failed to parse hand during reload");
                }
            }
        }
        (added, updated)
    }

    /// Install a hand from a directory containing HAND.toml (and optional SKILL.md / SKILL-{role}.md).
    pub fn install_from_path(
        &self,
        path: &std::path::Path,
        home_dir: &std::path::Path,
    ) -> HandResult<HandDefinition> {
        let toml_path = path.join("HAND.toml");
        let skill_path = path.join("SKILL.md");

        let toml_content = std::fs::read_to_string(&toml_path).map_err(|e| {
            HandError::NotFound(format!("Cannot read {}: {e}", toml_path.display()))
        })?;
        let skill_content = std::fs::read_to_string(&skill_path).unwrap_or_default();

        let agent_skill_content = scan_agent_skill_files(path);

        let agents_dir = resolve_agents_dir(home_dir);
        let def = parse_hand_toml_with_agents_dir(
            &toml_content,
            &skill_content,
            agent_skill_content,
            agents_dir.as_deref(),
        )?;

        if self.definitions.contains_key(&def.id) {
            return Err(HandError::AlreadyActive(format!(
                "Hand '{}' already registered",
                def.id
            )));
        }

        info!(hand = %def.id, name = %def.name, path = %path.display(), "Installed hand from path");
        self.definitions.insert(def.id.clone(), def.clone());
        Ok(def)
    }

    /// Install a hand from raw TOML + skill content (for API-based installs).
    ///
    /// Hands that use `base` template references in agent entries will be
    /// **rejected** because this path has no access to the agents registry
    /// directory. Use `install_from_path` or `install_from_content_persisted`
    /// when base template resolution is needed.
    pub fn install_from_content(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> HandResult<HandDefinition> {
        // Reject hands that use `base` template references — they cannot be
        // resolved without the agents registry directory.
        if let Ok(raw) = toml::from_str::<toml::Value>(toml_content) {
            let agents_table = raw
                .get("agents")
                .or_else(|| raw.get("hand").and_then(|h| h.get("agents")));
            if let Some(toml::Value::Table(agents)) = agents_table {
                for (role, entry) in agents {
                    if entry.get("base").and_then(|v| v.as_str()).is_some() {
                        return Err(HandError::Config(format!(
                            "Agent '{role}' uses `base` template reference which cannot be \
                             resolved via content install. Use install_from_path or \
                             install_from_content_persisted instead."
                        )));
                    }
                }
            }
        }

        let def = parse_hand_toml(toml_content, skill_content, HashMap::new())?;

        if self.definitions.contains_key(&def.id) {
            return Err(HandError::AlreadyActive(format!(
                "Hand '{}' already registered",
                def.id
            )));
        }

        info!(hand = %def.id, name = %def.name, "Installed hand from content");
        self.definitions.insert(def.id.clone(), def.clone());
        Ok(def)
    }

    /// Install a hand from raw TOML + skill content and persist it under
    /// `<home_dir>/hands/<id>/`.
    pub fn install_from_content_persisted(
        &self,
        home_dir: &std::path::Path,
        toml_content: &str,
        skill_content: &str,
    ) -> HandResult<HandDefinition> {
        let agents_dir = resolve_agents_dir(home_dir);
        let def = parse_hand_toml_with_agents_dir(
            toml_content,
            skill_content,
            HashMap::new(),
            agents_dir.as_deref(),
        )?;

        if self.definitions.contains_key(&def.id) {
            return Err(HandError::AlreadyActive(format!(
                "Hand '{}' already registered",
                def.id
            )));
        }

        let hand_dir = home_dir.join("workspaces").join(&def.id);
        std::fs::create_dir_all(&hand_dir)?;
        std::fs::write(hand_dir.join("HAND.toml"), toml_content)?;
        if !skill_content.is_empty() {
            std::fs::write(hand_dir.join("SKILL.md"), skill_content)?;
        }

        info!(
            hand = %def.id,
            name = %def.name,
            path = %hand_dir.display(),
            "Installed hand from content"
        );
        self.definitions.insert(def.id.clone(), def.clone());
        Ok(def)
    }

    /// List all known hand definitions.
    pub fn list_definitions(&self) -> Vec<HandDefinition> {
        let mut defs: Vec<HandDefinition> =
            self.definitions.iter().map(|r| r.value().clone()).collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Get a specific hand definition by ID.
    pub fn get_definition(&self, hand_id: &str) -> Option<HandDefinition> {
        self.definitions.get(hand_id).map(|r| r.value().clone())
    }

    /// Activate a hand — creates an instance (agent spawning is done by kernel).
    ///
    /// Uses a mutex to serialize the check-then-insert so two concurrent
    /// requests cannot both pass the "already active" check.
    ///
    /// If `instance_id` is `Some`, the instance is created with that UUID
    /// (used to restore a persisted instance across daemon restarts so that
    /// deterministic agent IDs remain stable).
    pub fn activate(
        &self,
        hand_id: &str,
        config: HashMap<String, serde_json::Value>,
    ) -> HandResult<HandInstance> {
        self.activate_with_id(hand_id, config, None)
    }

    /// Like [`activate`](Self::activate) but allows specifying an existing instance UUID.
    pub fn activate_with_id(
        &self,
        hand_id: &str,
        config: HashMap<String, serde_json::Value>,
        instance_id: Option<Uuid>,
    ) -> HandResult<HandInstance> {
        if !self.definitions.contains_key(hand_id) {
            return Err(HandError::NotFound(hand_id.to_string()));
        }

        // Hold the lock for the duration of check + insert to prevent races.
        let _guard = self.activate_lock.lock().unwrap_or_else(|e| e.into_inner());

        // Check if already active — only block when instance_id is None
        // (single-instance mode). When Some(uuid) is passed, it's an explicit
        // multi-instance request (e.g. daemon restart recovery) and should be
        // allowed through.
        if instance_id.is_none() {
            for entry in self.instances.iter() {
                if entry.hand_id == hand_id && entry.status == HandStatus::Active {
                    return Err(HandError::AlreadyActive(hand_id.to_string()));
                }
            }
        }

        let instance = HandInstance::new(hand_id, config, instance_id);
        let id = instance.instance_id;
        self.instances.insert(id, instance.clone());
        info!(hand = %hand_id, instance = %id, "Hand activated");
        Ok(instance)
    }

    /// Deactivate a hand instance (agent killing is done by kernel).
    pub fn deactivate(&self, instance_id: Uuid) -> HandResult<HandInstance> {
        let (_, instance) = self
            .instances
            .remove(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        info!(hand = %instance.hand_id, instance = %instance_id, "Hand deactivated");
        Ok(instance)
    }

    /// Pause a hand instance.
    pub fn pause(&self, instance_id: Uuid) -> HandResult<()> {
        let mut entry = self
            .instances
            .get_mut(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        entry.status = HandStatus::Paused;
        entry.updated_at = chrono::Utc::now();
        Ok(())
    }

    /// Resume a paused hand instance.
    pub fn resume(&self, instance_id: Uuid) -> HandResult<()> {
        let mut entry = self
            .instances
            .get_mut(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        entry.status = HandStatus::Active;
        entry.updated_at = chrono::Utc::now();
        Ok(())
    }

    /// Set all agent IDs for an instance (called after kernel spawns agents).
    pub fn set_agents(
        &self,
        instance_id: Uuid,
        agent_ids: BTreeMap<String, AgentId>,
        coordinator_role: Option<String>,
    ) -> HandResult<()> {
        let mut entry = self
            .instances
            .get_mut(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        entry.coordinator_role =
            HandInstance::normalize_coordinator_role(&agent_ids, coordinator_role.as_deref());
        entry.agent_ids = agent_ids;
        entry.updated_at = chrono::Utc::now();
        Ok(())
    }

    /// Backward-compatible: set a single agent ID under the "main" role.
    pub fn set_agent(&self, instance_id: Uuid, agent_id: AgentId) -> HandResult<()> {
        let mut map = BTreeMap::new();
        map.insert("main".to_string(), agent_id);
        self.set_agents(instance_id, map, Some("main".to_string()))
    }

    /// Find the hand instance associated with an agent (checks all roles).
    pub fn find_by_agent(&self, agent_id: AgentId) -> Option<HandInstance> {
        for entry in self.instances.iter() {
            if entry.agent_ids.values().any(|&id| id == agent_id) {
                return Some(entry.clone());
            }
        }
        None
    }

    /// List all active hand instances.
    pub fn list_instances(&self) -> Vec<HandInstance> {
        self.instances.iter().map(|e| e.clone()).collect()
    }

    /// Get a specific instance by ID.
    pub fn get_instance(&self, instance_id: Uuid) -> Option<HandInstance> {
        self.instances.get(&instance_id).map(|e| e.clone())
    }

    /// Check which requirements are satisfied for a given hand.
    pub fn check_requirements(&self, hand_id: &str) -> HandResult<Vec<(HandRequirement, bool)>> {
        let def = self
            .definitions
            .get(hand_id)
            .ok_or_else(|| HandError::NotFound(hand_id.to_string()))?;

        let results: Vec<(HandRequirement, bool)> = def
            .requires
            .iter()
            .map(|req| {
                let satisfied = check_requirement(req);
                (req.clone(), satisfied)
            })
            .collect();

        Ok(results)
    }

    /// Check availability of all settings options for a hand.
    pub fn check_settings_availability(
        &self,
        hand_id: &str,
        lang: Option<&str>,
    ) -> HandResult<Vec<SettingStatus>> {
        let def = self
            .definitions
            .get(hand_id)
            .ok_or_else(|| HandError::NotFound(hand_id.to_string()))?;

        let i18n_settings = lang
            .and_then(|l| def.i18n.get(l))
            .map(|entry| &entry.settings);

        Ok(def
            .settings
            .iter()
            .map(|setting| {
                let options = setting
                    .options
                    .iter()
                    .map(|opt| {
                        let available = check_option_available(
                            opt.provider_env.as_deref(),
                            opt.binary.as_deref(),
                        );
                        SettingOptionStatus {
                            value: opt.value.clone(),
                            label: opt.label.clone(),
                            provider_env: opt.provider_env.clone(),
                            binary: opt.binary.clone(),
                            available,
                        }
                    })
                    .collect();

                let setting_i18n = i18n_settings.and_then(|s| s.get(&setting.key));
                let label = setting_i18n
                    .and_then(|si| si.label.as_deref())
                    .unwrap_or(&setting.label)
                    .to_string();
                let description = setting_i18n
                    .and_then(|si| si.description.as_deref())
                    .unwrap_or(&setting.description)
                    .to_string();

                SettingStatus {
                    key: setting.key.clone(),
                    label,
                    description,
                    setting_type: setting.setting_type.clone(),
                    default: setting.default.clone(),
                    options,
                }
            })
            .collect())
    }

    /// Update config for an active hand instance.
    pub fn update_config(
        &self,
        instance_id: Uuid,
        config: HashMap<String, serde_json::Value>,
    ) -> HandResult<()> {
        let mut entry = self
            .instances
            .get_mut(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        entry.config = config;
        entry.updated_at = chrono::Utc::now();
        Ok(())
    }

    /// Mark an instance as errored.
    pub fn set_error(&self, instance_id: Uuid, message: String) -> HandResult<()> {
        let mut entry = self
            .instances
            .get_mut(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        entry.status = HandStatus::Error(message);
        entry.updated_at = chrono::Utc::now();
        Ok(())
    }

    /// Compute readiness for a hand, cross-referencing requirements with
    /// active instance state.
    ///
    /// `requirements_met` only considers non-optional requirements.
    /// `degraded` is true when the hand is active but any requirement
    /// (optional or not) is unmet.
    ///
    /// Returns `None` if the hand definition does not exist.
    pub fn readiness(&self, hand_id: &str) -> Option<HandReadiness> {
        let reqs = self.check_requirements(hand_id).ok()?;

        // Only mandatory (non-optional) requirements gate activation readiness.
        let requirements_met = reqs
            .iter()
            .filter(|(req, _)| !req.optional)
            .all(|(_, ok)| *ok);

        // A hand is active if at least one instance is in Active status.
        let active = self
            .instances
            .iter()
            .any(|entry| entry.hand_id == hand_id && entry.status == HandStatus::Active);

        // Degraded: active, but any requirement (including optional) is unmet.
        let degraded = active && reqs.iter().any(|(_, ok)| !ok);

        Some(HandReadiness {
            requirements_met,
            active,
            degraded,
        })
    }
}

/// Readiness snapshot for a hand definition — combines requirement checks
/// with runtime activation state so the API can report unambiguous status.
#[derive(Debug, Clone, Serialize)]
pub struct HandReadiness {
    /// Whether all declared requirements are currently satisfied.
    pub requirements_met: bool,
    /// Whether the hand currently has a running (Active-status) instance.
    pub active: bool,
    /// Whether the hand is active but some requirements are unmet.
    /// This means the hand is running in a degraded mode — some features
    /// may not work (e.g. browser hand without chromium).
    pub degraded: bool,
}

impl Default for HandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a single requirement is satisfied.
fn check_requirement(req: &HandRequirement) -> bool {
    match req.requirement_type {
        RequirementType::Binary => {
            // Special handling for python3: must actually run the command and verify
            // the output contains "Python 3", because Windows ships a python3.exe
            // Store shim that exists on PATH but doesn't actually work.
            if req.check_value == "python3" {
                return check_python3_available();
            }
            // Check if binary exists on PATH.
            if which_binary(&req.check_value) {
                return true;
            }
            if req.check_value == "chromium" {
                // Try common Chromium/Chrome binary names across platforms
                return which_binary("chromium-browser")
                    || which_binary("google-chrome")
                    || which_binary("google-chrome-stable")
                    || which_binary("chrome")
                    || std::env::var("CHROME_PATH")
                        .map(|v| !v.is_empty())
                        .unwrap_or(false);
            }
            false
        }
        RequirementType::EnvVar | RequirementType::ApiKey => {
            // Check if env var is set and non-empty
            std::env::var(&req.check_value)
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        }
        RequirementType::AnyEnvVar => {
            // check_value is comma-separated list of env var names; any one being set is enough
            req.check_value
                .split(',')
                .map(str::trim)
                .any(|var| std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false))
        }
    }
}

/// Check if Python 3 is actually available by running the command and checking
/// the version output. This avoids false negatives from Windows Store shims
/// (python3.exe that just opens the Microsoft Store) and false positives from
/// Python 2 installations where `python` exists but is Python 2.
fn check_python3_available() -> bool {
    // Try "python3 --version" first (Linux/macOS, some Windows installs)
    if run_returns_python3("python3") {
        return true;
    }
    // Try "python --version" (Windows commonly uses this, Docker containers too)
    if run_returns_python3("python") {
        return true;
    }
    // Fallback: try well-known absolute paths (handles cases where PATH is
    // minimal, e.g. inside Docker containers or cron jobs on Linux).
    for path in &["/usr/bin/python3", "/usr/local/bin/python3"] {
        if run_returns_python3(path) {
            return true;
        }
    }
    false
}

/// Run `{cmd} --version` and return true if the output contains "Python 3".
fn run_returns_python3(cmd: &str) -> bool {
    match std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return false;
            }
            // Python --version may print to stdout or stderr depending on version
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            stdout.contains("Python 3") || stderr.contains("Python 3")
        }
        Err(_) => false,
    }
}

/// Check if a binary is on PATH (cross-platform).
fn which_binary(name: &str) -> bool {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let separator = if cfg!(windows) { ';' } else { ':' };
    let extensions: Vec<&str> = if cfg!(windows) {
        vec!["", ".exe", ".cmd", ".bat"]
    } else {
        vec![""]
    };

    for dir in path_var.split(separator) {
        for ext in &extensions {
            let candidate = std::path::Path::new(dir).join(format!("{name}{ext}"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

/// Check if a setting option is available based on its provider_env and binary.
///
/// - No provider_env and no binary → always available (e.g. "auto", "none")
/// - provider_env set → check if env var is non-empty (special case: GEMINI_API_KEY also checks GOOGLE_API_KEY)
/// - binary set → check if binary is on PATH
fn check_option_available(provider_env: Option<&str>, binary: Option<&str>) -> bool {
    let env_ok = match provider_env {
        None => true,
        Some(env) => {
            let direct = std::env::var(env).map(|v| !v.is_empty()).unwrap_or(false);
            if direct {
                return binary.map(which_binary).unwrap_or(true);
            }
            // Gemini special case: also accept GOOGLE_API_KEY
            if env == "GEMINI_API_KEY" {
                std::env::var("GOOGLE_API_KEY")
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
            } else {
                false
            }
        }
    };

    if !env_ok {
        return false;
    }

    binary.map(which_binary).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure the test home dir has synced registry content.
    /// resolve_home_dir_for_tests() handles sync internally via OnceLock.
    fn ensure_test_home() -> std::path::PathBuf {
        librefang_runtime::registry_sync::resolve_home_dir_for_tests()
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = HandRegistry::new();
        assert!(reg.list_definitions().is_empty());
        assert!(reg.list_instances().is_empty());
    }

    #[test]
    fn load_hands_from_disk() {
        let reg = HandRegistry::new();
        let home = ensure_test_home();
        let (count, _) = reg.reload_from_disk(&home);
        assert_eq!(count, 15);
        assert!(!reg.list_definitions().is_empty());

        // Clip hand should be loaded
        let clip = reg.get_definition("clip");
        assert!(clip.is_some());
        let clip = clip.unwrap();
        assert_eq!(clip.name, "Clip Hand");

        // Einstein hands should be loaded
        assert!(reg.get_definition("lead").is_some());
        assert!(reg.get_definition("collector").is_some());
        assert!(reg.get_definition("predictor").is_some());
        assert!(reg.get_definition("researcher").is_some());
        assert!(reg.get_definition("twitter").is_some());

        // Browser hand should be loaded
        assert!(reg.get_definition("browser").is_some());
    }

    #[test]
    fn install_from_content_persists_hand_files() {
        let reg = HandRegistry::new();
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
id = "uptime-watcher"
name = "Uptime Watcher"
description = "Watches uptime."
category = "data"

[routing]
aliases = ["uptime watcher"]

[agent]
name = "uptime-watcher-agent"
description = "Test hand agent"
system_prompt = "Test prompt"
"#;
        let skill_content = "# Test skill\n";

        let def = reg
            .install_from_content_persisted(tmp.path(), toml_content, skill_content)
            .unwrap();

        assert_eq!(def.id, "uptime-watcher");
        assert!(tmp
            .path()
            .join("workspaces/uptime-watcher/HAND.toml")
            .exists());
        assert!(tmp
            .path()
            .join("workspaces/uptime-watcher/SKILL.md")
            .exists());
        assert!(reg.get_definition("uptime-watcher").is_some());
    }

    #[test]
    fn activate_and_deactivate() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        assert_eq!(instance.hand_id, "clip");
        assert_eq!(instance.status, HandStatus::Active);

        let instances = reg.list_instances();
        assert_eq!(instances.len(), 1);

        // Can't activate again while active
        let err = reg.activate("clip", HashMap::new());
        assert!(err.is_err());

        // Deactivate
        let removed = reg.deactivate(instance.instance_id).unwrap();
        assert_eq!(removed.hand_id, "clip");
        assert!(reg.list_instances().is_empty());
    }

    #[test]
    fn pause_and_resume() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        let id = instance.instance_id;

        reg.pause(id).unwrap();
        let paused = reg.get_instance(id).unwrap();
        assert_eq!(paused.status, HandStatus::Paused);

        reg.resume(id).unwrap();
        let resumed = reg.get_instance(id).unwrap();
        assert_eq!(resumed.status, HandStatus::Active);

        reg.deactivate(id).unwrap();
    }

    #[test]
    fn load_state_preserves_paused_instances() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        reg.pause(instance.instance_id).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("hand_state.json");
        reg.persist_state(&state_path).unwrap();

        let saved = HandRegistry::load_state(&state_path);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].hand_id, "clip");
        assert!(matches!(saved[0].status, HandStatus::Paused));
    }

    #[test]
    fn set_agent() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        let id = instance.instance_id;
        let agent_id = AgentId::new();

        reg.set_agent(id, agent_id).unwrap();

        let found = reg.find_by_agent(agent_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().instance_id, id);

        reg.deactivate(id).unwrap();
    }

    #[test]
    fn persist_and_load_explicit_coordinator_role() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        let id = instance.instance_id;
        let mut agent_ids = BTreeMap::new();
        agent_ids.insert("analyst".to_string(), AgentId::new());
        agent_ids.insert("planner".to_string(), AgentId::new());
        reg.set_agents(id, agent_ids, Some("planner".to_string()))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("hand_state.json");
        reg.persist_state(&state_path).unwrap();

        let saved = HandRegistry::load_state(&state_path);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].coordinator_role.as_deref(), Some("planner"));
    }

    #[test]
    fn check_requirements() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let results = reg.check_requirements("clip").unwrap();
        assert!(!results.is_empty());
        // Each result has a requirement and a bool
        for (req, _satisfied) in &results {
            assert!(!req.key.is_empty());
            assert!(!req.label.is_empty());
        }
    }

    #[test]
    fn not_found_errors() {
        let reg = HandRegistry::new();
        assert!(reg.get_definition("nonexistent").is_none());
        assert!(reg.activate("nonexistent", HashMap::new()).is_err());
        assert!(reg.check_requirements("nonexistent").is_err());
        assert!(reg.deactivate(Uuid::new_v4()).is_err());
        assert!(reg.pause(Uuid::new_v4()).is_err());
        assert!(reg.resume(Uuid::new_v4()).is_err());
    }

    #[test]
    fn set_error_status() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        let id = instance.instance_id;

        reg.set_error(id, "something broke".to_string()).unwrap();
        let inst = reg.get_instance(id).unwrap();
        assert_eq!(
            inst.status,
            HandStatus::Error("something broke".to_string())
        );

        reg.deactivate(id).unwrap();
    }

    #[test]
    fn which_binary_finds_common() {
        // On all platforms, at least one of these should exist
        let has_something =
            which_binary("echo") || which_binary("cmd") || which_binary("sh") || which_binary("ls");
        // This test is best-effort — in CI containers some might not exist
        let _ = has_something;
    }

    #[test]
    fn env_var_requirement_check() {
        std::env::set_var("LIBREFANG_TEST_HAND_REQ", "test_value");
        let req = HandRequirement {
            key: "test".to_string(),
            label: "test".to_string(),
            requirement_type: RequirementType::EnvVar,
            check_value: "LIBREFANG_TEST_HAND_REQ".to_string(),
            description: None,
            optional: false,
            install: None,
        };
        assert!(check_requirement(&req));

        let req_missing = HandRequirement {
            key: "test".to_string(),
            label: "test".to_string(),
            requirement_type: RequirementType::EnvVar,
            check_value: "LIBREFANG_NONEXISTENT_VAR_12345".to_string(),
            description: None,
            optional: false,
            install: None,
        };
        assert!(!check_requirement(&req_missing));
        std::env::remove_var("LIBREFANG_TEST_HAND_REQ");
    }

    #[test]
    fn readiness_nonexistent_hand() {
        let reg = HandRegistry::new();
        assert!(reg.readiness("nonexistent").is_none());
    }

    #[test]
    fn readiness_inactive_hand() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        // Lead hand has no requirements, so requirements_met = true
        let r = reg.readiness("lead").unwrap();
        assert!(r.requirements_met);
        assert!(!r.active);
        assert!(!r.degraded);
    }

    #[test]
    fn readiness_active_hand_all_met() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        // Lead hand has no requirements — activate it
        let instance = reg.activate("lead", HashMap::new()).unwrap();
        let r = reg.readiness("lead").unwrap();
        assert!(r.requirements_met);
        assert!(r.active);
        assert!(!r.degraded); // all met, so not degraded

        reg.deactivate(instance.instance_id).unwrap();
    }

    #[test]
    fn readiness_active_hand_degraded() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        // Browser hand requires python3 (mandatory) + chromium (optional).
        // Activate it — requirements_met only considers mandatory ones,
        // while degraded considers any unmet requirement (including optional).
        let instance = reg.activate("browser", HashMap::new()).unwrap();
        let r = reg.readiness("browser").unwrap();
        assert!(r.active);

        // Check all requirements to see if any are unmet
        let reqs = reg.check_requirements("browser").unwrap();
        let any_unmet = reqs.iter().any(|(_, ok)| !ok);
        let any_mandatory_unmet = reqs.iter().any(|(req, ok)| !ok && !req.optional);

        // requirements_met should be false only if a mandatory requirement is unmet
        assert_eq!(!r.requirements_met, any_mandatory_unmet);
        // degraded should be true if active and ANY requirement (including optional) is unmet
        assert_eq!(r.degraded, any_unmet);

        reg.deactivate(instance.instance_id).unwrap();
    }

    #[test]
    fn readiness_paused_hand_not_active() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("lead", HashMap::new()).unwrap();
        reg.pause(instance.instance_id).unwrap();

        let r = reg.readiness("lead").unwrap();
        assert!(!r.active); // Paused is not Active
        assert!(!r.degraded);

        reg.deactivate(instance.instance_id).unwrap();
    }

    #[test]
    fn load_state_preserves_paused_status() {
        let path = std::env::temp_dir().join(format!("hand-state-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 2,
                "instances": [{
                    "hand_id": "lead",
                    "config": {},
                    "agent_id": serde_json::Value::Null,
                    "status": "Paused",
                }],
            })
            .to_string(),
        )
        .unwrap();

        let restored = HandRegistry::load_state(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].hand_id, "lead");
        assert!(matches!(restored[0].status, HandStatus::Paused));
    }

    #[test]
    fn optional_field_defaults_false() {
        let req = HandRequirement {
            key: "test".to_string(),
            label: "test".to_string(),
            requirement_type: RequirementType::Binary,
            check_value: "test".to_string(),
            description: None,
            optional: false,
            install: None,
        };
        assert!(!req.optional);
    }

    #[test]
    fn parse_hand_toml_with_agent_skill_content() {
        let toml_content = r#"
id = "multi-test"
name = "Multi Test"
description = "Test per-agent skills"
category = "development"

[agents.pm]
name = "pm-agent"
description = "PM agent"
system_prompt = "You are a PM."

[agents.qa]
name = "qa-agent"
description = "QA agent"
system_prompt = "You are a QA."
"#;
        let shared_skill = "Shared knowledge";
        let mut agent_skills = HashMap::new();
        agent_skills.insert("pm".to_string(), "PM-specific knowledge".to_string());

        let def = parse_hand_toml(toml_content, shared_skill, agent_skills).unwrap();

        // Shared skill is set
        assert_eq!(def.skill_content.as_deref(), Some("Shared knowledge"));
        // Per-agent skill for PM
        assert_eq!(
            def.agent_skill_content.get("pm").map(|s| s.as_str()),
            Some("PM-specific knowledge")
        );
        // QA has no per-agent skill — should fall back to shared
        assert!(!def.agent_skill_content.contains_key("qa"));
    }

    #[test]
    fn parse_hand_toml_empty_agent_skills_is_backward_compatible() {
        let toml_content = r#"
id = "compat-test"
name = "Compat Test"
description = "Backward compat test"
category = "content"

[agent]
name = "main-agent"
description = "Main agent"
system_prompt = "You are helpful."
"#;
        let def = parse_hand_toml(toml_content, "Shared skill", HashMap::new()).unwrap();

        assert_eq!(def.skill_content.as_deref(), Some("Shared skill"));
        assert!(def.agent_skill_content.is_empty());
    }

    #[test]
    fn scan_hands_dir_picks_up_per_agent_skill_files() {
        let tmp = tempfile::tempdir().unwrap();
        let hand_dir = tmp.path().join("registry").join("hands").join("test-hand");
        std::fs::create_dir_all(&hand_dir).unwrap();

        // HAND.toml
        std::fs::write(
            hand_dir.join("HAND.toml"),
            r#"
id = "test-hand"
name = "Test Hand"
description = "Test"
category = "development"

[agents.dev]
name = "dev-agent"
description = "Dev"
system_prompt = "Dev prompt"

[agents.review]
name = "review-agent"
description = "Review"
system_prompt = "Review prompt"
"#,
        )
        .unwrap();

        // Shared skill
        std::fs::write(hand_dir.join("SKILL.md"), "Shared skill content").unwrap();
        // Per-agent skill for dev role
        std::fs::write(hand_dir.join("SKILL-dev.md"), "Dev-specific skill").unwrap();
        // Per-agent skill for review role
        std::fs::write(hand_dir.join("SKILL-review.md"), "Review-specific skill").unwrap();

        let results = scan_hands_dir(tmp.path());
        assert_eq!(results.len(), 1);
        let (id, _toml, skill, agent_skills) = &results[0];
        assert_eq!(id, "test-hand");
        assert_eq!(skill, "Shared skill content");
        assert_eq!(
            agent_skills.get("dev").map(|s| s.as_str()),
            Some("Dev-specific skill")
        );
        assert_eq!(
            agent_skills.get("review").map(|s| s.as_str()),
            Some("Review-specific skill")
        );
    }

    #[test]
    fn scan_hands_dir_no_agent_skills_backward_compat() {
        let tmp = tempfile::tempdir().unwrap();
        let hand_dir = tmp
            .path()
            .join("registry")
            .join("hands")
            .join("simple-hand");
        std::fs::create_dir_all(&hand_dir).unwrap();

        std::fs::write(
            hand_dir.join("HAND.toml"),
            r#"
id = "simple-hand"
name = "Simple Hand"
description = "Simple"
category = "content"

[agent]
name = "main-agent"
description = "Main"
system_prompt = "Prompt"
"#,
        )
        .unwrap();
        std::fs::write(hand_dir.join("SKILL.md"), "Shared only").unwrap();

        let results = scan_hands_dir(tmp.path());
        assert_eq!(results.len(), 1);
        let (_id, _toml, skill, agent_skills) = &results[0];
        assert_eq!(skill, "Shared only");
        assert!(agent_skills.is_empty());
    }

    #[test]
    fn scan_hands_dir_ignores_empty_agent_skill_files() {
        let tmp = tempfile::tempdir().unwrap();
        let hand_dir = tmp
            .path()
            .join("registry")
            .join("hands")
            .join("empty-skill");
        std::fs::create_dir_all(&hand_dir).unwrap();

        std::fs::write(
            hand_dir.join("HAND.toml"),
            r#"
id = "empty-skill"
name = "Empty Skill"
description = "Test"
category = "content"

[agent]
name = "main-agent"
description = "Main"
system_prompt = "Prompt"
"#,
        )
        .unwrap();
        // Empty per-agent skill file should be ignored
        std::fs::write(hand_dir.join("SKILL-dev.md"), "").unwrap();

        let results = scan_hands_dir(tmp.path());
        assert_eq!(results.len(), 1);
        let (_id, _toml, _skill, agent_skills) = &results[0];
        assert!(agent_skills.is_empty());
    }

    #[test]
    fn scan_hands_dir_lowercases_role_names() {
        let tmp = tempfile::tempdir().unwrap();
        let hand_dir = tmp.path().join("registry").join("hands").join("case-test");
        std::fs::create_dir_all(&hand_dir).unwrap();

        std::fs::write(
            hand_dir.join("HAND.toml"),
            r#"
id = "case-test"
name = "Case Test"
description = "Test"
category = "content"

[agent]
name = "main-agent"
description = "Main"
system_prompt = "Prompt"
"#,
        )
        .unwrap();
        // Mixed-case role name should be lowercased
        std::fs::write(hand_dir.join("SKILL-PM.md"), "PM skill content").unwrap();

        let results = scan_hands_dir(tmp.path());
        assert_eq!(results.len(), 1);
        let (_id, _toml, _skill, agent_skills) = &results[0];
        assert_eq!(
            agent_skills.get("pm").map(|s| s.as_str()),
            Some("PM skill content")
        );
        // Original case should NOT exist
        assert!(agent_skills.get("PM").is_none());
    }

    #[test]
    fn install_from_content_rejects_base_template() {
        let registry = HandRegistry::new();
        let toml_content = r#"
id = "base-ref-hand"
name = "Base Ref Hand"
description = "Uses base template"
category = "development"
tools = []

[agents.main]
coordinator = true
base = "some-template"

[dashboard]
metrics = []
"#;
        let result = registry.install_from_content(toml_content, "");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("base"),
            "Error should mention base template: {err}"
        );
    }

    #[test]
    fn install_from_content_accepts_hand_without_base() {
        let registry = HandRegistry::new();
        let toml_content = r#"
id = "no-base-hand"
name = "No Base Hand"
description = "No base used"
category = "content"
tools = []

[agents.main]
coordinator = true
name = "main-agent"
description = "A plain agent"
system_prompt = "Hello"

[dashboard]
metrics = []
"#;
        let result = registry.install_from_content(toml_content, "");
        assert!(result.is_ok());
    }
}
