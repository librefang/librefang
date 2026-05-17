//! SQLite structured store for key-value pairs and agent persistence.

use chrono::Utc;
use librefang_types::agent::{AgentEntry, AgentId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

/// Hard ceiling on a single serialized KV value, enforced inside
/// [`StructuredStore::set`] / [`StructuredStore::modify`] /
/// [`StructuredStore::set_returning_existed`] so no call path can land an
/// unbounded blob (#5138). 256 KiB comfortably holds the largest legitimate
/// structured payloads (goal arrays, peer KV) while keeping worst-case row
/// size, WAL replay, and cold-load RAM bounded. An over-limit write is
/// rejected with [`LibreFangError::InvalidInput`] *before* the INSERT runs,
/// so a coerced agent cannot wedge the substrate with a 100 MB array.
pub const MAX_KV_VALUE_BYTES: usize = 256 * 1024;

/// Reject a serialized value that exceeds [`MAX_KV_VALUE_BYTES`].
fn check_value_size(blob: &[u8], key: &str) -> LibreFangResult<()> {
    if blob.len() > MAX_KV_VALUE_BYTES {
        return Err(LibreFangError::InvalidInput(format!(
            "memory value for key '{key}' is {} bytes, exceeds the {MAX_KV_VALUE_BYTES}-byte limit",
            blob.len()
        )));
    }
    Ok(())
}

/// Structured store backed by SQLite for key-value operations and agent storage.
#[derive(Clone)]
pub struct StructuredStore {
    pool: Pool<SqliteConnectionManager>,
}

impl StructuredStore {
    /// Create a new structured store wrapping the given connection pool.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Get a value from the key-value store.
    pub fn get(&self, agent_id: AgentId, key: &str) -> LibreFangResult<Option<serde_json::Value>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT value FROM kv_store WHERE agent_id = ?1 AND key = ?2")
            .map_err(LibreFangError::memory)?;
        let result = stmt.query_row(rusqlite::params![agent_id.0.to_string(), key], |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob)
        });
        match result {
            Ok(blob) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&blob).map_err(LibreFangError::serialization)?;
                Ok(Some(value))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::memory(e)),
        }
    }

    /// Set a value in the key-value store.
    pub fn set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let blob = serde_json::to_vec(&value).map_err(LibreFangError::serialization)?;
        check_value_size(&blob, key)?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO kv_store (agent_id, key, value, version, updated_at) VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(agent_id, key) DO UPDATE SET value = ?3, version = version + 1, updated_at = ?4",
            rusqlite::params![agent_id.0.to_string(), key, blob, now],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Atomic read-modify-write of a single KV key under a `BEGIN IMMEDIATE`
    /// write transaction (#5138).
    ///
    /// Loads the current value (or `None`), hands it to `f`, and persists the
    /// returned value — all inside one SQLite write lock, so two concurrent
    /// `modify` calls on the same key serialize instead of clobbering each
    /// other (the lost-update / last-writer-wins race that the goals routes
    /// and `goal_update` previously had with a plain `get` → mutate → `set`).
    /// `BEGIN IMMEDIATE` escalates to a write lock immediately, mirroring the
    /// proven `SessionStore::append_canonical` shape for the identical
    /// single-shared-blob pattern.
    ///
    /// `f` may return an error to abort the transaction without writing; the
    /// error is propagated and the row is left unchanged. `f`'s `Ok` payload
    /// is also returned to the caller so handlers can echo the mutated entity
    /// without a second read.
    pub fn modify<T>(
        &self,
        agent_id: AgentId,
        key: &str,
        f: impl FnOnce(Option<serde_json::Value>) -> LibreFangResult<(serde_json::Value, T)>,
    ) -> LibreFangResult<T> {
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;

        let current: Option<serde_json::Value> = {
            let mut stmt = tx
                .prepare("SELECT value FROM kv_store WHERE agent_id = ?1 AND key = ?2")
                .map_err(LibreFangError::memory)?;
            let row = stmt.query_row(rusqlite::params![agent_id.0.to_string(), key], |row| {
                let blob: Vec<u8> = row.get(0)?;
                Ok(blob)
            });
            match row {
                Ok(blob) => {
                    Some(serde_json::from_slice(&blob).map_err(LibreFangError::serialization)?)
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(LibreFangError::memory(e)),
            }
        };

        let (new_value, out) = f(current)?;
        let blob = serde_json::to_vec(&new_value).map_err(LibreFangError::serialization)?;
        check_value_size(&blob, key)?;
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO kv_store (agent_id, key, value, version, updated_at) VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(agent_id, key) DO UPDATE SET value = ?3, version = version + 1, updated_at = ?4",
            rusqlite::params![agent_id.0.to_string(), key, blob, now],
        )
        .map_err(LibreFangError::memory)?;
        tx.commit().map_err(LibreFangError::memory)?;
        Ok(out)
    }

    /// Set a value and report whether the key already existed, atomically
    /// (#5138).
    ///
    /// The existence check and the write run inside one `BEGIN IMMEDIATE`
    /// transaction, so the returned `bool` reflects the state the write
    /// actually replaced — not a pre-read that could race with a concurrent
    /// first-time write. `memory_store` uses this to publish
    /// `MemoryUpdate{Created|Updated}` based on the committed transition
    /// rather than a stale `had_old` snapshot.
    ///
    /// Returns `true` if a prior value was overwritten, `false` if this
    /// created the key.
    pub fn set_returning_existed(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> LibreFangResult<bool> {
        let blob = serde_json::to_vec(&value).map_err(LibreFangError::serialization)?;
        check_value_size(&blob, key)?;
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(LibreFangError::memory)?;
        let existed: bool = tx
            .query_row(
                "SELECT 1 FROM kv_store WHERE agent_id = ?1 AND key = ?2",
                rusqlite::params![agent_id.0.to_string(), key],
                |_| Ok(()),
            )
            .map(|_| true)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(false),
                other => Err(LibreFangError::memory(other)),
            })?;
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO kv_store (agent_id, key, value, version, updated_at) VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(agent_id, key) DO UPDATE SET value = ?3, version = version + 1, updated_at = ?4",
            rusqlite::params![agent_id.0.to_string(), key, blob, now],
        )
        .map_err(LibreFangError::memory)?;
        tx.commit().map_err(LibreFangError::memory)?;
        Ok(existed)
    }

    /// Delete a value from the key-value store.
    pub fn delete(&self, agent_id: AgentId, key: &str) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "DELETE FROM kv_store WHERE agent_id = ?1 AND key = ?2",
            rusqlite::params![agent_id.0.to_string(), key],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Get wrapper guarded by a [`MemoryNamespaceGuard`]. The namespace
    /// presented to the guard is `kv:<key>` so callers can express
    /// per-prefix policies (e.g. `readable_namespaces = ["kv:user_*"]`).
    pub fn get_with_guard(
        &self,
        agent_id: AgentId,
        key: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> LibreFangResult<Option<serde_json::Value>> {
        let namespace = format!("kv:{key}");
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_read(&namespace) {
            return Err(LibreFangError::AuthDenied(reason));
        }
        self.get(agent_id, key)
    }

    /// Set wrapper guarded by a [`MemoryNamespaceGuard`].
    pub fn set_with_guard(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> LibreFangResult<()> {
        let namespace = format!("kv:{key}");
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_write(&namespace) {
            return Err(LibreFangError::AuthDenied(reason));
        }
        self.set(agent_id, key, value)
    }

    /// Delete wrapper guarded by a [`MemoryNamespaceGuard`]. Honours
    /// `delete_allowed` in addition to the write check.
    pub fn delete_with_guard(
        &self,
        agent_id: AgentId,
        key: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> LibreFangResult<()> {
        let namespace = format!("kv:{key}");
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_delete(&namespace) {
            return Err(LibreFangError::AuthDenied(reason));
        }
        self.delete(agent_id, key)
    }

    /// List all key-value pairs for an agent.
    pub fn list_kv(&self, agent_id: AgentId) -> LibreFangResult<Vec<(String, serde_json::Value)>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT key, value FROM kv_store WHERE agent_id = ?1 ORDER BY key")
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id.0.to_string()], |row| {
                let key: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                Ok((key, blob))
            })
            .map_err(LibreFangError::memory)?;

        let mut pairs = Vec::new();
        for row in rows {
            let (key, blob) = row.map_err(LibreFangError::memory)?;
            let value: serde_json::Value = serde_json::from_slice(&blob).unwrap_or_else(|_| {
                // Fallback: try as UTF-8 string
                String::from_utf8(blob)
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null)
            });
            pairs.push((key, value));
        }
        Ok(pairs)
    }

    /// List only keys for an agent (without values).
    pub fn list_keys(&self, agent_id: AgentId) -> LibreFangResult<Vec<String>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT key FROM kv_store WHERE agent_id = ?1 ORDER BY key")
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id.0.to_string()], |row| {
                let key: String = row.get(0)?;
                Ok(key)
            })
            .map_err(LibreFangError::memory)?;

        let mut keys = Vec::new();
        for row in rows {
            let key = row.map_err(LibreFangError::memory)?;
            keys.push(key);
        }
        Ok(keys)
    }

    /// Save an agent entry to the database.
    pub fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        // Use named-field encoding so new fields with #[serde(default)] are
        // handled gracefully when the struct evolves between versions.
        let manifest_blob =
            rmp_serde::to_vec_named(&entry.manifest).map_err(LibreFangError::serialization)?;
        let state_str =
            serde_json::to_string(&entry.state).map_err(LibreFangError::serialization)?;
        let now = Utc::now().to_rfc3339();

        // NOTE(#5138): the `session_id` / `identity` / `source_toml_path`
        // columns are NO LONGER added here. They were previously fired as
        // three `let _ = ALTER TABLE agents ADD COLUMN ...` on every
        // `save_agent`, swallowing the "duplicate column" error on the
        // common path. That bypassed the migration ladder entirely — the
        // columns never appeared in any `migrate_vN`, so `user_version` and
        // the `migrations` audit trail never reflected them, and deleting an
        // `ALTER` in a refactor would silently break fresh installs that
        // never had the column. They are now declared in `migrate_v40`,
        // which runs once at substrate boot inside the laddered transaction.

        let identity_json =
            serde_json::to_string(&entry.identity).map_err(LibreFangError::serialization)?;
        let source_toml_path = entry
            .source_toml_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        conn.execute(
            "INSERT INTO agents (id, name, manifest, state, created_at, updated_at, session_id, identity, source_toml_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET name = ?2, manifest = ?3, state = ?4, updated_at = ?6, session_id = ?7, identity = ?8, source_toml_path = ?9",
            rusqlite::params![
                entry.id.0.to_string(),
                entry.name,
                manifest_blob,
                state_str,
                entry.created_at.to_rfc3339(),
                now,
                entry.session_id.0.to_string(),
                identity_json,
                source_toml_path,
            ],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Load an agent entry from the database.
    pub fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let mut stmt = conn
            .prepare("SELECT id, name, manifest, state, created_at, updated_at, session_id, identity, source_toml_path FROM agents WHERE id = ?1")
            .or_else(|_| {
                conn.prepare("SELECT id, name, manifest, state, created_at, updated_at, session_id, identity FROM agents WHERE id = ?1")
                    .or_else(|_| {
                        conn.prepare("SELECT id, name, manifest, state, created_at, updated_at, session_id FROM agents WHERE id = ?1")
                    })
                    .or_else(|_| {
                        // Fallback without session_id/source_toml_path columns for old DBs
                        conn.prepare("SELECT id, name, manifest, state, created_at, updated_at FROM agents WHERE id = ?1")
                    })
            })
            .map_err(LibreFangError::memory)?;

        let col_count = stmt.column_count();
        let result = stmt.query_row(rusqlite::params![agent_id.0.to_string()], |row| {
            let manifest_blob: Vec<u8> = row.get(2)?;
            let state_str: String = row.get(3)?;
            let created_str: String = row.get(4)?;
            let name: String = row.get(1)?;
            let session_id_str: Option<String> = if col_count >= 7 {
                row.get(6).ok()
            } else {
                None
            };
            let identity_str: Option<String> = if col_count >= 8 {
                row.get(7).ok()
            } else {
                None
            };
            let source_toml_path: Option<String> = if col_count >= 9 {
                row.get(8).ok()
            } else {
                None
            };
            Ok((
                name,
                manifest_blob,
                state_str,
                created_str,
                session_id_str,
                identity_str,
                source_toml_path,
            ))
        });

        match result {
            Ok((
                name,
                manifest_blob,
                state_str,
                created_str,
                session_id_str,
                identity_str,
                source_toml_path,
            )) => {
                let mut manifest: librefang_types::agent::AgentManifest =
                    rmp_serde::from_slice(&manifest_blob).map_err(LibreFangError::serialization)?;
                // Migrate legacy hand agents: if manifest.is_hand is not set but
                // the agent looks like a hand (tags or name convention), fix it now.
                if !manifest.is_hand {
                    let looks_like_hand = manifest
                        .tags
                        .iter()
                        .any(|t: &String| t.starts_with("hand:"))
                        || name.contains(':');
                    if looks_like_hand {
                        manifest.is_hand = true;
                    }
                }
                let state =
                    serde_json::from_str(&state_str).map_err(LibreFangError::serialization)?;
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let session_id = session_id_str
                    .and_then(|s| uuid::Uuid::parse_str(&s).ok())
                    .map(librefang_types::agent::SessionId)
                    .unwrap_or_else(librefang_types::agent::SessionId::new);
                let identity = identity_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                let is_hand = manifest.is_hand;
                Ok(Some(AgentEntry {
                    id: agent_id,
                    name,
                    manifest,
                    state,
                    mode: Default::default(),
                    created_at,
                    last_active: Utc::now(),
                    parent: None,
                    children: vec![],
                    session_id,
                    source_toml_path: source_toml_path.map(std::path::PathBuf::from),
                    tags: vec![],
                    identity,
                    onboarding_completed: false,
                    onboarding_completed_at: None,
                    is_hand,
                    ..Default::default()
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::memory(e)),
        }
    }

    /// Remove an agent from the database, cascading to all agent-scoped tables.
    ///
    /// SQLite foreign keys are not enforced (`PRAGMA foreign_keys=OFF` default)
    /// and none of these tables declared `ON DELETE CASCADE`, so prior to
    /// this function rows keyed by `agent_id` would accumulate indefinitely
    /// after agent deletion. All DELETEs run inside a single transaction so
    /// a mid-cascade failure leaves no half-removed state.
    ///
    /// NOTE: most callers should use [`MemorySubstrate::remove_agent`]
    /// instead, which wraps sessions + structured cascade in one tx (#3501).
    /// This method does NOT touch `sessions` / `sessions_fts`.
    ///
    /// Tables covered: agents, kv_store, task_queue, memories,
    /// canonical_sessions, audit_entries, usage_events, entities, relations,
    /// approval_audit, prompt_versions, prompt_experiments (plus their
    /// dependent experiment_variants and experiment_metrics rows), and
    /// events via source_agent.
    pub fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let id = agent_id.0.to_string();
        let tx = conn
            .unchecked_transaction()
            .map_err(LibreFangError::memory)?;
        execute_structured_agent_deletes(&tx, &id)?;
        tx.commit().map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Load all agent entries from the database.
    ///
    /// Uses lenient deserialization (via `serde_compat`) to handle schema-mismatched
    /// fields gracefully. When an agent is loaded with lenient defaults, it is
    /// automatically re-saved to upgrade the stored blob. Duplicate agent names
    /// are deduplicated (first occurrence wins).
    pub fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        // Try with identity+session_id columns first, fall back gracefully
        let mut stmt = conn
            .prepare(
                "SELECT id, name, manifest, state, created_at, updated_at, session_id, identity, source_toml_path FROM agents",
            )
            .or_else(|_| {
                conn.prepare("SELECT id, name, manifest, state, created_at, updated_at, session_id, identity FROM agents")
            })
            .or_else(|_| {
                conn.prepare("SELECT id, name, manifest, state, created_at, updated_at, session_id FROM agents")
            })
            .or_else(|_| {
                conn.prepare("SELECT id, name, manifest, state, created_at, updated_at FROM agents")
            })
            .map_err(LibreFangError::memory)?;

        let col_count = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let manifest_blob: Vec<u8> = row.get(2)?;
                let state_str: String = row.get(3)?;
                let created_str: String = row.get(4)?;
                let session_id_str: Option<String> = if col_count >= 7 {
                    row.get(6).ok()
                } else {
                    None
                };
                let identity_str: Option<String> = if col_count >= 8 {
                    row.get(7).ok()
                } else {
                    None
                };
                let source_toml_path: Option<String> = if col_count >= 9 {
                    row.get(8).ok()
                } else {
                    None
                };
                Ok((
                    id_str,
                    name,
                    manifest_blob,
                    state_str,
                    created_str,
                    session_id_str,
                    identity_str,
                    source_toml_path,
                ))
            })
            .map_err(LibreFangError::memory)?;

        let mut agents = Vec::new();
        let mut seen_names = std::collections::HashSet::new();
        let mut repair_queue: Vec<(String, Vec<u8>, String)> = Vec::new();

        for row in rows {
            let (
                id_str,
                name,
                manifest_blob,
                state_str,
                created_str,
                session_id_str,
                identity_str,
                source_toml_path,
            ) = match row {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Skipping agent row with read error: {e}");
                    continue;
                }
            };

            // Deduplicate: skip agents with names we've already seen
            let name_lower = name.to_lowercase();
            if !seen_names.insert(name_lower) {
                tracing::info!(agent = %name, id = %id_str, "Skipping duplicate agent name");
                continue;
            }

            let agent_id = match uuid::Uuid::parse_str(&id_str).map(librefang_types::agent::AgentId)
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!(agent = %name, "Skipping agent with bad UUID '{id_str}': {e}");
                    continue;
                }
            };

            let mut manifest: librefang_types::agent::AgentManifest = match rmp_serde::from_slice(
                &manifest_blob,
            ) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        agent = %name, id = %id_str,
                        "Skipping agent with incompatible manifest (schema may have changed): {e}"
                    );
                    continue;
                }
            };

            // Migrate legacy hand agents: if manifest.is_hand is not set but the
            // agent looks like a hand (tags or name convention), fix it now so
            // the repaired blob persists the correct value.
            if !manifest.is_hand {
                let looks_like_hand = manifest
                    .tags
                    .iter()
                    .any(|t: &String| t.starts_with("hand:"))
                    || name.contains(':');
                if looks_like_hand {
                    manifest.is_hand = true;
                }
            }

            // Auto-repair: re-serialize with current schema and queue for update.
            // This upgrades the stored blob so future boots don't hit lenient paths.
            let new_blob =
                rmp_serde::to_vec_named(&manifest).map_err(LibreFangError::serialization)?;
            if new_blob != manifest_blob {
                tracing::debug!(
                    agent = %name, id = %id_str,
                    "Auto-repaired agent manifest (schema upgraded)"
                );
                repair_queue.push((id_str.clone(), new_blob, name.clone()));
            }

            let state = match serde_json::from_str(&state_str) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(agent = %name, "Skipping agent with bad state: {e}");
                    continue;
                }
            };
            let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let session_id = session_id_str
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
                .map(librefang_types::agent::SessionId)
                .unwrap_or_else(librefang_types::agent::SessionId::new);

            let identity = identity_str
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            let is_hand = manifest.is_hand;
            agents.push(AgentEntry {
                id: agent_id,
                name,
                manifest,
                state,
                mode: Default::default(),
                created_at,
                last_active: Utc::now(),
                parent: None,
                children: vec![],
                session_id,
                source_toml_path: source_toml_path.map(std::path::PathBuf::from),
                tags: vec![],
                identity,
                onboarding_completed: false,
                onboarding_completed_at: None,
                is_hand,
                ..Default::default()
            });
        }

        // Apply queued repairs (re-save upgraded blobs)
        for (id_str, new_blob, name) in repair_queue {
            if let Err(e) = conn.execute(
                "UPDATE agents SET manifest = ?1 WHERE id = ?2",
                rusqlite::params![new_blob, id_str],
            ) {
                tracing::warn!(agent = %name, "Failed to auto-repair agent blob: {e}");
            }
        }

        Ok(agents)
    }

    /// List all agents in the database.
    pub fn list_agents(&self) -> LibreFangResult<Vec<(String, String, String)>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT id, name, state FROM agents")
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(LibreFangError::memory)?;
        let mut agents = Vec::new();
        for row in rows {
            agents.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(agents)
    }
}

/// Run every structured-store DELETE for an agent inside the caller's
/// transaction. The single canonical list of agent-scoped tables;
/// [`StructuredStore::remove_agent`] and
/// [`crate::substrate::MemorySubstrate::remove_agent`] both share this
/// helper so a new agent-scoped table only has to be added in one place.
///
/// Subquery-scoped deletes (`experiment_metrics` / `experiment_variants`)
/// must run before `prompt_experiments` is cleared — otherwise the
/// `IN (SELECT ...)` matches nothing.
pub(crate) fn execute_structured_agent_deletes(
    tx: &rusqlite::Transaction<'_>,
    agent_id: &str,
) -> LibreFangResult<()> {
    for stmt in [
        "DELETE FROM experiment_metrics \
         WHERE experiment_id IN (SELECT id FROM prompt_experiments WHERE agent_id = ?1)",
        "DELETE FROM experiment_variants \
         WHERE experiment_id IN (SELECT id FROM prompt_experiments WHERE agent_id = ?1)",
        "DELETE FROM prompt_experiments WHERE agent_id = ?1",
        "DELETE FROM prompt_versions WHERE agent_id = ?1",
        "DELETE FROM approval_audit WHERE agent_id = ?1",
        "DELETE FROM audit_entries WHERE agent_id = ?1",
        "DELETE FROM usage_events WHERE agent_id = ?1",
        "DELETE FROM memories WHERE agent_id = ?1",
        "DELETE FROM canonical_sessions WHERE agent_id = ?1",
        "DELETE FROM kv_store WHERE agent_id = ?1",
        "DELETE FROM task_queue WHERE agent_id = ?1",
        "DELETE FROM entities WHERE agent_id = ?1",
        "DELETE FROM relations WHERE agent_id = ?1",
        "DELETE FROM events WHERE source_agent = ?1",
        "DELETE FROM agents WHERE id = ?1",
    ] {
        tx.execute(stmt, rusqlite::params![agent_id])
            .map_err(LibreFangError::memory)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> StructuredStore {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        StructuredStore::new(pool)
    }

    /// File-backed store with a multi-connection pool so two threads can
    /// genuinely contend for the SQLite write lock. An in-memory
    /// `:memory:` DB with `max_size(1)` cannot exercise the race the
    /// transactional `modify` fixes.
    fn setup_file_backed(path: &std::path::Path) -> StructuredStore {
        let pool = Pool::builder()
            .max_size(8)
            .build(SqliteConnectionManager::file(path))
            .unwrap();
        {
            let conn = pool.get().unwrap();
            conn.busy_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            run_migrations(&conn).unwrap();
        }
        StructuredStore::new(pool)
    }

    #[test]
    fn modify_concurrent_appends_lose_no_writes_5138() {
        // Regression for #5138: a plain get -> mutate -> set on a single
        // shared key drops one of two concurrent appends (last writer
        // wins). `modify` runs the RMW under BEGIN IMMEDIATE so both
        // appends must survive.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("kv.db");
        let store = setup_file_backed(&db);
        let agent = AgentId::new();
        store.set(agent, "arr", serde_json::json!([])).unwrap();

        let n = 24usize;
        std::thread::scope(|s| {
            for i in 0..n {
                let store = store.clone();
                s.spawn(move || {
                    store
                        .modify(agent, "arr", |cur| {
                            let mut v = match cur {
                                Some(serde_json::Value::Array(a)) => a,
                                _ => Vec::new(),
                            };
                            v.push(serde_json::json!(i));
                            Ok((serde_json::Value::Array(v), ()))
                        })
                        .unwrap();
                });
            }
        });

        let final_arr = match store.get(agent, "arr").unwrap() {
            Some(serde_json::Value::Array(a)) => a,
            other => panic!("expected array, got {other:?}"),
        };
        assert_eq!(
            final_arr.len(),
            n,
            "every concurrent append must persist; lost-update race not fixed"
        );
        let mut seen: Vec<u64> = final_arr.iter().map(|v| v.as_u64().unwrap()).collect();
        seen.sort_unstable();
        let expected: Vec<u64> = (0..n as u64).collect();
        assert_eq!(seen, expected, "no individual write may be clobbered");
    }

    #[test]
    fn modify_error_aborts_without_writing_5138() {
        let store = setup();
        let agent = AgentId::new();
        store.set(agent, "k", serde_json::json!("orig")).unwrap();
        let err = store.modify(agent, "k", |_cur| {
            Err::<(serde_json::Value, ()), _>(LibreFangError::InvalidInput("nope".into()))
        });
        assert!(matches!(err, Err(LibreFangError::InvalidInput(_))));
        // Row unchanged — the aborted tx must not have written.
        assert_eq!(
            store.get(agent, "k").unwrap(),
            Some(serde_json::json!("orig"))
        );
    }

    #[test]
    fn set_returning_existed_reports_atomic_transition_5138() {
        let store = setup();
        let agent = AgentId::new();
        // First write: key did not exist -> false (Created).
        assert!(!store
            .set_returning_existed(agent, "k", serde_json::json!(1))
            .unwrap());
        // Second write: key existed -> true (Updated).
        assert!(store
            .set_returning_existed(agent, "k", serde_json::json!(2))
            .unwrap());
        assert_eq!(store.get(agent, "k").unwrap(), Some(serde_json::json!(2)));
    }

    #[test]
    fn kv_value_size_cap_rejects_oversized_blob_5138() {
        let store = setup();
        let agent = AgentId::new();
        // Build a value whose serialized form exceeds MAX_KV_VALUE_BYTES.
        let big = "x".repeat(MAX_KV_VALUE_BYTES + 1);
        let v = serde_json::json!(big);
        let err = store.set(agent, "k", v.clone());
        assert!(
            matches!(err, Err(LibreFangError::InvalidInput(_))),
            "oversized set must be rejected, got {err:?}"
        );
        // The over-limit write must not have landed a row.
        assert_eq!(store.get(agent, "k").unwrap(), None);
        // Same guard via modify and set_returning_existed.
        assert!(matches!(
            store.modify(agent, "k", |_| Ok((v.clone(), ()))),
            Err(LibreFangError::InvalidInput(_))
        ));
        assert!(matches!(
            store.set_returning_existed(agent, "k", v),
            Err(LibreFangError::InvalidInput(_))
        ));
        // A value at exactly the limit is accepted.
        let ok_blob_str = "y".repeat(MAX_KV_VALUE_BYTES - 16);
        store
            .set(agent, "k2", serde_json::json!(ok_blob_str))
            .expect("value within the cap must be accepted");
    }

    #[test]
    fn test_kv_set_get() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .set(agent_id, "test_key", serde_json::json!("test_value"))
            .unwrap();
        let value = store.get(agent_id, "test_key").unwrap();
        assert_eq!(value, Some(serde_json::json!("test_value")));
    }

    #[test]
    fn test_kv_get_missing() {
        let store = setup();
        let agent_id = AgentId::new();
        let value = store.get(agent_id, "nonexistent").unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn test_kv_delete() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .set(agent_id, "to_delete", serde_json::json!(42))
            .unwrap();
        store.delete(agent_id, "to_delete").unwrap();
        let value = store.get(agent_id, "to_delete").unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn test_kv_update() {
        let store = setup();
        let agent_id = AgentId::new();
        store.set(agent_id, "key", serde_json::json!("v1")).unwrap();
        store.set(agent_id, "key", serde_json::json!("v2")).unwrap();
        let value = store.get(agent_id, "key").unwrap();
        assert_eq!(value, Some(serde_json::json!("v2")));
    }

    #[test]
    fn kv_namespace_guard_blocks_unauthorised_read() {
        use crate::namespace_acl::MemoryNamespaceGuard;
        use librefang_types::user_policy::UserMemoryAccess;

        let store = setup();
        let agent_id = AgentId::new();
        store
            .set(agent_id, "secret", serde_json::json!("treasure map"))
            .unwrap();

        // Guard with no read access at all.
        let guard = MemoryNamespaceGuard::new(UserMemoryAccess::default());
        let err = store.get_with_guard(agent_id, "secret", &guard);
        assert!(matches!(
            err,
            Err(librefang_types::error::LibreFangError::AuthDenied(_))
        ));
    }

    #[test]
    fn kv_namespace_guard_allows_matching_prefix() {
        use crate::namespace_acl::MemoryNamespaceGuard;
        use librefang_types::user_policy::UserMemoryAccess;

        let store = setup();
        let agent_id = AgentId::new();
        store
            .set(agent_id, "user_alice", serde_json::json!("hello"))
            .unwrap();

        let guard = MemoryNamespaceGuard::new(UserMemoryAccess {
            readable_namespaces: vec!["kv:user_*".into()],
            ..Default::default()
        });
        let v = store
            .get_with_guard(agent_id, "user_alice", &guard)
            .unwrap();
        assert_eq!(v, Some(serde_json::json!("hello")));

        // A different key prefix is denied.
        store
            .set(agent_id, "internal", serde_json::json!("nope"))
            .unwrap();
        assert!(matches!(
            store.get_with_guard(agent_id, "internal", &guard),
            Err(librefang_types::error::LibreFangError::AuthDenied(_))
        ));
    }

    #[test]
    fn kv_namespace_guard_delete_requires_flag() {
        use crate::namespace_acl::MemoryNamespaceGuard;
        use librefang_types::user_policy::UserMemoryAccess;

        let store = setup();
        let agent_id = AgentId::new();
        store.set(agent_id, "tmp", serde_json::json!(1)).unwrap();

        // Write access without delete_allowed → blocked.
        let no_delete = MemoryNamespaceGuard::new(UserMemoryAccess {
            readable_namespaces: vec!["*".into()],
            writable_namespaces: vec!["*".into()],
            delete_allowed: false,
            ..Default::default()
        });
        assert!(matches!(
            store.delete_with_guard(agent_id, "tmp", &no_delete),
            Err(librefang_types::error::LibreFangError::AuthDenied(_))
        ));

        // delete_allowed → succeeds.
        let with_delete = MemoryNamespaceGuard::new(UserMemoryAccess {
            readable_namespaces: vec!["*".into()],
            writable_namespaces: vec!["*".into()],
            delete_allowed: true,
            ..Default::default()
        });
        store
            .delete_with_guard(agent_id, "tmp", &with_delete)
            .unwrap();
        assert!(store.get(agent_id, "tmp").unwrap().is_none());
    }

    #[test]
    fn test_save_and_load_agent_source_toml_path() {
        let store = setup();
        let agent_id = AgentId::new();
        let entry = AgentEntry {
            id: agent_id,
            name: "test-agent".to_string(),
            manifest: librefang_types::agent::AgentManifest::default(),
            state: librefang_types::agent::AgentState::Running,
            mode: Default::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: librefang_types::agent::SessionId::new(),
            source_toml_path: Some(std::path::PathBuf::from("/tmp/test-agent/agent.toml")),
            tags: vec![],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        };

        store.save_agent(&entry).unwrap();
        let loaded = store.load_agent(agent_id).unwrap().unwrap();
        assert_eq!(
            loaded.source_toml_path,
            Some(std::path::PathBuf::from("/tmp/test-agent/agent.toml"))
        );
    }
}
