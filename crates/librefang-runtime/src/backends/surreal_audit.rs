//! SurrealDB-backed [`AuditStore`] implementation.
//!
//! Mirrors the rusqlite [`AuditLog`](crate::audit::AuditLog) but persists
//! through the shared [`librefang_storage::SurrealSession`] connection
//! pool. Schema is owned by the migration runner in
//! `librefang-storage::migrations` (see migration v1).
//!
//! ## Design notes
//!
//! - The Merkle tip and entry vector are kept in-memory (rebuilt at open
//!   time from the `audit_entries` table) so callers get the same `O(1)`
//!   `tip_hash()` / `len()` semantics as the rusqlite implementation.
//! - The optional external `anchor_path` is reused verbatim from the
//!   rusqlite design — see [`AuditLog`](crate::audit::AuditLog) for the
//!   threat model. Anchor I/O is implemented inline so this module does
//!   not have to expose private items from the rusqlite path.
//! - Async SurrealDB calls are bridged into the synchronous
//!   [`AuditStore`] trait surface via
//!   [`tokio::task::block_in_place`]. Callers must therefore drive the
//!   audit log on a multi-thread tokio runtime, same as the SurrealDB
//!   memory backend in [`librefang_memory::backends::surreal`].

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use librefang_storage::SurrealSession;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;
use tracing::{debug, warn};

use crate::audit::{AuditAction, AuditEntry};
use crate::storage_backends::AuditStore;

/// SurrealDB-backed implementation of [`AuditStore`].
///
/// Construct via [`SurrealAuditStore::open`]. The store rebuilds its
/// in-memory Merkle tip from the `audit_entries` table the first time it
/// is opened against an existing database, so daemon restarts are
/// transparent.
pub struct SurrealAuditStore {
    db: Surreal<Any>,
    /// In-memory mirror of `audit_entries`. The rusqlite implementation
    /// keeps the same shape; we mirror it for byte-for-byte trait parity
    /// with [`AuditLog`](crate::audit::AuditLog).
    entries: Mutex<Vec<AuditEntry>>,
    /// Hash of the most recent entry (or all-zeros when empty).
    tip: Mutex<String>,
    /// Optional external anchor file (see [`AuditLog`](crate::audit::AuditLog)
    /// for the rationale).
    anchor_path: Option<PathBuf>,
    /// Hash of the last entry that was dropped by a trim operation.
    /// When non-None, `verify_integrity` seeds the chain walk from this
    /// hash instead of ZERO_HASH so a trimmed log verifies correctly.
    chain_anchor: Mutex<Option<String>>,
}

/// Disk row layout for the `audit_entries` SurrealDB table.
///
/// Mirrors the columns declared in migration `001_audit_entries.surql`.
/// We store `action` as the same `Debug` representation as the rusqlite
/// path so the two implementations interoperate during migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuditRow {
    seq: u64,
    timestamp: String,
    agent_id: String,
    action: String,
    detail: String,
    outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    prev_hash: String,
    hash: String,
}

impl AuditRow {
    fn from_entry(entry: &AuditEntry) -> Self {
        Self {
            seq: entry.seq,
            timestamp: entry.timestamp.clone(),
            agent_id: entry.agent_id.clone(),
            action: entry.action.to_string(),
            detail: entry.detail.clone(),
            outcome: entry.outcome.clone(),
            user_id: entry.user_id.as_ref().map(|u| u.to_string()),
            channel: entry.channel.clone(),
            prev_hash: entry.prev_hash.clone(),
            hash: entry.hash.clone(),
        }
    }

    fn into_entry(self) -> AuditEntry {
        use librefang_types::agent::UserId;
        AuditEntry {
            seq: self.seq,
            timestamp: self.timestamp,
            agent_id: self.agent_id,
            action: parse_action(&self.action),
            detail: self.detail,
            outcome: self.outcome,
            user_id: self
                .user_id
                .as_deref()
                .and_then(|s| s.parse::<UserId>().ok()),
            channel: self.channel,
            prev_hash: self.prev_hash,
            hash: self.hash,
        }
    }
}

fn parse_action(s: &str) -> AuditAction {
    // The rusqlite path stores the `Debug` form (e.g. "ToolInvoke"). We
    // round-trip through the same vocabulary; unknown variants fall back
    // to `ConfigChange` so a forward-compat unfamiliar action does not
    // cause a load failure.
    match s {
        "ToolInvoke" => AuditAction::ToolInvoke,
        "CapabilityCheck" => AuditAction::CapabilityCheck,
        "AgentSpawn" => AuditAction::AgentSpawn,
        "AgentKill" => AuditAction::AgentKill,
        "AgentMessage" => AuditAction::AgentMessage,
        "MemoryAccess" => AuditAction::MemoryAccess,
        "FileAccess" => AuditAction::FileAccess,
        "NetworkAccess" => AuditAction::NetworkAccess,
        "ShellExec" => AuditAction::ShellExec,
        "AuthAttempt" => AuditAction::AuthAttempt,
        "WireConnect" => AuditAction::WireConnect,
        "ConfigChange" => AuditAction::ConfigChange,
        "DreamConsolidation" => AuditAction::DreamConsolidation,
        "UserLogin" => AuditAction::UserLogin,
        "RoleChange" => AuditAction::RoleChange,
        "PermissionDenied" => AuditAction::PermissionDenied,
        "BudgetExceeded" => AuditAction::BudgetExceeded,
        "RetentionTrim" => AuditAction::RetentionTrim,
        other => {
            warn!(
                action = other,
                "unknown audit action on load; defaulting to ConfigChange"
            );
            AuditAction::ConfigChange
        }
    }
}

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

impl SurrealAuditStore {
    /// Open the store against an existing [`SurrealSession`]. The session
    /// must already be signed in and have its namespace/database selected
    /// (the connection pool handles that at construction time).
    ///
    /// Rebuilds the in-memory chain from `audit_entries` so the
    /// `tip_hash()` / `len()` / `recent()` surface returns immediately
    /// without further DB round-trips, matching the rusqlite behaviour.
    ///
    /// # Errors
    ///
    /// Returns the SurrealDB-side error string if the initial table
    /// scan fails.
    pub fn open(session: &SurrealSession) -> Result<Self, String> {
        let db = session.client().clone();
        let entries = block_on(async { load_all(&db).await })?;
        let tip = entries
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_else(|| ZERO_HASH.to_string());
        debug!(
            count = entries.len(),
            tip_prefix = &tip[..8.min(tip.len())],
            "SurrealAuditStore opened"
        );
        Ok(Self {
            db,
            entries: Mutex::new(entries),
            tip: Mutex::new(tip),
            anchor_path: None,
            chain_anchor: Mutex::new(None),
        })
    }

    /// Attach an external anchor file. See
    /// [`AuditLog`](crate::audit::AuditLog) for the threat model.
    ///
    /// When opened with an anchor, the store seeds the file from the
    /// current tip on first use and refuses to verify if the tip and
    /// anchor diverge afterwards.
    #[must_use]
    pub fn with_anchor(mut self, anchor_path: PathBuf) -> Self {
        // Seed the anchor on first run so subsequent boots can detect
        // tampering even when upgrading an older deployment.
        let tip = self.tip.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let len = self.entries.lock().unwrap_or_else(|e| e.into_inner()).len() as u64;
        match read_anchor(&anchor_path) {
            Ok(None) => {
                if let Err(e) = write_anchor(&anchor_path, len, &tip) {
                    warn!(error = %e, path = ?anchor_path, "failed to seed audit anchor");
                }
            }
            Ok(Some(_)) | Err(_) => {
                // Existing or corrupt anchors are inspected by
                // `verify_integrity`, which is the canonical surface for
                // surfacing divergence to operators.
            }
        }
        self.anchor_path = Some(anchor_path);
        self
    }

    fn append_entry(
        &self,
        agent_id: &str,
        action: AuditAction,
        detail: &str,
        outcome: &str,
        user_id: Option<librefang_types::agent::UserId>,
        channel: Option<String>,
    ) -> AuditEntry {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut tip = self.tip.lock().unwrap_or_else(|e| e.into_inner());

        let seq = entries.len() as u64;
        let timestamp = Utc::now().to_rfc3339();
        let prev_hash = tip.clone();
        let hash = compute_hash(
            seq,
            &timestamp,
            agent_id,
            &action,
            detail,
            outcome,
            &prev_hash,
            user_id.as_ref(),
            channel.as_deref(),
        );

        let entry = AuditEntry {
            seq,
            timestamp,
            agent_id: agent_id.to_string(),
            action,
            detail: detail.to_string(),
            outcome: outcome.to_string(),
            user_id,
            channel,
            prev_hash,
            hash,
        };

        // Persist before mutating in-memory mirror so a write failure does
        // not leave the chain inconsistent.
        if let Err(e) = block_on(persist_entry(&self.db, &entry)) {
            warn!(error = %e, seq = entry.seq, "failed to persist audit entry");
        }

        if let Some(anchor) = &self.anchor_path {
            // The anchor stores the post-push count so `verify_integrity`
            // can compare it directly against `entries.len()` after the
            // append.
            let count = entries.len() as u64 + 1;
            if let Err(e) = write_anchor(anchor, count, &entry.hash) {
                warn!(error = %e, path = ?anchor, "failed to update audit anchor");
            }
        }

        *tip = entry.hash.clone();
        entries.push(entry.clone());
        entry
    }
}

impl AuditStore for SurrealAuditStore {
    fn record(&self, agent_id: &str, action: AuditAction, detail: &str, outcome: &str) {
        let _ = self.append_entry(agent_id, action, detail, outcome, None, None);
    }

    fn record_with_context(
        &self,
        agent_id: &str,
        action: AuditAction,
        detail: &str,
        outcome: &str,
        user_id: Option<librefang_types::agent::UserId>,
        channel: Option<String>,
    ) {
        let _ = self.append_entry(agent_id, action, detail, outcome, user_id, channel);
    }

    fn verify_integrity(&self) -> Result<(), String> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        // When a trim has dropped a prefix, the chain_anchor records the hash
        // of the last dropped entry. Seed the walk from it so trimmed logs verify.
        let anchor = self
            .chain_anchor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut prev = anchor.unwrap_or_else(|| ZERO_HASH.to_string());
        for entry in entries.iter() {
            let expected = compute_hash(
                entry.seq,
                &entry.timestamp,
                &entry.agent_id,
                &entry.action,
                &entry.detail,
                &entry.outcome,
                &prev,
                entry.user_id.as_ref(),
                entry.channel.as_deref(),
            );
            if expected != entry.hash {
                return Err(format!("hash mismatch at seq {}", entry.seq));
            }
            if entry.prev_hash != prev {
                return Err(format!("prev_hash mismatch at seq {}", entry.seq));
            }
            prev = entry.hash.clone();
        }

        if let Some(anchor) = &self.anchor_path {
            match read_anchor(anchor) {
                Ok(Some(record)) => {
                    let tip_hash = self.tip.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let tip_len = entries.len() as u64;
                    if record.seq != tip_len || record.hash != tip_hash {
                        return Err(format!(
                            "audit anchor mismatch: anchor says seq={} tip={} \
                             but DB has len={} tip={}",
                            record.seq, record.hash, tip_len, tip_hash
                        ));
                    }
                }
                Ok(None) => {
                    return Err(format!(
                        "audit anchor file {anchor:?} is missing — cannot \
                         verify tip integrity against external witness"
                    ));
                }
                Err(e) => return Err(format!("audit anchor unreadable: {e}")),
            }
        }

        Ok(())
    }

    fn tip_hash(&self) -> String {
        self.tip.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    fn is_empty(&self) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    fn recent(&self, n: usize) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let len = entries.len();
        let start = len.saturating_sub(n);
        entries[start..].to_vec()
    }

    fn since_seq(&self, cursor: u64) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let idx = entries.partition_point(|e| e.seq <= cursor);
        entries[idx..].to_vec()
    }

    fn prune(&self, retention_days: u32) -> usize {
        if retention_days == 0 {
            return 0;
        }
        // Same semantics as `AuditLog::prune`: best-effort, returns the
        // number of in-memory entries dropped. The persistent rows are
        // also deleted from the `audit_entries` table.
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let cutoff_str = cutoff.to_rfc3339();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let before = entries.len();
        entries.retain(|e| e.timestamp >= cutoff_str);
        let pruned = before - entries.len();

        // Mirror the prune to the persistent table. Failures are logged
        // but not propagated — the in-memory mirror is already correct
        // and we'd rather drift slightly than crash the runtime.
        if let Err(e) = block_on(async {
            let _ = self
                .db
                .query("DELETE FROM audit_entries WHERE timestamp < $cutoff RETURN NONE")
                .bind(("cutoff", cutoff_str))
                .await
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        }) {
            warn!(error = %e, "failed to prune persistent audit entries");
        }
        pruned
    }

    fn trim(
        &self,
        config: &librefang_types::config::AuditRetentionConfig,
        now: chrono::DateTime<Utc>,
    ) -> crate::audit::TrimReport {
        use crate::audit::TrimReport;
        use std::collections::BTreeMap;

        let mut dropped_by_action: BTreeMap<String, usize> = BTreeMap::new();
        let mut new_chain_anchor: Option<String> = None;

        // Pass 1: enforce max_in_memory_entries cap (same semantics as AuditLog::trim).
        // Drop the oldest entries from both the in-memory mirror and SurrealDB.
        {
            let cap = config.max_in_memory_entries.unwrap_or(0);
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let total = entries.len();
            if cap > 0 && total > cap {
                let drop_count = total - cap;
                for entry in &entries[..drop_count] {
                    *dropped_by_action
                        .entry(entry.action.to_string())
                        .or_insert(0) += 1;
                }
                let first_survivor_seq = entries[drop_count].seq;
                let last_dropped_hash = entries[drop_count - 1].hash.clone();
                new_chain_anchor = Some(last_dropped_hash.clone());
                entries.drain(..drop_count);
                drop(entries); // release lock before blocking DB call
                               // Update chain_anchor so verify_integrity seeds from the right hash.
                *self.chain_anchor.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(last_dropped_hash);
                if let Err(e) = block_on(async {
                    self.db
                        .query("DELETE FROM audit_entries WHERE seq < $seq RETURN NONE")
                        .bind(("seq", first_survivor_seq))
                        .await
                        .map_err(|e| e.to_string())?
                        .take::<Vec<serde_json::Value>>(0)
                        .map_err(|e| e.to_string())
                }) {
                    warn!(error = %e, "failed to trim capped audit entries from SurrealDB");
                }
            }
        }

        // Pass 2: per-action retention — delete entries whose action exceeds its window.
        for (action_str, &days) in &config.retention_days_by_action {
            let cutoff = (now - chrono::Duration::days(i64::from(days))).to_rfc3339();
            let action_str = action_str.clone();
            let result = block_on(async {
                self.db
                    .query(
                        "SELECT count() AS n FROM audit_entries \
                         WHERE action = $action AND timestamp < $cutoff GROUP ALL",
                    )
                    .bind(("action", action_str.clone()))
                    .bind(("cutoff", cutoff.clone()))
                    .await
                    .map_err(|e| e.to_string())?
                    .take::<Vec<serde_json::Value>>(0)
                    .map_err(|e| e.to_string())
            });
            let n = result
                .ok()
                .and_then(|rows| rows.into_iter().next().and_then(|v| v.get("n")?.as_u64()))
                .unwrap_or(0) as usize;

            if n > 0 {
                if let Err(e) = block_on(async {
                    self.db
                        .query("DELETE FROM audit_entries WHERE action = $action AND timestamp < $cutoff RETURN NONE")
                        .bind(("action", action_str.clone()))
                        .bind(("cutoff", cutoff.clone()))
                        .await
                        .map_err(|e| e.to_string())?
                        .take::<Vec<serde_json::Value>>(0)
                        .map_err(|e| e.to_string())
                }) {
                    warn!(error = %e, action = %action_str, "failed to trim persistent audit entries by action");
                } else {
                    dropped_by_action.insert(action_str, n);
                }
            }
        }

        let total_dropped: usize = dropped_by_action.values().sum();

        // Rebuild in-memory mirror after DB trim.
        let refreshed = block_on(load_all(&self.db));
        match refreshed {
            Ok(entries) => {
                new_chain_anchor = entries.last().map(|e| e.hash.clone());
                let tip = entries
                    .last()
                    .map(|e| e.hash.clone())
                    .unwrap_or_else(|| ZERO_HASH.to_string());
                *self.entries.lock().unwrap_or_else(|e| e.into_inner()) = entries;
                *self.tip.lock().unwrap_or_else(|e| e.into_inner()) = tip;
            }
            Err(e) => warn!(error = %e, "failed to reload audit entries after trim"),
        }

        TrimReport {
            dropped_by_action,
            total_dropped,
            new_chain_anchor,
        }
    }

    fn anchor_path(&self) -> Option<&std::path::Path> {
        self.anchor_path.as_deref()
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_hash(
    seq: u64,
    timestamp: &str,
    agent_id: &str,
    action: &AuditAction,
    detail: &str,
    outcome: &str,
    prev_hash: &str,
    user_id: Option<&librefang_types::agent::UserId>,
    channel: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_string().as_bytes());
    hasher.update(timestamp.as_bytes());
    hasher.update(agent_id.as_bytes());
    hasher.update(action.to_string().as_bytes());
    hasher.update(detail.as_bytes());
    hasher.update(outcome.as_bytes());
    if let Some(uid) = user_id {
        hasher.update(b"\x1fuser_id=");
        hasher.update(uid.0.as_bytes());
    }
    if let Some(ch) = channel {
        hasher.update(b"\x1fchannel=");
        hasher.update(ch.as_bytes());
    }
    hasher.update(prev_hash.as_bytes());
    hex::encode(hasher.finalize())
}

async fn persist_entry(db: &Surreal<Any>, entry: &AuditEntry) -> Result<(), String> {
    let row = AuditRow::from_entry(entry);
    let row_json = serde_json::to_value(&row).map_err(|e| e.to_string())?;
    let id = format!("seq{}", entry.seq);
    let _: Option<serde_json::Value> = db
        .upsert(("audit_entries", id.as_str()))
        .content(row_json)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn load_all(db: &Surreal<Any>) -> Result<Vec<AuditEntry>, String> {
    let rows: Vec<serde_json::Value> = db
        .query(
            "SELECT seq, timestamp, agent_id, action, detail, outcome, \
             user_id, channel, prev_hash, hash \
             FROM audit_entries ORDER BY seq ASC",
        )
        .await
        .map_err(|e| e.to_string())?
        .take(0)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        match serde_json::from_value::<AuditRow>(row) {
            Ok(r) => out.push(r.into_entry()),
            Err(e) => warn!(error = %e, "skipping malformed audit row"),
        }
    }
    Ok(out)
}

/// Bridge a future onto the active tokio runtime from a synchronous
/// trait method. Falls back to a temporary runtime when called from
/// a plain `#[test]` thread that has no active Tokio context.
fn block_on<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build temporary tokio runtime")
            .block_on(fut),
    }
}

// ── Anchor I/O ─────────────────────────────────────────────────────────
//
// The anchor file format mirrors `librefang-runtime::audit`: a single
// `<seq> <hex-hash>\n` line. We keep the I/O local rather than re-using
// the rusqlite path's private helpers so this module compiles even when
// the `sqlite-backend` feature is off.

/// A tip hash recovered from the anchor file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnchorRecord {
    seq: u64,
    hash: String,
}

fn write_anchor(path: &Path, seq: u64, hash: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("anchor.tmp");
    std::fs::write(&tmp, format!("{seq} {hash}\n"))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn read_anchor(path: &Path) -> Result<Option<AnchorRecord>, String> {
    match std::fs::read_to_string(path) {
        Ok(body) => {
            let line = body.trim();
            if line.is_empty() {
                return Err("anchor file is empty".to_string());
            }
            let mut parts = line.splitn(2, char::is_whitespace);
            let seq_str = parts.next().ok_or("anchor file has no seq column")?;
            let hash = parts
                .next()
                .ok_or("anchor file has no hash column")?
                .trim()
                .to_string();
            let seq = seq_str
                .parse::<u64>()
                .map_err(|e| format!("anchor seq is not a u64: {e}"))?;
            if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("anchor hash is not 64 hex chars: {hash:?}"));
            }
            Ok(Some(AnchorRecord { seq, hash }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read audit anchor: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_storage::{StorageBackendKind, StorageConfig, SurrealConnectionPool};
    use tempfile::tempdir;

    async fn open_store_with_pool(
        pool: &SurrealConnectionPool,
        path: &std::path::Path,
    ) -> SurrealAuditStore {
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(path.to_path_buf()),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = pool.open(&cfg).await.expect("open session");
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .expect("migrations");
        SurrealAuditStore::open(&session).expect("open store")
    }

    async fn open_store(path: &std::path::Path) -> SurrealAuditStore {
        let pool = SurrealConnectionPool::new();
        open_store_with_pool(&pool, path).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn append_and_verify_chain() {
        let dir = tempdir().unwrap();
        let store = open_store(&dir.path().join("audit.surreal")).await;

        store.record("agent-1", AuditAction::ToolInvoke, "read /etc/hosts", "ok");
        store.record("agent-1", AuditAction::ShellExec, "ls", "ok");
        store.record("agent-2", AuditAction::AgentSpawn, "helper", "ok");

        assert_eq!(store.len(), 3);
        store.verify_integrity().expect("chain integrity");

        let recent = store.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[1].agent_id, "agent-2");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reload_picks_up_persisted_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.surreal");
        // Share one pool across both opens so the embedded RocksDB lock is
        // held by a single transport (the pool caches embedded URLs).
        let pool = SurrealConnectionPool::new();

        {
            let store = open_store_with_pool(&pool, &path).await;
            store.record("agent-x", AuditAction::ToolInvoke, "first", "ok");
            store.record("agent-x", AuditAction::ToolInvoke, "second", "ok");
            assert_eq!(store.len(), 2);
        }

        let store2 = open_store_with_pool(&pool, &path).await;
        assert_eq!(store2.len(), 2);
        store2
            .verify_integrity()
            .expect("chain integrity after reload");
        store2.record("agent-y", AuditAction::AgentMessage, "third", "ok");
        assert_eq!(store2.len(), 3);
        store2
            .verify_integrity()
            .expect("chain integrity post-append");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn anchor_round_trips_through_filesystem() {
        let dir = tempdir().unwrap();
        let store = open_store(&dir.path().join("audit.surreal")).await;
        let anchor_path = dir.path().join("audit.anchor");
        let store = store.with_anchor(anchor_path.clone());

        store.record("agent-1", AuditAction::ToolInvoke, "first", "ok");
        store.verify_integrity().expect("anchor agrees with tip");

        // Tamper: rewrite the anchor with bogus tip; verify_integrity must
        // refuse.
        write_anchor(&anchor_path, 99, &"a".repeat(64)).unwrap();
        let err = store
            .verify_integrity()
            .expect_err("anchor mismatch must be rejected");
        assert!(
            err.contains("audit anchor mismatch"),
            "unexpected error: {err}"
        );
    }
}
