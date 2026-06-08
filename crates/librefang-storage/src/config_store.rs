//! System-scoped configuration store.
//!
//! Backs the database-backed configuration store
//! (`phase-9-config-store-migration`). Runtime-mutable, UI-editable
//! configuration — MCP server registrations, the default model, provider URLs,
//! and the `/api/config/set` allowlisted subset — lives in the `config_store`
//! table (migration v31) instead of `config.toml`, so the web UI can persist
//! changes even when `config.toml` is mounted read-only from a Kubernetes
//! ConfigMap.
//!
//! ## Scope
//!
//! This store is **system-scoped** (one row per `key`), unlike the
//! agent-scoped [`crate::migrations`] `kv_store` table (one row per
//! `(agent_id, key)`). It is NOT a secrets store — credentials, auth config,
//! and the storage-connection settings stay in file + env (see the phase-9
//! assessment §3b for the bootstrap-paradox and security rationale).
//!
//! ## Conflict model
//!
//! Each entry carries provenance and a content hash so the seed/merge logic
//! (C-003) can reconcile file defaults against UI edits **without ever reading
//! a file mtime** — a ConfigMap-projected file's mtime changes on every kubelet
//! sync even when its content is identical, so mtime is not a change signal.
//!
//! - [`ConfigSource::Bootstrap`] — seeded from `config.toml` defaults.
//! - [`ConfigSource::Runtime`] — written by the UI / API.
//! - `content_hash` — hash of the bootstrap value last seen for this key.
//! - `revision` — operator-bumped bootstrap revision; a strictly-greater
//!   bootstrap revision is the only way a file value overrides an existing row
//!   regardless of source.
//!
//! ## Backends
//!
//! The [`SurrealConfigStore`] implementation works for both embedded and remote
//! SurrealDB — both are a `Surreal<Any>` handle, so one implementation covers
//! both. A `sqlite-backend` parity implementation is intentionally deferred
//! (best-effort); the default `surreal-backend` build is the load-bearing path.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{StorageError, StorageResult};

/// Name of the SurrealDB table backing the config store (migration v31).
pub const CONFIG_STORE_TABLE: &str = "config_store";

/// Provenance of a stored configuration value.
///
/// Serialises to the lowercase strings stored in the `source` column
/// (`"bootstrap"` / `"runtime"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    /// Seeded from `config.toml` bootstrap defaults.
    Bootstrap,
    /// Written at runtime by the UI / API.
    Runtime,
}

impl ConfigSource {
    /// String form stored in the `source` column.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Runtime => "runtime",
        }
    }

    /// Parse the string form stored in the `source` column.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backend`] when the stored value is neither
    /// `"bootstrap"` nor `"runtime"`.
    pub fn parse(raw: &str) -> StorageResult<Self> {
        match raw {
            "bootstrap" => Ok(Self::Bootstrap),
            "runtime" => Ok(Self::Runtime),
            other => Err(StorageError::Backend(format!(
                "config_store: unknown source '{other}' (expected bootstrap|runtime)"
            ))),
        }
    }
}

/// A single configuration entry as stored in / read from the config store.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigEntry {
    /// Dotted config key (e.g. `mcp_servers`, `default_model`, `ui.theme`).
    pub key: String,
    /// The config value as arbitrary JSON (object, array, or scalar).
    pub value: serde_json::Value,
    /// Whether this value was seeded from a file (`Bootstrap`) or written by
    /// the UI / API (`Runtime`).
    pub source: ConfigSource,
    /// Hash of the bootstrap value last seen for this key (see module docs).
    pub content_hash: String,
    /// Operator-bumped bootstrap revision (see module docs).
    pub revision: i64,
    /// RFC-3339 timestamp of the last write, stamped by the store.
    pub updated_at: String,
}

/// Read/write interface over the system config store.
///
/// `list` MUST return entries in a deterministic order (sorted by `key`) so
/// that anything derived from the store and rendered into an LLM prompt stays
/// byte-stable across runs (repo invariant #3298 — see C-007).
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// Fetch a single entry by exact key. Returns `None` if absent.
    async fn get(&self, key: &str) -> StorageResult<Option<ConfigEntry>>;

    /// List all entries whose key starts with `prefix`, sorted by key.
    /// Pass `""` to list every entry.
    async fn list(&self, prefix: &str) -> StorageResult<Vec<ConfigEntry>>;

    /// Insert or replace the entry for `key`. `updated_at` is stamped by the
    /// store; the returned entry reflects what was persisted.
    async fn upsert(
        &self,
        key: &str,
        value: serde_json::Value,
        source: ConfigSource,
        content_hash: &str,
        revision: i64,
    ) -> StorageResult<ConfigEntry>;

    /// Delete the entry for `key`. Returns `true` if a row was removed.
    async fn delete(&self, key: &str) -> StorageResult<bool>;
}

/// Canonical, order-independent hash of a config value.
///
/// Object keys are sorted recursively before hashing so that two values that
/// differ only in JSON key order produce the same hash. This is the comparator
/// the seed/merge logic uses in place of a file mtime.
#[must_use]
pub fn content_hash(value: &serde_json::Value) -> String {
    let mut canon = String::new();
    canonicalize(value, &mut canon);
    let mut hasher = Sha256::new();
    hasher.update(canon.as_bytes());
    hex::encode(hasher.finalize())
}

fn canonicalize(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                // `serde_json::to_string` on a string never fails.
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                canonicalize(&map[*k], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(arr) => {
            out.push('[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonicalize(v, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

#[cfg(feature = "surreal-backend")]
pub use surreal_impl::SurrealConfigStore;

#[cfg(feature = "surreal-backend")]
mod surreal_impl {
    use super::{ConfigEntry, ConfigSource, ConfigStore, CONFIG_STORE_TABLE};
    use crate::error::{StorageError, StorageResult};
    use crate::pool::SurrealSession;
    use async_trait::async_trait;
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use surrealdb::{engine::any::Any, Surreal};

    /// SurrealDB-backed [`ConfigStore`]. Works for embedded and remote alike —
    /// both are a `Surreal<Any>` handle.
    #[derive(Clone)]
    pub struct SurrealConfigStore {
        db: Surreal<Any>,
    }

    impl SurrealConfigStore {
        /// Open a config store over an existing session. The session's
        /// namespace/database are re-selected on an owned client clone so the
        /// store can be stored in a struct and queried independently.
        ///
        /// # Errors
        ///
        /// Returns [`StorageError::Backend`] if re-selecting the session's
        /// namespace/database fails.
        pub async fn open(session: &SurrealSession) -> StorageResult<Self> {
            Ok(Self {
                db: session.clone_db().await?,
            })
        }

        /// Deterministic record id for a key (hex SHA-256). Keeps one row per
        /// logical key regardless of dots / characters in the key, and makes
        /// upsert/delete idempotent without a secondary lookup.
        fn record_id(key: &str) -> String {
            let mut hasher = Sha256::new();
            hasher.update(key.as_bytes());
            hex::encode(hasher.finalize())
        }
    }

    /// Internal row shape as stored. `value` is enveloped as `{ "data": <v> }`
    /// because the `value` column is `option<object>` (migration v31) and the
    /// real config value may be an array or scalar, not just an object.
    #[derive(Deserialize)]
    struct StoredRow {
        key: String,
        value: Option<serde_json::Value>,
        source: String,
        content_hash: String,
        revision: i64,
        updated_at: String,
    }

    impl StoredRow {
        fn from_json(row: serde_json::Value) -> StorageResult<Self> {
            serde_json::from_value(row)
                .map_err(|e| StorageError::Backend(format!("config_store: malformed row: {e}")))
        }

        fn into_entry(self) -> StorageResult<ConfigEntry> {
            let value = self
                .value
                .and_then(|env| env.get("data").cloned())
                .unwrap_or(serde_json::Value::Null);
            Ok(ConfigEntry {
                key: self.key,
                value,
                source: ConfigSource::parse(&self.source)?,
                content_hash: self.content_hash,
                revision: self.revision,
                updated_at: self.updated_at,
            })
        }
    }

    fn now_rfc3339() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    #[async_trait]
    impl ConfigStore for SurrealConfigStore {
        async fn get(&self, key: &str) -> StorageResult<Option<ConfigEntry>> {
            let q = format!(
                "SELECT key, value, source, content_hash, revision, updated_at \
                 FROM {CONFIG_STORE_TABLE} WHERE key = $key LIMIT 1"
            );
            // `take` requires `SurrealValue`; take JSON then deserialise (same
            // pattern as the migration runner's `load_applied`).
            let mut rows: Vec<serde_json::Value> = self
                .db
                .query(q)
                .bind(("key", key.to_string()))
                .await
                .map_err(|e| StorageError::Backend(e.to_string()))?
                .take(0)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            match rows.pop() {
                Some(row) => Ok(Some(StoredRow::from_json(row)?.into_entry()?)),
                None => Ok(None),
            }
        }

        async fn list(&self, prefix: &str) -> StorageResult<Vec<ConfigEntry>> {
            // ORDER BY key for deterministic ordering (#3298). `starts_with`
            // with an empty prefix matches every row.
            let q = format!(
                "SELECT key, value, source, content_hash, revision, updated_at \
                 FROM {CONFIG_STORE_TABLE} \
                 WHERE string::starts_with(key, $prefix) ORDER BY key ASC"
            );
            let rows: Vec<serde_json::Value> = self
                .db
                .query(q)
                .bind(("prefix", prefix.to_string()))
                .await
                .map_err(|e| StorageError::Backend(e.to_string()))?
                .take(0)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            rows.into_iter()
                .map(|row| StoredRow::from_json(row)?.into_entry())
                .collect()
        }

        async fn upsert(
            &self,
            key: &str,
            value: serde_json::Value,
            source: ConfigSource,
            content_hash: &str,
            revision: i64,
        ) -> StorageResult<ConfigEntry> {
            let updated_at = now_rfc3339();
            let envelope = serde_json::json!({ "data": value });
            let row = serde_json::json!({
                "key": key,
                "value": envelope,
                "source": source.as_str(),
                "content_hash": content_hash,
                "revision": revision,
                "updated_at": updated_at,
            });
            let id = Self::record_id(key);
            let _: Option<serde_json::Value> = self
                .db
                .upsert((CONFIG_STORE_TABLE, id.as_str()))
                .content(row)
                .await
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(ConfigEntry {
                key: key.to_string(),
                value,
                source,
                content_hash: content_hash.to_string(),
                revision,
                updated_at,
            })
        }

        async fn delete(&self, key: &str) -> StorageResult<bool> {
            let id = Self::record_id(key);
            let deleted: Option<serde_json::Value> = self
                .db
                .delete((CONFIG_STORE_TABLE, id.as_str()))
                .await
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(deleted.is_some())
        }
    }
}

#[cfg(all(test, feature = "surreal-backend"))]
mod tests {
    use super::*;
    use crate::config::{StorageBackendKind, StorageConfig};
    use crate::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
    use crate::pool::SurrealConnectionPool;
    use tempfile::tempdir;

    async fn open_store(dir: &std::path::Path) -> SurrealConfigStore {
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(dir.join("cfg.surreal")),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let pool = SurrealConnectionPool::new();
        let session = pool.open(&cfg).await.expect("open session");
        apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
            .await
            .expect("migrations");
        SurrealConfigStore::open(&session)
            .await
            .expect("open store")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn round_trips_upsert_get_list_delete() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path()).await;

        // get on empty
        assert!(store.get("missing").await.unwrap().is_none());

        // upsert an array value (mcp_servers shape) — exercises the envelope.
        let servers = serde_json::json!([{ "name": "seq-thinking", "timeout_secs": 30 }]);
        let h = content_hash(&servers);
        let entry = store
            .upsert("mcp_servers", servers.clone(), ConfigSource::Runtime, &h, 0)
            .await
            .unwrap();
        assert_eq!(entry.key, "mcp_servers");
        assert_eq!(entry.source, ConfigSource::Runtime);

        // get round-trips the array value intact
        let got = store.get("mcp_servers").await.unwrap().expect("present");
        assert_eq!(got.value, servers);
        assert_eq!(got.content_hash, h);
        assert_eq!(got.source, ConfigSource::Runtime);

        // upsert is idempotent-by-key (replace, not duplicate); also a scalar value
        let scalar = serde_json::json!("info");
        store
            .upsert(
                "log_level",
                scalar.clone(),
                ConfigSource::Bootstrap,
                &content_hash(&scalar),
                2,
            )
            .await
            .unwrap();
        store
            .upsert(
                "mcp_servers",
                servers.clone(),
                ConfigSource::Bootstrap,
                &h,
                1,
            )
            .await
            .unwrap();

        // list is sorted by key and reflects the replace (no duplicate mcp_servers)
        let all = store.list("").await.unwrap();
        assert_eq!(all.len(), 2, "replace must not duplicate the row");
        assert_eq!(all[0].key, "log_level");
        assert_eq!(all[1].key, "mcp_servers");
        assert_eq!(all[1].revision, 1, "second upsert replaced the first");
        assert_eq!(all[1].source, ConfigSource::Bootstrap);
        assert_eq!(all[0].value, scalar);

        // prefix filter
        let mcp = store.list("mcp").await.unwrap();
        assert_eq!(mcp.len(), 1);
        assert_eq!(mcp[0].key, "mcp_servers");

        // delete
        assert!(store.delete("mcp_servers").await.unwrap());
        assert!(
            !store.delete("mcp_servers").await.unwrap(),
            "second delete is a no-op"
        );
        assert!(store.get("mcp_servers").await.unwrap().is_none());
        assert_eq!(store.list("").await.unwrap().len(), 1);
    }

    #[test]
    fn content_hash_is_object_key_order_independent() {
        let a = serde_json::json!({ "x": 1, "y": [2, 3], "z": { "a": 1, "b": 2 } });
        let b = serde_json::json!({ "z": { "b": 2, "a": 1 }, "y": [2, 3], "x": 1 });
        assert_eq!(content_hash(&a), content_hash(&b));
        // array order DOES matter
        let c = serde_json::json!({ "y": [3, 2] });
        let d = serde_json::json!({ "y": [2, 3] });
        assert_ne!(content_hash(&c), content_hash(&d));
    }

    #[test]
    fn config_source_round_trips() {
        assert_eq!(
            ConfigSource::parse("bootstrap").unwrap(),
            ConfigSource::Bootstrap
        );
        assert_eq!(
            ConfigSource::parse("runtime").unwrap(),
            ConfigSource::Runtime
        );
        assert!(ConfigSource::parse("nonsense").is_err());
        assert_eq!(ConfigSource::Bootstrap.as_str(), "bootstrap");
    }

    /// Determinism guard (#3298): `list()` returns entries sorted by key
    /// regardless of insertion order. Config values can reach an LLM prompt, so
    /// any list derived from the store must be byte-stable across runs — the
    /// `ORDER BY key` in the impl is load-bearing, not cosmetic.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_is_sorted_by_key_regardless_of_insertion_order() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path()).await;

        // Insert in deliberately non-sorted order.
        for key in ["z_last", "a_first", "m_middle"] {
            let v = serde_json::json!({ "k": key });
            store
                .upsert(
                    key,
                    v.clone(),
                    ConfigSource::Bootstrap,
                    &content_hash(&v),
                    0,
                )
                .await
                .unwrap();
        }

        // `ORDER BY key` makes this independent of RocksDB iteration order:
        // drop the ORDER BY and the rows would come back in insertion order
        // (z, a, m), failing this assertion.
        let keys: Vec<String> = store
            .list("")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.key)
            .collect();
        assert_eq!(
            keys,
            vec!["a_first", "m_middle", "z_last"],
            "list() must be sorted by key (insertion-order-independent)"
        );

        // A prefix query is likewise sorted.
        let m = store.list("m").await.unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].key, "m_middle");
    }
}
