//! Hand registry — manages hand definitions and active instances.

use crate::{
    HandDefinition, HandError, HandInstance, HandRequirement, HandResult, HandSettingType,
    HandStatus, RequirementType,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use librefang_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

/// Current version of the persisted hand state format.
const PERSIST_VERSION: u32 = 4;

/// Typed representation of persisted hand state.
#[derive(Serialize, Deserialize)]
struct PersistedState {
    version: u32,
    instances: Vec<PersistedInstance>,
}

/// Typed representation of a single persisted hand instance.
#[derive(Serialize, Deserialize)]
struct PersistedInstance {
    hand_id: String,
    instance_id: Uuid,
    config: HashMap<String, serde_json::Value>,
    agent_ids: BTreeMap<String, AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coordinator_role: Option<String>,
    status: HandStatus,
    /// When the hand was originally activated. `None` for legacy v1/v2/v3 state files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activated_at: Option<DateTime<Utc>>,
    /// Last status change before persist. `None` for legacy v1/v2/v3 state files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<DateTime<Utc>>,
}

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

/// Scan for subdirectories containing HAND.toml across both the read-only
/// registry (`home_dir/registry/hands/`) and the user-writable workspaces
/// directory (`home_dir/workspaces/`), where `install_from_content_persisted`
/// writes locally-installed hands.
///
/// Both locations are scanned because registry hands come from the shared
/// librefang-registry tarball (reset on every sync) while workspaces hands
/// come from the dashboard "install from content" flow and must survive
/// daemon restarts. Registry entries take precedence when an id collides
/// (the `seen` set drops duplicates after the first hit).
///
/// Subdirectories of `workspaces/` that are not hands (e.g. agent workspace
/// directories) are naturally filtered out by the `HAND.toml` existence check.
///
/// Returns `(hand_id, toml_content, shared_skill_content, per_agent_skill_content)`.
/// Per-agent skill files follow the pattern `SKILL-{role}.md` (e.g. `SKILL-pm.md`).
fn scan_hands_dir(home_dir: &Path) -> Vec<(String, String, String, HashMap<String, String>)> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();

    let dirs = [
        home_dir.join("registry").join("hands"),
        home_dir.join("workspaces"),
    ];

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
    /// When the hand was originally activated. `None` for legacy v1/v2/v3 state files.
    pub activated_at: Option<DateTime<Utc>>,
    /// Last status change before persist. `None` for legacy v1/v2/v3 state files.
    pub updated_at: Option<DateTime<Utc>>,
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
#[derive(Default)]
pub struct HandRegistry {
    /// All known hand definitions, keyed by hand_id.
    definitions: DashMap<String, HandDefinition>,
    /// Active hand instances, keyed by instance UUID.
    instances: DashMap<Uuid, HandInstance>,
    /// Reverse index: agent_id → instance_id for O(1) agent lookup.
    agent_index: DashMap<String, Uuid>,
    /// Reverse index: hand_id → active instance_id for O(1) active-instance check.
    active_index: DashMap<String, Uuid>,
    /// Serializes activate/deactivate to prevent race conditions where two
    /// concurrent requests both pass the "already active" check.
    activate_lock: Mutex<()>,
    /// Guards concurrent writes to hand_state.json.
    persist_lock: Mutex<()>,
}

// Static assertion that HandRegistry is Send + Sync (all fields are lock-free
// DashMaps or Mutexes).
const _: () = {
    #[allow(dead_code)]
    fn assert_send_sync<T: Send + Sync>() {}
    fn _check() {
        assert_send_sync::<HandRegistry>()
    }
};

impl HandRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            definitions: DashMap::new(),
            instances: DashMap::new(),
            agent_index: DashMap::new(),
            active_index: DashMap::new(),
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
        let _guard = self.persist_lock.lock().unwrap_or_else(|e| {
            warn!("persist_state: persist_lock poisoned, recovering: {e}");
            e.into_inner()
        });
        let instances: Vec<PersistedInstance> = self
            .instances
            .iter()
            .filter(|e| !matches!(e.status, HandStatus::Inactive))
            .map(|e| PersistedInstance {
                hand_id: e.hand_id.clone(),
                instance_id: e.instance_id,
                config: e.config.clone(),
                agent_ids: e.agent_ids.clone(),
                coordinator_role: e.coordinator_role.clone(),
                status: e.status.clone(),
                activated_at: Some(e.activated_at),
                updated_at: Some(e.updated_at),
            })
            .collect();
        let state = PersistedState {
            version: PERSIST_VERSION,
            instances,
        };
        let json = serde_json::to_string_pretty(&state)
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

        // Try typed deserialization first (v3 format with PersistedState struct).
        if let Ok(state) = serde_json::from_str::<PersistedState>(&data) {
            return state
                .instances
                .into_iter()
                .filter_map(|inst| {
                    let status = inst.status;
                    match &status {
                        HandStatus::Active | HandStatus::Paused => {}
                        HandStatus::Error(message) => {
                            info!(
                                hand = %inst.hand_id,
                                error = %message,
                                "Skipping errored hand from persisted state"
                            );
                            return None;
                        }
                        HandStatus::Inactive => {
                            info!(hand = %inst.hand_id, "Skipping inactive hand from persisted state");
                            return None;
                        }
                    }

                    let coordinator_role = HandInstance::normalize_coordinator_role(
                        &inst.agent_ids,
                        inst.coordinator_role.as_deref(),
                    );

                    Some(HandStateEntry {
                        hand_id: inst.hand_id,
                        config: inst.config,
                        old_agent_ids: inst.agent_ids,
                        coordinator_role,
                        status,
                        instance_id: Some(inst.instance_id),
                        activated_at: inst.activated_at,
                        updated_at: inst.updated_at,
                    })
                })
                .collect();
        }

        // Fallback: legacy v2/v1 format using untyped parsing.
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

                // v2: agent_id as single AgentId → convert to {"main": id}
                // v1: agent_id as single AgentId → same conversion
                let old_agent_ids: BTreeMap<String, AgentId> =
                    if let Some(id_val) = e.get("agent_id") {
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
                    activated_at: None,
                    updated_at: None,
                })
            })
            .collect()
    }

    /// Insert a definition into the registry, rejecting duplicates.
    ///
    /// Returns the stored definition (cloned from the map). This helper is the
    /// single point of truth for the "check duplicate → insert → return" pattern
    /// shared by all install methods.
    fn register_definition(&self, def: HandDefinition) -> HandResult<HandDefinition> {
        let id = def.id.clone();
        if self.definitions.contains_key(&id) {
            return Err(HandError::AlreadyRegistered(id));
        }
        self.definitions.insert(id.clone(), def);
        Ok(self.definitions.get(&id).unwrap().value().clone())
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
                    // insert returns Option<old_value>; Some means updated, None means added.
                    if self.definitions.insert(def.id.clone(), def).is_some() {
                        updated += 1;
                    } else {
                        added += 1;
                    }
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
        let skill_content = std::fs::read_to_string(&skill_path).unwrap_or_else(|e| {
            tracing::debug!(path = %skill_path.display(), error = %e, "Failed to read SKILL.md");
            String::new()
        });

        let agent_skill_content = scan_agent_skill_files(path);

        let agents_dir = resolve_agents_dir(home_dir);
        let def = parse_hand_toml_with_agents_dir(
            &toml_content,
            &skill_content,
            agent_skill_content,
            agents_dir.as_deref(),
        )?;
        let id = def.id.clone();
        let name = def.name.clone();
        let stored = self.register_definition(def)?;

        info!(hand = %id, name = %name, path = %path.display(), "Installed hand from path");
        Ok(stored)
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
        let id = def.id.clone();
        let name = def.name.clone();
        let stored = self.register_definition(def)?;

        info!(hand = %id, name = %name, "Installed hand from content");
        Ok(stored)
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
        let id = def.id.clone();
        let name = def.name.clone();

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

        let stored = self.register_definition(def)?;

        info!(
            hand = %id,
            name = %name,
            path = %hand_dir.display(),
            "Installed hand from content"
        );
        Ok(stored)
    }

    /// Uninstall a user-installed hand — removes it from memory and deletes
    /// its `workspaces/{id}/` directory on disk.
    ///
    /// Refuses to uninstall built-in hands (those that live under
    /// `registry/hands/`), since the next registry sync would just recreate
    /// them. Refuses to uninstall a hand that has any active instance;
    /// callers should deactivate first.
    ///
    /// Returns `HandError::NotFound` if no such hand is registered,
    /// `HandError::BuiltinHand` if the target is a built-in, and
    /// `HandError::AlreadyActive` if there is still a live instance.
    pub fn uninstall_hand(&self, home_dir: &std::path::Path, hand_id: &str) -> HandResult<()> {
        if !self.definitions.contains_key(hand_id) {
            return Err(HandError::NotFound(hand_id.to_string()));
        }

        // Built-in hands live under `home/registry/hands/{id}`. Those are
        // regenerated by the registry sync on every boot — deleting the
        // workspace entry would be pointless, and touching the registry dir
        // is out of scope. Refuse.
        let workspace_dir = home_dir.join("workspaces").join(hand_id);
        if !workspace_dir.join("HAND.toml").exists() {
            return Err(HandError::BuiltinHand(hand_id.to_string()));
        }

        // Refuse if any instance is still alive — the kernel would be
        // holding a stale reference to a definition we're about to drop.
        let has_live_instance = self.instances.iter().any(|e| e.value().hand_id == hand_id);
        if has_live_instance {
            return Err(HandError::AlreadyActive(format!(
                "Deactivate hand '{hand_id}' before uninstalling"
            )));
        }

        // Drop the in-memory definition first. If the filesystem removal
        // fails halfway, the next reload will repopulate the definition
        // from disk, leaving the system in a consistent state.
        self.definitions.remove(hand_id);

        std::fs::remove_dir_all(&workspace_dir)?;

        info!(
            hand = %hand_id,
            path = %workspace_dir.display(),
            "Uninstalled hand"
        );
        Ok(())
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
        self.activate_with_id(hand_id, config, None, None)
    }

    /// Like [`activate`](Self::activate) but allows specifying an existing instance UUID
    /// and optional preserved timestamps (for daemon restart recovery).
    pub fn activate_with_id(
        &self,
        hand_id: &str,
        config: HashMap<String, serde_json::Value>,
        instance_id: Option<Uuid>,
        timestamps: Option<(DateTime<Utc>, DateTime<Utc>)>,
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
        if instance_id.is_none() && self.active_index.contains_key(hand_id) {
            return Err(HandError::AlreadyActive(hand_id.to_string()));
        } else if let Some(id) = instance_id {
            if self.instances.contains_key(&id) {
                return Err(HandError::ActivationFailed(format!(
                    "Instance {id} already exists"
                )));
            }
        }

        let mut instance = HandInstance::new(hand_id, config, instance_id);
        // Restore original timestamps when recovering from persisted state.
        if let Some((activated, updated)) = timestamps {
            instance.activated_at = activated;
            instance.updated_at = updated;
        }
        let id = instance.instance_id;
        self.instances.insert(id, instance.clone());
        // Track in active_index — newly activated instances are Active by default.
        self.active_index.insert(hand_id.to_string(), id);
        info!(hand = %hand_id, instance = %id, "Hand activated");
        Ok(instance)
    }

    /// Deactivate a hand instance (agent killing is done by kernel).
    pub fn deactivate(&self, instance_id: Uuid) -> HandResult<HandInstance> {
        let (_, instance) = self
            .instances
            .remove(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        // Clean up reverse indexes.
        self.agent_index.retain(|_, v| *v != instance_id);
        // Only remove from active_index if it still points to this instance.
        // When multiple instances of the same hand_id exist (restart recovery),
        // we must not clobber the entry if another instance took over.
        if let Some(active_id) = self.active_index.get(&instance.hand_id) {
            if *active_id == instance_id {
                drop(active_id);
                self.active_index.remove(&instance.hand_id);
                // Re-insert another active instance of the same hand_id if one exists.
                if let Some(other) = self
                    .instances
                    .iter()
                    .find(|e| e.hand_id == instance.hand_id && e.status == HandStatus::Active)
                {
                    self.active_index
                        .insert(instance.hand_id.clone(), other.instance_id);
                }
            }
        }
        info!(hand = %instance.hand_id, instance = %instance_id, "Hand deactivated");
        Ok(instance)
    }

    /// Set the status of an instance, updating timestamps and indexes.
    fn set_status(&self, instance_id: Uuid, status: HandStatus) -> HandResult<()> {
        let mut entry = self
            .instances
            .get_mut(&instance_id)
            .ok_or(HandError::InstanceNotFound(instance_id))?;
        let hand_id = entry.hand_id.clone();
        entry.status = status.clone();
        entry.updated_at = chrono::Utc::now();
        drop(entry); // release the DashMap write lock before touching indexes

        match status {
            HandStatus::Active => {
                self.active_index.insert(hand_id, instance_id);
            }
            HandStatus::Paused | HandStatus::Error(_) | HandStatus::Inactive => {
                // Only remove from active_index if it still points to this instance,
                // and re-insert another active instance of the same hand_id if one exists.
                if let Some(active_id) = self.active_index.get(&hand_id) {
                    if *active_id == instance_id {
                        drop(active_id);
                        self.active_index.remove(&hand_id);
                        if let Some(other) = self
                            .instances
                            .iter()
                            .find(|e| e.hand_id == hand_id && e.status == HandStatus::Active)
                        {
                            self.active_index.insert(hand_id, other.instance_id);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Pause a hand instance.
    pub fn pause(&self, instance_id: Uuid) -> HandResult<()> {
        self.set_status(instance_id, HandStatus::Paused)
    }

    /// Resume a paused hand instance.
    pub fn resume(&self, instance_id: Uuid) -> HandResult<()> {
        self.set_status(instance_id, HandStatus::Active)
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
        // Capture old agent IDs before overwriting, for index cleanup.
        let old_agent_ids = std::mem::take(&mut entry.agent_ids);
        entry.agent_ids = agent_ids;
        entry.updated_at = chrono::Utc::now();
        // Update agent_index: remove old entries, add new entries.
        for aid in old_agent_ids.values() {
            self.agent_index.remove(&aid.to_string());
        }
        for aid in entry.agent_ids.values() {
            self.agent_index.insert(aid.to_string(), instance_id);
        }
        Ok(())
    }

    /// Backward-compatible: set a single agent ID under the "main" role.
    pub fn set_agent(&self, instance_id: Uuid, agent_id: AgentId) -> HandResult<()> {
        let mut map = BTreeMap::new();
        let key = "main".to_string();
        map.insert(key.clone(), agent_id);
        self.set_agents(instance_id, map, Some(key))
    }

    /// Find the hand instance associated with an agent (O(1) via reverse index).
    pub fn find_by_agent(&self, agent_id: AgentId) -> Option<HandInstance> {
        let instance_id = self.agent_index.get(&agent_id.to_string())?;
        self.instances
            .get(instance_id.value())
            .map(|e| e.value().clone())
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
        self.set_status(instance_id, HandStatus::Error(message))
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

        // A hand is active if at least one instance is in Active status (O(1) via index).
        let active = self.active_index.contains_key(hand_id);

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
                        .unwrap_or(false)
                    || std::path::Path::new(
                        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                    )
                    .exists()
                    || std::path::Path::new("/Applications/Chromium.app/Contents/MacOS/Chromium")
                        .exists();
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

/// Cached result of Python 3 availability check.
static PYTHON3_CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Check if Python 3 is actually available by running the command and checking
/// the version output. This avoids false negatives from Windows Store shims
/// (python3.exe that just opens the Microsoft Store) and false positives from
/// Python 2 installations where `python` exists but is Python 2.
///
/// The result is cached for the lifetime of the process using `OnceLock`.
fn check_python3_available() -> bool {
    *PYTHON3_CACHED.get_or_init(|| {
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
    })
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
///
/// On Unix, also verifies the execute bit is set. Empty PATH segments are
/// treated as the current directory per POSIX convention.
fn which_binary(name: &str) -> bool {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let extensions: Vec<&str> = if cfg!(windows) {
        vec!["", ".exe", ".cmd", ".bat"]
    } else {
        vec![""]
    };

    for raw_dir in std::env::split_paths(&path_var) {
        // POSIX: empty PATH segment means current directory.
        let dir = if raw_dir.as_os_str().is_empty() && !cfg!(windows) {
            std::path::PathBuf::from(".")
        } else {
            raw_dir
        };
        for ext in &extensions {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() && is_executable(&candidate) {
                return true;
            }
        }
    }
    false
}

/// Check if a path is executable. On Unix, verifies the execute bit.
/// On Windows, all files are considered executable (permissions are ACL-based).
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

/// Return alternative environment variable names for a given env var.
/// Used to support aliases (e.g. GEMINI_API_KEY also accepts GOOGLE_API_KEY).
fn env_aliases(env: &str) -> Vec<&str> {
    match env {
        "GEMINI_API_KEY" => vec![env, "GOOGLE_API_KEY"],
        _ => vec![env],
    }
}

/// Check if a setting option is available based on its provider_env and binary.
///
/// - No provider_env and no binary → always available (e.g. "auto", "none")
/// - provider_env set → check if env var (or any alias) is non-empty
/// - binary set → check if binary is on PATH
fn check_option_available(provider_env: Option<&str>, binary: Option<&str>) -> bool {
    let env_ok = match provider_env {
        None => true,
        Some(env) => env_aliases(env)
            .iter()
            .any(|e| std::env::var(e).map(|v| !v.is_empty()).unwrap_or(false)),
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
        assert!(count >= 15, "expected at least 15 hands, got {count}");
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

    // ── uninstall_hand ───────────────────────────────────────────────────
    //
    // These tests cover all four branches of `uninstall_hand` so we don't
    // regress the contract the DELETE /api/hands/{id} route depends on:
    //
    //   1. Hand id unknown               → NotFound
    //   2. Hand is built-in (registry)   → BuiltinHand, definition untouched
    //   3. Hand still has a live inst.   → AlreadyActive, nothing touched
    //   4. Custom hand, no live inst.    → Ok, definition + workspace gone
    //
    // The "built-in" case exercises the specific rule that
    // `home/workspaces/{id}/HAND.toml` must exist for a hand to be
    // uninstallable — anything else (even if loaded into memory) is
    // assumed to come from the registry cache and would be recreated on
    // the next sync.

    const UNINSTALL_TEST_TOML: &str = r#"
id = "__PLACEHOLDER__"
name = "Test Hand"
description = "Fixture for uninstall_hand tests"
category = "data"

[routing]
aliases = []

[agent]
name = "test-agent"
description = "Agent"
system_prompt = "Test"
"#;

    fn install_custom_for_uninstall(
        reg: &HandRegistry,
        home: &std::path::Path,
        id: &str,
    ) -> HandDefinition {
        let toml = UNINSTALL_TEST_TOML.replace("__PLACEHOLDER__", id);
        reg.install_from_content_persisted(home, &toml, "# skill\n")
            .unwrap()
    }

    #[test]
    fn uninstall_nonexistent_hand_returns_not_found() {
        let reg = HandRegistry::new();
        let tmp = tempfile::tempdir().unwrap();

        let err = reg
            .uninstall_hand(tmp.path(), "does-not-exist")
            .unwrap_err();

        assert!(
            matches!(err, HandError::NotFound(ref id) if id == "does-not-exist"),
            "expected NotFound(\"does-not-exist\"), got {err:?}"
        );
    }

    #[test]
    fn uninstall_builtin_hand_is_refused() {
        let reg = HandRegistry::new();
        let tmp = tempfile::tempdir().unwrap();

        // Simulate a built-in: create the HAND.toml under
        // `home/registry/hands/{id}/` (the registry cache location) and
        // reload. After reload, the definition exists in memory, but the
        // `home/workspaces/{id}/HAND.toml` file does NOT — that is exactly
        // the state uninstall_hand treats as "built-in".
        let reg_hand_dir = tmp.path().join("registry").join("hands").join("builtin-x");
        std::fs::create_dir_all(&reg_hand_dir).unwrap();
        std::fs::write(
            reg_hand_dir.join("HAND.toml"),
            UNINSTALL_TEST_TOML.replace("__PLACEHOLDER__", "builtin-x"),
        )
        .unwrap();
        reg.reload_from_disk(tmp.path());

        assert!(
            reg.get_definition("builtin-x").is_some(),
            "pre-check: hand should be loaded from registry/hands/"
        );
        assert!(
            !tmp.path().join("workspaces/builtin-x/HAND.toml").exists(),
            "pre-check: no user-installed copy should exist"
        );

        let err = reg.uninstall_hand(tmp.path(), "builtin-x").unwrap_err();

        assert!(
            matches!(err, HandError::BuiltinHand(ref id) if id == "builtin-x"),
            "expected BuiltinHand(\"builtin-x\"), got {err:?}"
        );

        // A failed uninstall MUST leave the in-memory registry untouched —
        // otherwise the UI would see the hand disappear until the next
        // reload, and the reload would silently bring it back, confusing
        // the user.
        assert!(
            reg.get_definition("builtin-x").is_some(),
            "definition must be preserved after refused uninstall"
        );
    }

    #[test]
    fn uninstall_active_hand_is_refused() {
        let reg = HandRegistry::new();
        let tmp = tempfile::tempdir().unwrap();
        install_custom_for_uninstall(&reg, tmp.path(), "test-hand");

        reg.activate("test-hand", HashMap::new()).unwrap();

        let err = reg.uninstall_hand(tmp.path(), "test-hand").unwrap_err();

        assert!(
            matches!(err, HandError::AlreadyActive(_)),
            "expected AlreadyActive, got {err:?}"
        );

        // Nothing should be touched on a refused uninstall — both the
        // in-memory definition and the on-disk workspace must survive.
        assert!(reg.get_definition("test-hand").is_some());
        assert!(tmp.path().join("workspaces/test-hand/HAND.toml").exists());
    }

    #[test]
    fn uninstall_custom_hand_removes_definition_and_workspace() {
        let reg = HandRegistry::new();
        let tmp = tempfile::tempdir().unwrap();
        install_custom_for_uninstall(&reg, tmp.path(), "removable");

        let workspace = tmp.path().join("workspaces").join("removable");
        assert!(workspace.join("HAND.toml").exists(), "pre-check");
        assert!(reg.get_definition("removable").is_some(), "pre-check");

        reg.uninstall_hand(tmp.path(), "removable").unwrap();

        assert!(
            reg.get_definition("removable").is_none(),
            "definition must be dropped from memory"
        );
        assert!(
            !workspace.exists(),
            "workspace directory must be removed from disk"
        );
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

    #[serial_test::serial]
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
    fn parse_hand_toml_wrapped_format() {
        let wrapped_toml = r#"
[hand]
id = "wrapped-test"
name = "Wrapped Test"
description = "Test with [hand] wrapper"
category = "content"

[hand.agent]
name = "wrapped-agent"
description = "Test"
system_prompt = "Test."

[hand.dashboard]
metrics = []
"#;
        let def = parse_hand_toml(wrapped_toml, "skill content", HashMap::new()).unwrap();
        assert_eq!(def.id, "wrapped-test");
        assert_eq!(def.name, "Wrapped Test");
        assert_eq!(def.skill_content.as_deref(), Some("skill content"));
    }

    #[test]
    fn parse_hand_toml_flat_format_with_skill() {
        let flat_toml = r#"
id = "flat-test"
name = "Flat Test"
description = "Flat format"
category = "data"

[agent]
name = "flat-agent"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        let def = parse_hand_toml(flat_toml, "", HashMap::new()).unwrap();
        assert_eq!(def.id, "flat-test");
        assert!(def.skill_content.is_none());
    }

    #[test]
    fn parse_hand_toml_invalid_toml() {
        let result = parse_hand_toml("this is not valid toml [[[[", "", HashMap::new());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HandError::TomlParse(_)));
    }

    #[test]
    fn install_from_content_in_memory() {
        let reg = HandRegistry::new();
        let toml_content = r#"
id = "memory-hand"
name = "Memory Hand"
description = "In-memory install"
category = "data"

[agent]
name = "mem-agent"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        let def = reg.install_from_content(toml_content, "").unwrap();
        assert_eq!(def.id, "memory-hand");
        assert!(reg.get_definition("memory-hand").is_some());

        // Duplicate should fail
        let dup = reg.install_from_content(toml_content, "");
        assert!(dup.is_err());
        assert!(matches!(dup.unwrap_err(), HandError::AlreadyRegistered(_)));
    }

    #[test]
    fn install_from_path_from_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let hand_toml = r#"
id = "path-hand"
name = "Path Hand"
description = "Installed from dir"
category = "productivity"

[agent]
name = "path-agent"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        std::fs::write(tmp.path().join("HAND.toml"), hand_toml).unwrap();
        std::fs::write(tmp.path().join("SKILL.md"), "# Skill content").unwrap();

        let reg = HandRegistry::new();
        let def = reg
            .install_from_path(tmp.path(), &ensure_test_home())
            .unwrap();
        assert_eq!(def.id, "path-hand");
        assert_eq!(def.skill_content.as_deref(), Some("# Skill content"));
        assert!(reg.get_definition("path-hand").is_some());

        // Duplicate from path should fail
        let dup = reg.install_from_path(tmp.path(), &ensure_test_home());
        assert!(dup.is_err());
    }

    #[test]
    fn install_from_path_missing_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = HandRegistry::new();
        let result = reg.install_from_path(tmp.path(), &ensure_test_home());
        assert!(result.is_err());
    }

    #[test]
    fn activate_with_explicit_instance_id() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let custom_id = Uuid::new_v4();
        let instance = reg
            .activate_with_id("clip", HashMap::new(), Some(custom_id), None)
            .unwrap();
        assert_eq!(instance.instance_id, custom_id);
        assert_eq!(instance.hand_id, "clip");

        // Re-activate with same UUID should fail
        let dup = reg.activate_with_id("clip", HashMap::new(), Some(custom_id), None);
        assert!(dup.is_err());

        reg.deactivate(custom_id).unwrap();
    }

    #[test]
    fn update_config_replaces_and_refreshes() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let instance = reg.activate("clip", HashMap::new()).unwrap();
        let id = instance.instance_id;
        let before = reg.get_instance(id).unwrap().updated_at;

        let mut new_config = HashMap::new();
        new_config.insert("key".to_string(), serde_json::json!("value"));
        reg.update_config(id, new_config).unwrap();

        let updated = reg.get_instance(id).unwrap();
        assert_eq!(updated.config["key"], serde_json::json!("value"));
        assert!(updated.updated_at >= before);

        // Nonexistent instance
        assert!(reg.update_config(Uuid::new_v4(), HashMap::new()).is_err());

        reg.deactivate(id).unwrap();
    }

    #[test]
    fn load_state_v1_bare_array_format() {
        let path = std::env::temp_dir().join(format!("hand-state-v1-{}.json", Uuid::new_v4()));
        std::fs::write(
            &path,
            serde_json::json!([{
                "hand_id": "lead",
                "config": {},
                "agent_id": serde_json::Value::Null,
                "status": "Active",
            }])
            .to_string(),
        )
        .unwrap();

        let restored = HandRegistry::load_state(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].hand_id, "lead");
        assert!(matches!(restored[0].status, HandStatus::Active));
    }

    #[test]
    fn load_state_v3_instance_id_roundtrip() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        let custom_id = Uuid::new_v4();
        let _instance = reg
            .activate_with_id("clip", HashMap::new(), Some(custom_id), None)
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("hand_state.json");
        reg.persist_state(&state_path).unwrap();

        let saved = HandRegistry::load_state(&state_path);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].instance_id, Some(custom_id));
        assert_eq!(saved[0].hand_id, "clip");

        reg.deactivate(custom_id).unwrap();
    }

    #[test]
    fn check_settings_availability_basic() {
        let reg = HandRegistry::new();
        reg.reload_from_disk(&ensure_test_home());

        // Clip hand has settings
        let result = reg.check_settings_availability("clip", None);
        assert!(result.is_ok());
        let statuses = result.unwrap();
        // Check structure — each SettingStatus has key, label, options
        for s in &statuses {
            assert!(!s.key.is_empty());
            assert!(!s.label.is_empty());
            // options don't have to be non-empty (toggle/text don't have options)
        }
    }

    #[test]
    fn check_settings_availability_with_i18n() {
        let reg = HandRegistry::new();
        let toml_content = r#"
id = "i18n-hand"
name = "I18n Hand"
description = "Test"
category = "content"

[[settings]]
key = "provider"
label = "Provider"
description = "Choose provider"
setting_type = "select"
default = "auto"

[[settings.options]]
value = "auto"
label = "Auto"

[i18n.zh]
name = "国际化测试"

[i18n.zh.settings.provider]
label = "提供商"
description = "选择提供商"

[agent]
name = "test-agent"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        reg.install_from_content(toml_content, "").unwrap();

        let statuses_en = reg.check_settings_availability("i18n-hand", None).unwrap();
        assert_eq!(statuses_en[0].label, "Provider");

        let statuses_zh = reg
            .check_settings_availability("i18n-hand", Some("zh"))
            .unwrap();
        assert_eq!(statuses_zh[0].label, "提供商");
        assert_eq!(statuses_zh[0].description, "选择提供商");
    }

    #[serial_test::serial]
    #[test]
    fn check_option_available_env_and_binary() {
        // No env, no binary → always available
        assert!(check_option_available(None, None));

        // Env var set
        std::env::set_var("LIBREFANG_TEST_OPT_ENV", "yes");
        assert!(check_option_available(Some("LIBREFANG_TEST_OPT_ENV"), None));
        assert!(!check_option_available(
            Some("LIBREFANG_NONEXISTENT_ENV_99999"),
            None
        ));
        std::env::remove_var("LIBREFANG_TEST_OPT_ENV");

        // Gemini fallback: GEMINI_API_KEY also checks GOOGLE_API_KEY
        std::env::set_var("GOOGLE_API_KEY", "test-key");
        assert!(check_option_available(Some("GEMINI_API_KEY"), None));
        std::env::remove_var("GOOGLE_API_KEY");
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
