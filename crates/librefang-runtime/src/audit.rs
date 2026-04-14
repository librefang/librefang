//! Merkle hash chain audit trail for security-critical actions.
//!
//! Every auditable event is appended to an append-only log where each entry
//! contains the SHA-256 hash of its own contents concatenated with the hash of
//! the previous entry, forming a tamper-evident chain (similar to a blockchain).
//!
//! When a database connection is provided (`with_db`), entries are persisted to
//! the `audit_entries` table (schema V8) so the trail survives daemon restarts.

use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

/// Categories of auditable actions within the agent runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditAction {
    ToolInvoke,
    CapabilityCheck,
    AgentSpawn,
    AgentKill,
    AgentMessage,
    MemoryAccess,
    FileAccess,
    NetworkAccess,
    ShellExec,
    AuthAttempt,
    WireConnect,
    ConfigChange,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A single entry in the Merkle hash chain audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Monotonically increasing sequence number (0-indexed).
    pub seq: u64,
    /// ISO-8601 timestamp of when this entry was recorded.
    pub timestamp: String,
    /// The agent that triggered (or is the subject of) this action.
    pub agent_id: String,
    /// The category of action being audited.
    pub action: AuditAction,
    /// Free-form detail about the action (e.g. tool name, file path).
    pub detail: String,
    /// The outcome of the action (e.g. "ok", "denied", an error message).
    pub outcome: String,
    /// SHA-256 hash of the previous entry (or all-zeros for the genesis).
    pub prev_hash: String,
    /// SHA-256 hash of this entry's content concatenated with `prev_hash`.
    pub hash: String,
}

/// Computes the SHA-256 hash for a single audit entry from its fields.
fn compute_entry_hash(
    seq: u64,
    timestamp: &str,
    agent_id: &str,
    action: &AuditAction,
    detail: &str,
    outcome: &str,
    prev_hash: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_string().as_bytes());
    hasher.update(timestamp.as_bytes());
    hasher.update(agent_id.as_bytes());
    hasher.update(action.to_string().as_bytes());
    hasher.update(detail.as_bytes());
    hasher.update(outcome.as_bytes());
    hasher.update(prev_hash.as_bytes());
    hex::encode(hasher.finalize())
}

/// An append-only, tamper-evident audit log using a Merkle hash chain.
///
/// Thread-safe — all access is serialised through internal mutexes.
/// Optionally backed by SQLite for persistence across daemon restarts,
/// and optionally anchored to an external file so a full rewrite of the
/// SQLite table can be detected on the next verification.
///
/// # Threat model — the anchor file
///
/// The in-DB Merkle chain alone is only self-consistent: an attacker with
/// write access to `audit_entries` can delete every row, insert a
/// fabricated history, and recompute every hash from the genesis sentinel
/// forward — `verify_integrity` returns `Ok` because it has nothing to
/// compare the tip against. The anchor file closes that gap by storing
/// the latest `seq:hash` outside the SQLite row store, so the chain must
/// agree with an external witness the attacker would have to tamper with
/// separately. For stronger guarantees point `anchor_path` at a location
/// the daemon can write to but unprivileged code cannot (a chmod-0400
/// file owned by a different user, a systemd `ReadOnlyPaths=` mount, an
/// NFS share, or a pipe to `logger`).
pub struct AuditLog {
    entries: Mutex<Vec<AuditEntry>>,
    tip: Mutex<String>,
    /// Optional database connection for persistent storage.
    db: Option<Arc<Mutex<Connection>>>,
    /// Optional filesystem path where the latest `seq:hash` pair is
    /// atomically rewritten after every `record()`. Startup and
    /// `verify_integrity()` compare the in-DB tip against the anchor's
    /// contents and refuse to return success if they diverge.
    anchor_path: Option<std::path::PathBuf>,
}

/// On-disk format of the audit anchor file: `<seq> <hex-hash>\n`. Parsed
/// by [`AuditLog::read_anchor`]. Kept deliberately minimal so a human
/// inspecting the file (or a log collector) can read it directly.
fn format_anchor_line(seq: u64, hash: &str) -> String {
    format!("{seq} {hash}\n")
}

/// A tip hash recovered from the anchor file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnchorRecord {
    seq: u64,
    hash: String,
}

impl AuditLog {
    /// Creates a new empty audit log (in-memory only, no persistence).
    ///
    /// The initial tip hash is 64 zero characters (the "genesis" sentinel).
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            tip: Mutex::new("0".repeat(64)),
            db: None,
            anchor_path: None,
        }
    }

    /// Atomically rewrite the anchor file with the given `seq:hash`.
    ///
    /// Uses `<path>.tmp` + rename so a crash mid-write never leaves a
    /// truncated anchor that would fail startup verification.
    fn write_anchor(path: &std::path::Path, seq: u64, hash: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            // Best-effort; if the parent exists already this is a no-op.
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("anchor.tmp");
        std::fs::write(&tmp, format_anchor_line(seq, hash))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load the `AnchorRecord` stored in `path`, or `None` if the file
    /// does not exist. Malformed contents are reported as `Err` so
    /// verification can fail closed rather than silently treating a
    /// corrupted anchor as "no anchor".
    fn read_anchor(path: &std::path::Path) -> Result<Option<AnchorRecord>, String> {
        match std::fs::read_to_string(path) {
            Ok(body) => {
                let line = body.lines().next().unwrap_or("").trim();
                if line.is_empty() {
                    return Ok(None);
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

    /// Creates an audit log backed by a database connection **and** an
    /// external tip-anchor file. See the struct-level docs for why the
    /// anchor matters: a DB-only chain is self-consistent but cannot
    /// detect a full rewrite of `audit_entries`, while the anchor closes
    /// that gap by storing the latest `seq:hash` outside SQLite.
    ///
    /// On construction:
    ///  1. Entries are loaded from SQLite as before.
    ///  2. The Merkle chain is re-verified.
    ///  3. The anchor file (if it exists) is compared against the in-DB
    ///     tip. If they disagree, a loud error is logged — the daemon
    ///     still comes up, because refusing to start would be worse than
    ///     surfacing the integrity failure via `/api/audit/verify`, but
    ///     subsequent `verify_integrity()` calls will return `Err`.
    ///  4. If the DB has rows but no anchor exists yet, the anchor is
    ///     created from the current tip so future rewrites can be
    ///     detected even when upgrading an older deployment.
    pub fn with_db_anchored(conn: Arc<Mutex<Connection>>, anchor_path: std::path::PathBuf) -> Self {
        let mut log = Self::with_db(conn);
        log.anchor_path = Some(anchor_path.clone());

        // Compare against the anchor file (if any) and warn loudly on
        // divergence. The call to `verify_integrity` below will also
        // return `Err` in that case so `/api/audit/verify` surfaces it.
        match Self::read_anchor(&anchor_path) {
            Ok(Some(record)) => {
                let current_tip = log.tip.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let current_seq =
                    log.entries.lock().unwrap_or_else(|e| e.into_inner()).len() as u64;
                if record.hash != current_tip {
                    tracing::error!(
                        anchor_seq = record.seq,
                        anchor_hash = %record.hash,
                        db_seq = current_seq,
                        db_tip = %current_tip,
                        "Audit anchor MISMATCH on boot — SQLite audit_entries may \
                         have been rewritten; `/api/audit/verify` will fail until \
                         the database and anchor agree again"
                    );
                }
            }
            Ok(None) => {
                // First run with an anchor configured: seed it from the
                // current tip so subsequent boots can detect tampering.
                let tip = log.tip.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let seq = log.entries.lock().unwrap_or_else(|e| e.into_inner()).len() as u64;
                if let Err(e) = Self::write_anchor(&anchor_path, seq, &tip) {
                    tracing::warn!("Failed to initialise audit anchor {anchor_path:?}: {e}");
                } else {
                    tracing::info!(
                        path = ?anchor_path,
                        seq = seq,
                        "Audit anchor file initialised"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    "Audit anchor at {anchor_path:?} is corrupt ({e}); refusing to \
                     overwrite it until an operator inspects / removes the file — \
                     `/api/audit/verify` will fail until resolved"
                );
            }
        }

        log
    }

    /// Creates an audit log backed by a database connection.
    ///
    /// On construction, loads all existing entries from the `audit_entries`
    /// table and verifies the Merkle chain integrity. New entries are written
    /// to both the in-memory chain and the database.
    pub fn with_db(conn: Arc<Mutex<Connection>>) -> Self {
        let mut entries = Vec::new();
        let mut tip = "0".repeat(64);

        // Load existing entries from database
        if let Ok(db) = conn.lock() {
            let result = db.prepare(
                "SELECT seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash FROM audit_entries ORDER BY seq ASC",
            );
            if let Ok(mut stmt) = result {
                let rows = stmt.query_map([], |row| {
                    let action_str: String = row.get(3)?;
                    let action = match action_str.as_str() {
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
                        _ => AuditAction::ToolInvoke, // fallback
                    };
                    let seq_raw: i64 = row.get(0)?;
                    let seq = u64::try_from(seq_raw)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, seq_raw))?;
                    Ok(AuditEntry {
                        seq,
                        timestamp: row.get(1)?,
                        agent_id: row.get(2)?,
                        action,
                        detail: row.get(4)?,
                        outcome: row.get(5)?,
                        prev_hash: row.get(6)?,
                        hash: row.get(7)?,
                    })
                });
                if let Ok(rows) = rows {
                    for entry in rows.flatten() {
                        tip = entry.hash.clone();
                        entries.push(entry);
                    }
                }
            }
        }

        let count = entries.len();
        let log = Self {
            entries: Mutex::new(entries),
            tip: Mutex::new(tip),
            db: Some(conn),
            anchor_path: None,
        };

        // Verify chain integrity on load
        if count > 0 {
            if let Err(e) = log.verify_integrity() {
                tracing::error!("Audit trail integrity check FAILED on boot: {e}");
            } else {
                tracing::info!("Audit trail loaded: {count} entries, chain integrity OK");
            }
        }

        log
    }

    /// Records a new auditable event and returns the SHA-256 hash of the entry.
    ///
    /// The entry is atomically appended to the chain with the current tip as
    /// its `prev_hash`, and the tip is advanced to the new hash.
    /// If a database connection is available, the entry is also persisted.
    pub fn record(
        &self,
        agent_id: impl Into<String>,
        action: AuditAction,
        detail: impl Into<String>,
        outcome: impl Into<String>,
    ) -> String {
        let agent_id = agent_id.into();
        let detail = detail.into();
        let outcome = outcome.into();
        let timestamp = Utc::now().to_rfc3339();

        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut tip = self.tip.lock().unwrap_or_else(|e| e.into_inner());

        let seq = entries.len() as u64;
        let prev_hash = tip.clone();

        let hash = compute_entry_hash(
            seq, &timestamp, &agent_id, &action, &detail, &outcome, &prev_hash,
        );

        let entry = AuditEntry {
            seq,
            timestamp,
            agent_id,
            action,
            detail,
            outcome,
            prev_hash,
            hash: hash.clone(),
        };

        // Persist to database if available
        if let Some(ref db) = self.db {
            if let Ok(conn) = db.lock() {
                let _ = conn.execute(
                    "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        entry.seq as i64,
                        &entry.timestamp,
                        &entry.agent_id,
                        entry.action.to_string(),
                        &entry.detail,
                        &entry.outcome,
                        &entry.prev_hash,
                        &entry.hash,
                    ],
                );
            }
        }

        entries.push(entry);
        *tip = hash.clone();

        // Advance the external anchor so a later DB rewrite is detectable.
        // The anchor stores the post-push count so `verify_integrity`
        // can compare it directly against `entries.len()`. Failures are
        // logged but not propagated — the entry is already in SQLite,
        // and refusing the append because of a filesystem hiccup would
        // lose an audit record, which is strictly worse than an anchor
        // that trails by one tick.
        if let Some(ref anchor_path) = self.anchor_path {
            let count = entries.len() as u64;
            if let Err(e) = Self::write_anchor(anchor_path, count, &hash) {
                tracing::warn!(
                    path = ?anchor_path,
                    "Failed to update audit anchor (entry still persisted): {e}"
                );
            }
        }

        hash
    }

    /// Walks the entire chain and recomputes every hash to detect tampering.
    ///
    /// Returns `Ok(())` if the chain is intact, or `Err(msg)` describing
    /// the first inconsistency found.
    pub fn verify_integrity(&self) -> Result<(), String> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut expected_prev = "0".repeat(64);

        for entry in entries.iter() {
            if entry.prev_hash != expected_prev {
                return Err(format!(
                    "chain break at seq {}: expected prev_hash {} but found {}",
                    entry.seq, expected_prev, entry.prev_hash
                ));
            }

            let recomputed = compute_entry_hash(
                entry.seq,
                &entry.timestamp,
                &entry.agent_id,
                &entry.action,
                &entry.detail,
                &entry.outcome,
                &entry.prev_hash,
            );

            if recomputed != entry.hash {
                return Err(format!(
                    "hash mismatch at seq {}: expected {} but found {}",
                    entry.seq, recomputed, entry.hash
                ));
            }

            expected_prev = entry.hash.clone();
        }

        // External anchor check (if configured). The in-DB chain is
        // internally consistent at this point, so we now make sure the
        // tip agrees with the anchor file that lives outside SQLite.
        // This is the step that catches a full table rewrite where the
        // attacker recomputed every hash from the genesis sentinel
        // forward and the linked-list check above is useless.
        if let Some(ref anchor_path) = self.anchor_path {
            match Self::read_anchor(anchor_path) {
                Ok(Some(record)) => {
                    let current_tip = expected_prev.clone(); // hash of last entry
                    let current_len = entries.len() as u64;
                    // `seq` in the anchor is the number of entries at
                    // the time it was last written. For an append-only
                    // log this must match `entries.len()` once the
                    // chain is up to date.
                    if record.seq != current_len || record.hash != current_tip {
                        return Err(format!(
                            "audit anchor mismatch: anchor says seq={} tip={} \
                             but DB has len={} tip={}",
                            record.seq, record.hash, current_len, current_tip
                        ));
                    }
                }
                Ok(None) => {
                    // Anchor was configured but the file is missing —
                    // fail closed. A legitimate operator would either
                    // remove the anchor configuration or let
                    // `with_db_anchored` seed it on boot; a silent
                    // disappearance is indistinguishable from an
                    // attacker deleting it.
                    return Err(format!(
                        "audit anchor file {anchor_path:?} is missing — cannot \
                         verify tip integrity against external witness"
                    ));
                }
                Err(e) => {
                    return Err(format!("audit anchor unreadable: {e}"));
                }
            }
        }

        Ok(())
    }

    /// Returns the current tip hash (the hash of the most recent entry,
    /// or the genesis sentinel if the log is empty).
    pub fn tip_hash(&self) -> String {
        self.tip.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Returns the number of entries in the log.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Returns whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Returns up to the most recent `n` entries (cloned).
    pub fn recent(&self, n: usize) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let start = entries.len().saturating_sub(n);
        entries[start..].to_vec()
    }

    /// Remove audit entries older than `retention_days` days.
    ///
    /// Returns the number of entries pruned. When `retention_days` is 0 the
    /// call is a no-op (unlimited retention).
    pub fn prune(&self, retention_days: u32) -> usize {
        if retention_days == 0 {
            return 0;
        }

        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        let mut pruned = 0;

        // Prune from database
        if let Some(ref db) = self.db {
            if let Ok(conn) = db.lock() {
                if let Ok(n) = conn.execute(
                    "DELETE FROM audit_entries WHERE timestamp < ?1",
                    rusqlite::params![cutoff_str],
                ) {
                    pruned = n;
                }
            }
        }

        // Prune from in-memory list
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let before = entries.len();
        entries.retain(|e| e.timestamp >= cutoff_str);
        let mem_pruned = before - entries.len();

        // Prefer DB count (authoritative), fall back to in-memory count
        if pruned > 0 {
            pruned
        } else {
            mem_pruned
        }
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_chain_integrity() {
        let log = AuditLog::new();
        log.record(
            "agent-1",
            AuditAction::ToolInvoke,
            "read_file /etc/passwd",
            "ok",
        );
        log.record("agent-1", AuditAction::ShellExec, "ls -la", "ok");
        log.record("agent-2", AuditAction::AgentSpawn, "spawning helper", "ok");
        log.record(
            "agent-1",
            AuditAction::NetworkAccess,
            "https://example.com",
            "denied",
        );

        assert_eq!(log.len(), 4);
        assert!(log.verify_integrity().is_ok());

        // Verify the chain links are correct
        let entries = log.recent(4);
        assert_eq!(entries[0].prev_hash, "0".repeat(64));
        assert_eq!(entries[1].prev_hash, entries[0].hash);
        assert_eq!(entries[2].prev_hash, entries[1].hash);
        assert_eq!(entries[3].prev_hash, entries[2].hash);
    }

    #[test]
    fn test_audit_tamper_detection() {
        let log = AuditLog::new();
        log.record("agent-1", AuditAction::ToolInvoke, "read_file /tmp/a", "ok");
        log.record("agent-1", AuditAction::ShellExec, "rm -rf /", "denied");
        log.record("agent-1", AuditAction::MemoryAccess, "read key foo", "ok");

        // Tamper with an entry
        {
            let mut entries = log.entries.lock().unwrap();
            entries[1].detail = "echo hello".to_string(); // change the detail
        }

        let result = log.verify_integrity();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("hash mismatch at seq 1"));
    }

    #[test]
    fn test_audit_tip_changes() {
        let log = AuditLog::new();
        let genesis_tip = log.tip_hash();
        assert_eq!(genesis_tip, "0".repeat(64));

        let h1 = log.record("a", AuditAction::AgentSpawn, "spawn", "ok");
        assert_eq!(log.tip_hash(), h1);
        assert_ne!(log.tip_hash(), genesis_tip);

        let h2 = log.record("b", AuditAction::AgentKill, "kill", "ok");
        assert_eq!(log.tip_hash(), h2);
        assert_ne!(h2, h1);
    }

    #[test]
    fn test_audit_persists_to_db() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();

        let db = Arc::new(Mutex::new(conn));

        // Record entries with DB
        let log = AuditLog::with_db(Arc::clone(&db));
        log.record("agent-1", AuditAction::AgentSpawn, "spawn test", "ok");
        log.record("agent-1", AuditAction::ShellExec, "ls", "ok");
        assert_eq!(log.len(), 2);

        // Verify entries in database
        let db_conn = db.lock().unwrap();
        let count: i64 = db_conn
            .query_row("SELECT COUNT(*) FROM audit_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
        drop(db_conn);

        // Simulate restart: create new AuditLog from same DB
        let log2 = AuditLog::with_db(Arc::clone(&db));
        assert_eq!(log2.len(), 2);
        assert!(log2.verify_integrity().is_ok());

        // Chain continues correctly after restart
        log2.record("agent-2", AuditAction::ToolInvoke, "file_read", "ok");
        assert_eq!(log2.len(), 3);
        assert!(log2.verify_integrity().is_ok());

        // Verify tip is correct
        let entries = log2.recent(3);
        assert_eq!(entries[2].prev_hash, entries[1].hash);
    }

    // ── External tip anchor ───────────────────────────────────────────────
    //
    // These tests target the scenario documented in the SECURITY audit
    // threat model: an attacker who can write `audit_entries` can wipe
    // every row, insert a fabricated history, and recompute every hash
    // from the genesis sentinel forward, because the linked-list check
    // only proves internal consistency. The external anchor file is
    // what catches that rewrite.

    fn setup_anchored_log() -> (AuditLog, Arc<Mutex<Connection>>, std::path::PathBuf) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));
        let dir = tempfile::tempdir().unwrap();
        let anchor_path = dir.path().join("audit.anchor");
        // Leak the TempDir so the file survives for the duration of the
        // test — we return the PathBuf so the caller keeps owning the
        // cleanup via process exit. Keeping it simple avoids plumbing
        // the TempDir through every test helper.
        std::mem::forget(dir);
        let log = AuditLog::with_db_anchored(Arc::clone(&db), anchor_path.clone());
        (log, db, anchor_path)
    }

    #[test]
    fn test_anchor_detects_full_chain_rewrite() {
        let (log, db, anchor_path) = setup_anchored_log();
        log.record(
            "agent-1",
            AuditAction::ToolInvoke,
            "read_file /etc/hosts",
            "ok",
        );
        log.record("agent-1", AuditAction::ShellExec, "ls -la", "ok");
        log.record("agent-2", AuditAction::AgentSpawn, "spawn helper", "ok");
        assert!(log.verify_integrity().is_ok(), "clean chain should verify");

        // Simulate an attacker wiping the DB and planting a fabricated
        // history with hashes recomputed from the genesis sentinel.
        // Mirror the logic the audit module uses so the in-DB chain
        // stays internally consistent and fools the linked-list check.
        {
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM audit_entries", []).unwrap();
            let mut prev = "0".repeat(64);
            let fabricated: [(u64, &str, AuditAction, &str, &str); 2] = [
                (
                    0,
                    "innocent",
                    AuditAction::AgentMessage,
                    "everything was fine",
                    "ok",
                ),
                (
                    1,
                    "innocent",
                    AuditAction::ToolInvoke,
                    "read-only access",
                    "ok",
                ),
            ];
            for (seq, aid, action, detail, outcome) in fabricated {
                let ts = "2026-04-14T00:00:00+00:00";
                let hash = compute_entry_hash(seq, ts, aid, &action, detail, outcome, &prev);
                conn.execute(
                    "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        seq as i64,
                        ts,
                        aid,
                        action.to_string(),
                        detail,
                        outcome,
                        &prev,
                        &hash,
                    ],
                )
                .unwrap();
                prev = hash;
            }
        }

        // Reopen the log against the rewritten DB — the anchor file
        // still holds the pre-rewrite tip, so verify_integrity must
        // refuse the new chain.
        let log2 = AuditLog::with_db_anchored(Arc::clone(&db), anchor_path.clone());
        let result = log2.verify_integrity();
        assert!(
            result.is_err(),
            "full chain rewrite must be rejected once the anchor exists"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("audit anchor mismatch"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn test_anchor_is_seeded_on_first_boot_if_missing() {
        // DB has rows but no anchor yet: `with_db_anchored` must create
        // the file so subsequent boots can detect tampering.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));

        // First run — no anchor argument, build up some history.
        let log = AuditLog::with_db(Arc::clone(&db));
        log.record("agent-1", AuditAction::ToolInvoke, "read_file", "ok");
        log.record("agent-1", AuditAction::ShellExec, "ls", "ok");
        let current_tip = log.tip_hash();
        drop(log);

        // Second run — upgrade path: anchor file does not exist yet.
        let dir = tempfile::tempdir().unwrap();
        let anchor_path = dir.path().join("audit.anchor");
        assert!(!anchor_path.exists());
        let log2 = AuditLog::with_db_anchored(Arc::clone(&db), anchor_path.clone());
        assert!(
            anchor_path.exists(),
            "anchor file should be seeded on first boot with an existing DB"
        );
        assert!(
            log2.verify_integrity().is_ok(),
            "seeded anchor should agree with current tip"
        );

        // The anchor file should hold the current tip.
        let record = AuditLog::read_anchor(&anchor_path)
            .unwrap()
            .expect("anchor file should parse");
        assert_eq!(record.hash, current_tip);
    }

    #[test]
    fn test_anchor_missing_after_config_fails_closed() {
        let (log, _db, anchor_path) = setup_anchored_log();
        log.record("agent-1", AuditAction::ToolInvoke, "read_file", "ok");
        assert!(log.verify_integrity().is_ok());

        // Attacker removes the anchor file hoping verification will
        // fall back to the DB-only path. It must not.
        std::fs::remove_file(&anchor_path).unwrap();
        let result = log.verify_integrity();
        assert!(result.is_err(), "missing anchor must fail closed");
        assert!(
            result.unwrap_err().contains("missing"),
            "error message should mention the missing anchor"
        );
    }

    #[test]
    fn test_anchor_write_atomic_rename_on_record() {
        let (log, _db, anchor_path) = setup_anchored_log();
        log.record("agent-1", AuditAction::ToolInvoke, "first", "ok");
        let first = AuditLog::read_anchor(&anchor_path).unwrap().unwrap();
        log.record("agent-1", AuditAction::ToolInvoke, "second", "ok");
        let second = AuditLog::read_anchor(&anchor_path).unwrap().unwrap();

        assert_ne!(first.hash, second.hash, "anchor should advance per record");
        assert_eq!(second.seq, 2, "anchor seq should equal entries.len()");
        // No leftover .tmp file.
        let tmp = anchor_path.with_extension("anchor.tmp");
        assert!(!tmp.exists(), "tempfile should have been renamed away");
    }
}
