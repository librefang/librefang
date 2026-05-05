//! Canonical agent UUID registry — refs #4614.
//!
//! Persists `agent_name → canonical_uuid` mappings *independently* of the
//! agent registry / SQLite agent rows so that a respawn (after a panic, a
//! manifest reload, an explicit kill, etc.) reuses the same `AgentId` instead
//! of generating a fresh one. Without this, sessions / memories / cron jobs
//! keyed under the prior UUID become silently orphaned.
//!
//! Today's spawn path already derives top-level agent IDs deterministically
//! via [`AgentId::from_name`] (UUID v5), which preserves identity for agents
//! whose `name` never changes. The registry adds a layer of explicit history
//! on top:
//!
//! 1. **Identity stability under rename / re-derivation.** The recorded UUID
//!    survives even if the v5 derivation later evolves (e.g. namespace bump,
//!    name normalisation change). Rather than silently rewriting every
//!    user's existing data, the kernel keeps honoring whatever id was first
//!    handed out.
//! 2. **Explicit delete-vs-purge separation.** A normal `kill_agent` keeps
//!    the registry entry intact, so a later respawn lands back on the same
//!    UUID — surviving sessions remain reachable. A purge (explicit
//!    `?purge_identity=true`) drops the entry; the next spawn starts from a
//!    clean slate.
//!
//! Storage: a TOML file at `<home_dir>/agent_identities.toml` written
//! atomically (write to `.tmp.<pid>.<seq>.<nanos>`, fsync, rename). Schema is
//! intentionally narrow — the file is *not* a config the user is expected to
//! edit by hand, but it is human-readable for emergency surgery.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use librefang_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::{debug, warn};

/// File name used inside `home_dir`.
const FILE_NAME: &str = "agent_identities.toml";

/// One persisted entry: the canonical UUID assigned to an agent name plus
/// the timestamp at which the binding was first recorded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentIdentityRecord {
    /// The canonical [`AgentId`] (UUID) for this agent name.
    pub canonical_uuid: AgentId,
    /// When the mapping was first registered.
    pub created_at: DateTime<Utc>,
}

/// Top-level on-disk shape: `[agents.<name>]` tables.
///
/// ```toml
/// [agents.nika]
/// canonical_uuid = "660bef7c-04d5-4480-8af2-0ce029981a14"
/// created_at = "2026-04-01T10:00:00Z"
/// ```
#[derive(Debug, Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default)]
    agents: std::collections::BTreeMap<String, AgentIdentityRecord>,
}

/// In-memory canonical-UUID registry.
///
/// Concurrency: a `DashMap` for the read/insert path; a separate `Mutex`
/// guards the on-disk write so two concurrent persisters don't race on
/// `rename`. The DashMap entries are the source of truth — `persist` simply
/// snapshots them.
#[derive(Debug)]
pub struct AgentIdentityRegistry {
    map: DashMap<String, AgentIdentityRecord>,
    persist_path: Option<PathBuf>,
    /// Serialises atomic writes so two `register` calls in flight never
    /// produce an interleaved on-disk file.
    persist_lock: Mutex<()>,
}

impl AgentIdentityRegistry {
    /// Build an empty in-memory registry with no persistence (test helper).
    pub fn in_memory() -> Self {
        Self {
            map: DashMap::new(),
            persist_path: None,
            persist_lock: Mutex::new(()),
        }
    }

    /// Build a registry rooted at `home_dir`, eagerly loading any existing
    /// `agent_identities.toml`. Errors during load are logged and treated
    /// as "empty registry" — we never want to silently lose entries by
    /// returning `Err` from boot, but we also don't want a malformed file
    /// to wipe the user's history. See [`load_from`] for the explicit form
    /// used by tests.
    pub fn load(home_dir: &Path) -> Self {
        let persist_path = home_dir.join(FILE_NAME);
        let map = match Self::read_file(&persist_path) {
            Ok(entries) => {
                let map = DashMap::with_capacity(entries.len());
                for (name, identity) in entries {
                    map.insert(name, identity);
                }
                map
            }
            Err(e) => {
                warn!(
                    path = %persist_path.display(),
                    error = %e,
                    "agent_identities.toml: failed to load — starting empty (existing file left intact)"
                );
                DashMap::new()
            }
        };
        Self {
            map,
            persist_path: Some(persist_path),
            persist_lock: Mutex::new(()),
        }
    }

    /// Read raw entries from a path. Missing file ⇒ `Ok(empty)`.
    fn read_file(
        path: &Path,
    ) -> Result<std::collections::BTreeMap<String, AgentIdentityRecord>, std::io::Error> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let parsed: OnDisk = toml::from_str(&s).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("agent_identities.toml: parse error: {e}"),
                    )
                })?;
                Ok(parsed.agents)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(std::collections::BTreeMap::new())
            }
            Err(e) => Err(e),
        }
    }

    /// Look up the canonical UUID for `name`, if one was previously recorded.
    pub fn get(&self, name: &str) -> Option<AgentId> {
        self.map.get(name).map(|e| e.canonical_uuid)
    }

    /// Snapshot the current entries. Stable order (BTreeMap) so callers can
    /// rely on deterministic output for diagnostics.
    pub fn list(&self) -> std::collections::BTreeMap<String, AgentIdentityRecord> {
        self.map
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Number of recorded mappings.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when no mappings are recorded.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Register `name → canonical_uuid` if no entry exists. If an entry
    /// already exists, it is **not** overwritten — the first UUID issued
    /// to that name wins, even if the caller passes a different one.
    /// Returns the canonical UUID after the call (existing or freshly
    /// inserted).
    ///
    /// Persists to disk on insert. A persistence error is logged but does
    /// not fail the in-memory write — the kernel needs the in-memory map
    /// to be authoritative for the running process even if the disk is
    /// momentarily wedged.
    pub fn register_if_absent(&self, name: &str, canonical_uuid: AgentId) -> AgentId {
        if let Some(existing) = self.map.get(name) {
            return existing.canonical_uuid;
        }
        let entry = AgentIdentityRecord {
            canonical_uuid,
            created_at: Utc::now(),
        };
        // Race-window: between the `get` above and the insert below, another
        // thread could register the same name. `entry().or_insert_with` would
        // be cleaner, but DashMap's `entry` API holds a write guard for the
        // duration of the closure — fine here since the closure is cheap.
        let final_uuid = self
            .map
            .entry(name.to_string())
            .or_insert(entry)
            .canonical_uuid;
        if let Err(e) = self.persist() {
            warn!(
                name,
                error = %e,
                "agent_identities.toml: persist failed (in-memory entry retained)"
            );
        }
        final_uuid
    }

    /// Remove the entry for `name`, if any. Returns the dropped UUID so
    /// callers can audit what was purged. Persists on success.
    pub fn purge(&self, name: &str) -> Option<AgentId> {
        let dropped = self.map.remove(name).map(|(_, v)| v.canonical_uuid)?;
        if let Err(e) = self.persist() {
            warn!(
                name,
                error = %e,
                "agent_identities.toml: persist failed after purge (in-memory removal retained)"
            );
        }
        Some(dropped)
    }

    /// Persist the current in-memory state to disk via atomic write.
    /// No-op when the registry was constructed without a persist path.
    pub fn persist(&self) -> Result<(), std::io::Error> {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(()),
        };
        let _guard = self.persist_lock.lock().unwrap_or_else(|e| e.into_inner());

        let snapshot: std::collections::BTreeMap<String, AgentIdentityRecord> = self
            .map
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();

        let on_disk = OnDisk { agents: snapshot };
        let body = toml::to_string_pretty(&on_disk).map_err(|e| {
            std::io::Error::other(format!("agent_identities.toml: serialize failed: {e}"))
        })?;

        // Ensure parent dir exists. `home_dir` is normally created at boot,
        // but tests sometimes hand in a path under a freshly-made tempdir
        // before the boot scaffolding ran.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp_path = crate::persist_tmp_path(path);
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp_path, path)?;
        debug!(
            path = %path.display(),
            count = self.map.len(),
            "Persisted agent_identities.toml"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_is_lossless() {
        let dir = tempdir().unwrap();
        let reg = AgentIdentityRegistry::load(dir.path());
        assert!(reg.is_empty());

        let alice_uuid = AgentId::from_name("alice");
        let bob_uuid = AgentId::from_name("bob");
        let returned = reg.register_if_absent("alice", alice_uuid);
        assert_eq!(returned, alice_uuid);
        reg.register_if_absent("bob", bob_uuid);
        assert_eq!(reg.len(), 2);

        // Re-load — same path — should reproduce the same entries.
        let reloaded = AgentIdentityRegistry::load(dir.path());
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.get("alice"), Some(alice_uuid));
        assert_eq!(reloaded.get("bob"), Some(bob_uuid));
    }

    #[test]
    fn first_register_wins() {
        let dir = tempdir().unwrap();
        let reg = AgentIdentityRegistry::load(dir.path());
        let first = AgentId::from_name("nika");
        let intruder = AgentId::new(); // random — different UUID
        assert_ne!(first, intruder);

        let got1 = reg.register_if_absent("nika", first);
        assert_eq!(got1, first);
        let got2 = reg.register_if_absent("nika", intruder);
        assert_eq!(
            got2, first,
            "second register must not clobber the canonical UUID"
        );
    }

    #[test]
    fn purge_removes_and_persists() {
        let dir = tempdir().unwrap();
        let reg = AgentIdentityRegistry::load(dir.path());
        let uuid = AgentId::from_name("ephemeral");
        reg.register_if_absent("ephemeral", uuid);

        let dropped = reg.purge("ephemeral");
        assert_eq!(dropped, Some(uuid));
        assert!(reg.is_empty());

        // Re-load: the file on disk must reflect the purge.
        let reloaded = AgentIdentityRegistry::load(dir.path());
        assert!(reloaded.is_empty());
    }

    #[test]
    fn purge_missing_is_ok() {
        let dir = tempdir().unwrap();
        let reg = AgentIdentityRegistry::load(dir.path());
        assert!(reg.purge("never-existed").is_none());
    }

    #[test]
    fn malformed_file_is_treated_as_empty_and_left_alone() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "this is not valid toml ===\n").unwrap();
        let original = std::fs::read_to_string(&path).unwrap();

        let reg = AgentIdentityRegistry::load(dir.path());
        assert!(reg.is_empty());
        // The malformed file must NOT have been overwritten by the
        // empty-registry view — load() is a *read*, and silently rewriting
        // the file would destroy the operator's chance to recover by hand.
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, original);
    }

    #[test]
    fn in_memory_persist_is_noop() {
        let reg = AgentIdentityRegistry::in_memory();
        let uuid = AgentId::from_name("foo");
        reg.register_if_absent("foo", uuid);
        // No persist path — must not panic, must succeed.
        reg.persist().expect("in-memory persist is a no-op");
    }

    #[test]
    fn list_is_deterministic() {
        let dir = tempdir().unwrap();
        let reg = AgentIdentityRegistry::load(dir.path());
        for name in ["c", "a", "b"] {
            reg.register_if_absent(name, AgentId::from_name(name));
        }
        let listed: Vec<String> = reg.list().keys().cloned().collect();
        assert_eq!(
            listed,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }
}
