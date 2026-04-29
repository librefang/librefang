//! Merkle hash chain audit trail for security-critical actions.
//!
//! Every auditable event is appended to an append-only log where each entry
//! contains the SHA-256 hash of its own contents concatenated with the hash of
//! the previous entry, forming a tamper-evident chain (similar to a blockchain).
//!
//! When a database connection is provided (`with_db`), entries are persisted to
//! the `audit_entries` table (schema V8) so the trail survives daemon restarts.

use chrono::Utc;
use librefang_types::agent::UserId;
use librefang_types::config::AuditRetentionConfig;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Hard cap on the number of audit entries kept in memory.
///
/// When `record_with_context` appends an entry that would push the in-memory
/// buffer above this ceiling, the oldest entries are drained from the front so
/// only the most recent `MAX_AUDIT_ENTRIES` survive. This prevents unbounded
/// memory growth in long-running daemons that lack a configured retention
/// policy. The cap applies only to the in-memory window; entries have already
/// been persisted to SQLite before the drain, so forensic completeness is
/// preserved on disk.
const MAX_AUDIT_ENTRIES: usize = 10_000;

/// Categories of auditable actions within the agent runtime.
///
/// **Hash-chain stability:** the variant name is folded into the per-entry
/// SHA-256 via `Display` (which derives from `Debug`). Adding a new variant
/// is safe — old entries keep verifying because their action string is
/// unchanged. Renaming or reordering is a breaking change that invalidates
/// every persisted hash, so treat this enum as append-only.
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
    /// Auto-dream memory consolidation events (start / complete / fail /
    /// abort). The detail string carries the lifecycle phase and task id.
    DreamConsolidation,
    /// RBAC M5: a user authenticated successfully against the API surface.
    /// Recorded on every credential exchange that yields a session token.
    UserLogin,
    /// RBAC M5: a user's role was changed (config edit or admin action).
    /// Detail carries `from=<role> to=<role>`.
    RoleChange,
    /// RBAC M5: a request was rejected by the role-check layer (HTTP 403 or
    /// kernel-level `authorize()` denial). Detail carries the resource that
    /// was denied (path / tool / capability).
    PermissionDenied,
    /// RBAC M5: a per-user, per-agent, or global spend cap was hit. Detail
    /// carries `<window>=$<spend>/$<limit>` (e.g. `daily=$5.20/$5.00`).
    BudgetExceeded,
    /// Retention M7: the audit retention trim job ran and dropped a
    /// prefix of the in-memory window. Detail carries a JSON document
    /// listing per-action drop counts and the new chain anchor hash so
    /// the trim itself is auditable. By construction this entry is the
    /// most recent at the moment it is written and therefore survives
    /// every future trim.
    RetentionTrim,
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
    /// LibreFang user that triggered the action, if known. `None` for kernel
    /// internal events (cron jobs, startup tasks) and pre-migration entries
    /// recorded before user attribution was added in M1.
    #[serde(default)]
    pub user_id: Option<UserId>,
    /// Channel the action originated from (e.g. "telegram", "slack",
    /// "dashboard", "cli"). `None` for kernel-internal events and
    /// pre-migration entries.
    #[serde(default)]
    pub channel: Option<String>,
    /// SHA-256 hash of the previous entry (or all-zeros for the genesis).
    pub prev_hash: String,
    /// SHA-256 hash of this entry's content concatenated with `prev_hash`.
    pub hash: String,
}

/// Computes the SHA-256 hash for a single audit entry from its fields.
///
/// `user_id` and `channel` are folded into the hash only when present so
/// pre-M1 entries — recorded before user attribution existed — verify with
/// the same hash they were originally written with. New entries that supply
/// either field commit it to the chain so a later attempt to strip user
/// attribution from a row would break the Merkle link.
//
// Argument count exceeds clippy's default; folding the inputs into a
// struct would either require building a temporary on every record/verify
// call or change the on-disk hash inputs, both of which are strictly worse
// than the readability cost of nine plain arguments. This is private and
// purely additive — the previous six fields hash identically.
#[allow(clippy::too_many_arguments)]
fn compute_entry_hash(
    seq: u64,
    timestamp: &str,
    agent_id: &str,
    action: &AuditAction,
    detail: &str,
    outcome: &str,
    user_id: Option<&UserId>,
    channel: Option<&str>,
    prev_hash: &str,
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
    /// Hash of the most recent **dropped** entry — set when the
    /// retention trim job removes a prefix of the chain. Verification
    /// checks the first surviving entry's `prev_hash` against this
    /// anchor instead of expecting the genesis sentinel, so the chain
    /// stays verifiable across trim boundaries.
    ///
    /// Held in-memory only and recomputed on `with_db()` boot from the
    /// surviving rows: if the lowest-seq entry's `prev_hash` is not the
    /// genesis sentinel, that `prev_hash` IS the anchor (it points at
    /// the dropped predecessor). No new schema column required.
    chain_anchor: Mutex<Option<String>>,
}

/// Per-trim summary returned by [`AuditLog::trim`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrimReport {
    /// Per-`AuditAction` Display string -> number of entries dropped.
    pub dropped_by_action: BTreeMap<String, usize>,
    /// Total entries dropped (sum of `dropped_by_action`).
    pub total_dropped: usize,
    /// Hash of the last dropped entry, recorded as the new chain anchor.
    /// `None` when no entries were dropped.
    pub new_chain_anchor: Option<String>,
}

impl TrimReport {
    /// Whether this trim removed any entries.
    pub fn is_empty(&self) -> bool {
        self.total_dropped == 0
    }
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
            chain_anchor: Mutex::new(None),
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
                         the database and anchor agree again. \
                         Inspect with `librefang security verify`; if you accept the \
                         loss of pre-break forensic value (typical in dev), \
                         `librefang security audit-reset` truncates the chain and \
                         re-anchors at zero. DO NOT run reset in compliance / \
                         production environments."
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

        // Load existing entries from database. Schema v22 added the
        // `user_id` / `channel` columns; rows persisted before that
        // migration return NULL for both, which deserialises to `None`
        // and keeps the original hash intact (the hash function omits
        // absent fields, see `compute_entry_hash`).
        if let Ok(db) = conn.lock() {
            let result = db.prepare(
                "SELECT seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash FROM audit_entries ORDER BY seq ASC",
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
                        "DreamConsolidation" => AuditAction::DreamConsolidation,
                        "UserLogin" => AuditAction::UserLogin,
                        "RoleChange" => AuditAction::RoleChange,
                        "PermissionDenied" => AuditAction::PermissionDenied,
                        "BudgetExceeded" => AuditAction::BudgetExceeded,
                        "RetentionTrim" => AuditAction::RetentionTrim,
                        _ => AuditAction::ToolInvoke, // fallback
                    };
                    let seq_raw: i64 = row.get(0)?;
                    let seq = u64::try_from(seq_raw)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, seq_raw))?;
                    let user_id_str: Option<String> = row.get(6)?;
                    let user_id = user_id_str.as_deref().and_then(|s| s.parse().ok());
                    let channel: Option<String> = row.get(7)?;
                    Ok(AuditEntry {
                        seq,
                        timestamp: row.get(1)?,
                        agent_id: row.get(2)?,
                        action,
                        detail: row.get(4)?,
                        outcome: row.get(5)?,
                        user_id,
                        channel,
                        prev_hash: row.get(8)?,
                        hash: row.get(9)?,
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

        // Recover any chain anchor left behind by a prior trim cycle.
        // If the surviving entries' lowest seq is N>0, OR the first
        // entry's `prev_hash` is non-genesis, the predecessor was dropped
        // and that prev_hash IS the anchor — no separate persisted column
        // needed because the anchor is just "what the surviving prefix
        // already points at". This keeps verification working across
        // restarts without schema changes.
        let recovered_anchor = match entries.first() {
            Some(first) if first.prev_hash != "0".repeat(64) => Some(first.prev_hash.clone()),
            _ => None,
        };

        let log = Self {
            entries: Mutex::new(entries),
            tip: Mutex::new(tip),
            db: Some(conn),
            anchor_path: None,
            chain_anchor: Mutex::new(recovered_anchor),
        };

        // Verify chain integrity on load
        if count > 0 {
            if let Err(e) = log.verify_integrity() {
                tracing::error!(
                    "Audit trail integrity check FAILED on boot: {e}. \
                     Run `librefang security verify` to inspect; if you accept the \
                     loss of pre-break forensic value (typical in dev), \
                     `librefang security audit-reset` truncates the chain and \
                     re-anchors at zero. DO NOT run reset in compliance / \
                     production environments."
                );
            } else {
                tracing::info!("Audit trail loaded: {count} entries, chain integrity OK");
            }
        }

        log
    }

    /// Records a new auditable event and returns the SHA-256 hash of the entry.
    ///
    /// Convenience wrapper over [`AuditLog::record_with_context`] that omits
    /// user / channel attribution. Prefer the contextual variant when the
    /// caller knows who or where the action originated from — pre-M1 call
    /// sites use this form and remain valid.
    pub fn record(
        &self,
        agent_id: impl Into<String>,
        action: AuditAction,
        detail: impl Into<String>,
        outcome: impl Into<String>,
    ) -> String {
        self.record_with_context(agent_id, action, detail, outcome, None, None)
    }

    /// Records a new auditable event with optional user / channel attribution.
    ///
    /// The entry is atomically appended to the chain with the current tip as
    /// its `prev_hash`, and the tip is advanced to the new hash.
    /// If a database connection is available, the entry is also persisted.
    pub fn record_with_context(
        &self,
        agent_id: impl Into<String>,
        action: AuditAction,
        detail: impl Into<String>,
        outcome: impl Into<String>,
        user_id: Option<UserId>,
        channel: Option<String>,
    ) -> String {
        let agent_id = agent_id.into();
        let detail = detail.into();
        let outcome = outcome.into();
        let timestamp = Utc::now().to_rfc3339();

        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut tip = self.tip.lock().unwrap_or_else(|e| e.into_inner());

        // Derive the next seq from the last entry, not `entries.len()`,
        // because a retention trim may have dropped a prefix — using
        // `len()` would re-issue a seq the surviving entries already
        // hold and would also collide with the SQLite PRIMARY KEY.
        let seq = entries.last().map(|e| e.seq + 1).unwrap_or(0);
        let prev_hash = tip.clone();

        let hash = compute_entry_hash(
            seq,
            &timestamp,
            &agent_id,
            &action,
            &detail,
            &outcome,
            user_id.as_ref(),
            channel.as_deref(),
            &prev_hash,
        );

        let entry = AuditEntry {
            seq,
            timestamp,
            agent_id,
            action,
            detail,
            outcome,
            user_id,
            channel,
            prev_hash,
            hash: hash.clone(),
        };

        // Persist to database if available. Schema v22 added the
        // `user_id` / `channel` columns; old NULL rows keep working
        // because the hash function omits absent fields.
        //
        // CRITICAL: chain integrity requires that the in-memory tip and
        // the persisted tail agree at all times.  If the SQLite INSERT
        // fails but we still push the entry into `entries` and advance
        // `tip`, the next record() reads the new tip, hashes it into
        // the next entry's `prev_hash`, and writes *that* row to disk.
        // After a restart, `with_db()` reloads the DB and finds an
        // entry whose `prev_hash` points at a row that was never
        // persisted — `verify_integrity()` then reports
        // `chain break at seq N` on every subsequent boot, and the
        // operator has to run `audit-reset` to recover.
        //
        // The earlier in-memory `non_persisted_seqs` queue (#4050)
        // tried to delay this corruption by retrying inside the
        // process, but the queue lived only in memory — any restart
        // (graceful or otherwise) before the retry succeeded
        // committed the broken on-disk chain.
        //
        // We invert the trade-off: a transient DB write failure drops
        // the audit event and leaves chain state untouched.  The ERROR
        // log below is the operator's signal to investigate.  The
        // next call uses the same `seq` (since `entries.last()` did
        // not advance) with a fresh timestamp and tries again.
        let persisted = match self.db.as_ref() {
            None => true, // pure in-memory mode: memory IS the source of truth
            Some(db) => match db.lock() {
                Ok(conn) => match conn.execute(
                    "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        entry.seq as i64,
                        &entry.timestamp,
                        &entry.agent_id,
                        entry.action.to_string(),
                        &entry.detail,
                        &entry.outcome,
                        entry.user_id.as_ref().map(|u| u.to_string()),
                        entry.channel.as_deref(),
                        &entry.prev_hash,
                        &entry.hash,
                    ],
                ) {
                    Ok(_) => true,
                    Err(e) => {
                        tracing::error!(
                            seq = entry.seq,
                            agent_id = %entry.agent_id,
                            action = %entry.action,
                            error = %e,
                            "Audit DB INSERT failed; chain NOT advanced. \
                             Entry dropped to preserve on-disk chain integrity. \
                             Investigate disk space, permissions, or DB state."
                        );
                        false
                    }
                },
                Err(e) => {
                    tracing::error!(
                        seq = entry.seq,
                        "Audit DB mutex poisoned ({e:?}); chain NOT advanced."
                    );
                    false
                }
            },
        };

        if !persisted {
            // Drop locks without mutating; caller's discarded return
            // value is the (uncommitted) hash, mirroring the success
            // path's signature.  The next record() will reuse the same
            // `seq` because `entries.last()` is unchanged.
            return hash;
        }

        entries.push(entry);
        *tip = hash.clone();

        // Hard cap: if the in-memory buffer grew beyond MAX_AUDIT_ENTRIES,
        // drain the oldest prefix.  Every entry in `entries` is now
        // known to be persisted on disk (the only path that pushes is
        // the success branch above), so dropping the prefix loses no
        // forensic data — a restart would reload the same rows from
        // SQLite anyway.  We update `chain_anchor` to the hash of the
        // last dropped entry so `verify_integrity()` keeps working
        // across the trim boundary.
        if entries.len() > MAX_AUDIT_ENTRIES {
            let overflow = entries.len() - MAX_AUDIT_ENTRIES;
            let new_anchor = entries[overflow - 1].hash.clone();
            {
                let mut anchor =
                    self.chain_anchor.lock().unwrap_or_else(|e| e.into_inner());
                *anchor = Some(new_anchor);
            }
            entries.drain(..overflow);
        }

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
        // When the retention trim job has dropped a prefix, the first
        // surviving entry's `prev_hash` points at the last dropped
        // entry rather than the genesis sentinel. Seed the walk from
        // the chain anchor so the trim boundary verifies cleanly.
        let anchor = self
            .chain_anchor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut expected_prev = anchor.unwrap_or_else(|| "0".repeat(64));

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
                entry.user_id.as_ref(),
                entry.channel.as_deref(),
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

    /// Apply the per-action retention `policy` against the in-memory
    /// audit window, dropping a prefix and updating the chain anchor so
    /// the surviving entries still verify.
    ///
    /// Drop logic per entry (top-down, in seq order):
    ///   1. If `max_in_memory_entries` is set and non-zero, drop oldest
    ///      until the survivor count <= cap.
    ///   2. Then for each remaining entry: if its action has a
    ///      configured retention window AND the entry is older than the
    ///      window, drop it. Actions without a configured window are
    ///      kept forever ("default = preserve").
    ///
    /// **Prefix-only:** to keep the chain anchor logic sound, dropping
    /// is a contiguous prefix only. The first action whose retention
    /// keeps it stops the trim — newer entries (even of the "should
    /// drop" actions) survive. This matches how the chain works: you
    /// can't punch holes in a Merkle list. In practice the in-memory
    /// log is append-ordered by time, so per-action retention rules
    /// trim exactly the rows the operator expects.
    ///
    /// Returns a [`TrimReport`] describing what was removed.
    pub fn trim(
        &self,
        policy: &AuditRetentionConfig,
        now: chrono::DateTime<chrono::Utc>,
    ) -> TrimReport {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());

        // Decide the prefix length to drop. We compute `drop_count`
        // first without mutating, then apply both the DB delete and the
        // in-memory truncation atomically below.
        let total = entries.len();
        if total == 0 {
            return TrimReport::default();
        }

        // Pass 1: enforce max_in_memory_entries cap. This is independent
        // of action and acts as a hard floor on memory pressure.
        let cap = policy.max_in_memory_entries.unwrap_or(0);
        let mut drop_count: usize = if cap > 0 && total > cap {
            total - cap
        } else {
            0
        };

        // Pass 2: walk forward from the current `drop_count` index and
        // extend the prefix as long as the next entry is eligible
        // (action has a retention rule + entry is older than its
        // window). Stop at the first survivor — the chain is contiguous,
        // so we cannot drop holes.
        while drop_count < total {
            let entry = &entries[drop_count];
            let action_str = entry.action.to_string();
            let retention_days = match policy.retention_days_by_action.get(&action_str) {
                Some(d) if *d > 0 => *d,
                // No rule (or 0 = unlimited) -> keep forever, stop here.
                _ => break,
            };
            let cutoff = now - chrono::Duration::days(retention_days as i64);
            // Entry timestamps are RFC-3339; parse failure means we keep
            // the entry to avoid dropping rows we can't reason about.
            let ts = match chrono::DateTime::parse_from_rfc3339(&entry.timestamp) {
                Ok(t) => t.with_timezone(&chrono::Utc),
                Err(_) => break,
            };
            if ts < cutoff {
                drop_count += 1;
            } else {
                break;
            }
        }

        if drop_count == 0 {
            return TrimReport::default();
        }

        // Tally per-action drops for the report and capture the new
        // anchor (hash of the last dropped entry).
        let mut report = TrimReport::default();
        for entry in &entries[..drop_count] {
            *report
                .dropped_by_action
                .entry(entry.action.to_string())
                .or_insert(0) += 1;
        }
        report.total_dropped = drop_count;
        report.new_chain_anchor = Some(entries[drop_count - 1].hash.clone());

        // Persist: drop the same prefix from SQLite so a restart sees a
        // consistent view. We delete by seq < first-survivor.seq —
        // works whether or not seq starts at 0.
        let first_survivor_seq = if drop_count < total {
            entries[drop_count].seq
        } else {
            // Reachable when every action has a per-action retention
            // rule and every entry is older than its window. Drop the
            // tail row from the DB too so the on-disk view matches the
            // empty in-memory log; otherwise a restart would load an
            // orphan row whose `prev_hash` points at a hash no `with_db`
            // anchor recovery can reconstruct, and `verify_integrity`
            // would fail on the next boot. The next `record()` call
            // (typically the self-audit `RetentionTrim` written by the
            // caller) re-anchors against the chain_anchor we set below.
            entries[total - 1].seq + 1
        };
        if let Some(ref db) = self.db {
            if let Ok(conn) = db.lock() {
                let _ = conn.execute(
                    "DELETE FROM audit_entries WHERE seq < ?1",
                    rusqlite::params![first_survivor_seq as i64],
                );
            }
        }

        // Mutate in-memory state. Order matters: anchor before drain
        // so a concurrent verify_integrity (blocked on the entries
        // lock) sees a consistent (anchor, first_survivor) pair when
        // it acquires.
        {
            let mut anchor = self.chain_anchor.lock().unwrap_or_else(|e| e.into_inner());
            *anchor = report.new_chain_anchor.clone();
        }
        entries.drain(..drop_count);

        // Refresh the external anchor file so its `seq` column tracks
        // the new (post-trim) `entries.len()`. The tip hash itself does
        // NOT change — trimming a prefix never moves the tail — but the
        // seq does, and `verify_integrity` insists they agree. Failing
        // to rewrite the anchor here would surface as a spurious
        // "audit anchor mismatch" on the very next verification.
        if let Some(ref anchor_path) = self.anchor_path {
            let new_len = entries.len() as u64;
            let tip = self.tip.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Err(e) = Self::write_anchor(anchor_path, new_len, &tip) {
                tracing::warn!(
                    path = ?anchor_path,
                    "Failed to refresh audit anchor after trim: {e}"
                );
            }
        }

        report
    }

    /// Remove audit entries older than `retention_days` days.
    ///
    /// Returns the number of entries pruned. When `retention_days` is 0 the
    /// call is a no-op (unlimited retention).
    ///
    /// Like [`AuditLog::trim`], this is **prefix-only**: it walks forward
    /// from the oldest entry and stops at the first whose timestamp is
    /// inside the retention window, so the surviving log stays a
    /// contiguous suffix of the original chain. The `chain_anchor` is
    /// updated to the hash of the last dropped entry so
    /// [`AuditLog::verify_integrity`] keeps verifying across the prune
    /// boundary — without this the next verify would fail with a chain
    /// break at the new first survivor (whose `prev_hash` no longer
    /// points at any in-DB row).
    pub fn prune(&self, retention_days: u32) -> usize {
        if retention_days == 0 {
            return 0;
        }

        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();

        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let total = entries.len();
        if total == 0 {
            return 0;
        }

        // Walk the oldest contiguous prefix of expired entries. Stops at
        // the first entry whose timestamp is inside the retention window
        // — even if later entries are also expired (they shouldn't be in
        // an append-ordered log, but guard anyway so we never punch a
        // hole in the chain).
        let mut drop_count = 0usize;
        while drop_count < total && entries[drop_count].timestamp < cutoff_str {
            drop_count += 1;
        }
        if drop_count == 0 {
            return 0;
        }

        // Update the in-memory chain anchor BEFORE draining so a verify
        // racing against this prune (blocked on the entries lock) sees a
        // consistent (anchor, first_survivor) pair on the next acquire.
        let new_anchor = entries[drop_count - 1].hash.clone();
        {
            let mut anchor = self.chain_anchor.lock().unwrap_or_else(|e| e.into_inner());
            *anchor = Some(new_anchor);
        }

        // Persist: delete the same prefix from SQLite using `seq` rather
        // than `timestamp` so DB and in-memory share one source of truth
        // for what survived. When we drop everything, bump past the last
        // seq so the tail row is not orphaned (mirrors the fix in
        // `AuditLog::trim`).
        let first_survivor_seq = if drop_count < total {
            entries[drop_count].seq
        } else {
            entries[total - 1].seq + 1
        };
        if let Some(ref db) = self.db {
            if let Ok(conn) = db.lock() {
                let _ = conn.execute(
                    "DELETE FROM audit_entries WHERE seq < ?1",
                    rusqlite::params![first_survivor_seq as i64],
                );
            }
        }

        entries.drain(..drop_count);

        // Refresh the external anchor file's `seq` column so the next
        // verify_integrity() does not trip the "anchor seq mismatch"
        // guard. Tip itself does not move (we only drop a prefix).
        if let Some(ref anchor_path) = self.anchor_path {
            let new_len = entries.len() as u64;
            let tip = self.tip.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Err(e) = Self::write_anchor(anchor_path, new_len, &tip) {
                tracing::warn!(
                    path = ?anchor_path,
                    "Failed to refresh audit anchor after prune: {e}"
                );
            }
        }

        drop_count
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
    fn test_record_with_context_round_trips_user_and_channel() {
        // RBAC M1: AuditEntry carries user_id + channel attribution. Both
        // are optional so legacy `record(...)` still works (folds to None).
        let log = AuditLog::new();
        let alice = UserId::from_name("Alice");

        log.record("agent-1", AuditAction::AgentSpawn, "boot", "ok"); // legacy
        log.record_with_context(
            "agent-1",
            AuditAction::ToolInvoke,
            "file_read /tmp/x",
            "ok",
            Some(alice),
            Some("api".to_string()),
        );

        assert!(log.verify_integrity().is_ok());

        let entries = log.recent(2);
        assert_eq!(entries[0].user_id, None);
        assert_eq!(entries[0].channel, None);
        assert_eq!(entries[1].user_id, Some(alice));
        assert_eq!(entries[1].channel.as_deref(), Some("api"));

        // Tampering with a recorded user_id must break the chain — proves
        // attribution is committed to the Merkle hash, not a side note.
        let tampered_hash = compute_entry_hash(
            entries[1].seq,
            &entries[1].timestamp,
            &entries[1].agent_id,
            &entries[1].action,
            &entries[1].detail,
            &entries[1].outcome,
            None, // pretend user_id was never there
            entries[1].channel.as_deref(),
            &entries[1].prev_hash,
        );
        assert_ne!(
            tampered_hash, entries[1].hash,
            "stripping user_id must change the hash"
        );
    }

    #[test]
    fn test_record_with_context_persists_user_and_channel() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                user_id TEXT,
                channel TEXT,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();

        let db = Arc::new(Mutex::new(conn));
        let bob = UserId::from_name("Bob");

        let log = AuditLog::with_db(Arc::clone(&db));
        log.record("agent-1", AuditAction::AgentSpawn, "boot", "ok");
        log.record_with_context(
            "agent-1",
            AuditAction::ConfigChange,
            "config set: x",
            "ok",
            Some(bob),
            Some("api".to_string()),
        );

        // Reopen — chain must verify and the contextual entry must round-trip.
        let log2 = AuditLog::with_db(Arc::clone(&db));
        assert_eq!(log2.len(), 2);
        assert!(log2.verify_integrity().is_ok());
        let entries = log2.recent(2);
        assert_eq!(entries[1].user_id, Some(bob));
        assert_eq!(entries[1].channel.as_deref(), Some("api"));
    }

    #[test]
    fn test_new_rbac_variants_preserve_chain() {
        // RBAC M5: UserLogin / RoleChange / PermissionDenied / BudgetExceeded
        // must hash like every other variant — adding them MUST NOT shift the
        // hash of pre-existing rows. We verify two things:
        //   1. Mixing the new variants into a fresh chain still verifies.
        //   2. The variant names round-trip through `Display` exactly so
        //      `with_db()` can decode them after a daemon restart.
        let log = AuditLog::new();
        let alice = UserId::from_name("Alice");
        log.record_with_context(
            "system",
            AuditAction::UserLogin,
            "alice via api key",
            "ok",
            Some(alice),
            Some("api".to_string()),
        );
        log.record_with_context(
            "system",
            AuditAction::RoleChange,
            "from=user to=admin",
            "ok",
            Some(alice),
            Some("api".to_string()),
        );
        log.record_with_context(
            "system",
            AuditAction::PermissionDenied,
            "/api/budget/users",
            "denied",
            Some(alice),
            Some("api".to_string()),
        );
        log.record_with_context(
            "system",
            AuditAction::BudgetExceeded,
            "daily=$5.20/$5.00",
            "denied",
            Some(alice),
            Some("api".to_string()),
        );
        // M7: RetentionTrim joins the locked-name set so the trim
        // self-audit row also survives a daemon restart.
        log.record(
            "system",
            AuditAction::RetentionTrim,
            r#"{"dropped":{"ToolInvoke":3}}"#,
            "ok",
        );
        assert!(log.verify_integrity().is_ok(), "new variants must verify");

        // Lock the on-disk display of every variant. Renaming any of these
        // would invalidate every persisted hash that mentions them — the
        // assertions exist so a casual refactor surfaces as a test failure.
        assert_eq!(AuditAction::UserLogin.to_string(), "UserLogin");
        assert_eq!(AuditAction::RoleChange.to_string(), "RoleChange");
        assert_eq!(
            AuditAction::PermissionDenied.to_string(),
            "PermissionDenied"
        );
        assert_eq!(AuditAction::BudgetExceeded.to_string(), "BudgetExceeded");
        assert_eq!(AuditAction::RetentionTrim.to_string(), "RetentionTrim");
    }

    #[test]
    fn test_user_id_from_name_is_stable_across_audit_writes() {
        // The whole point of `UserId::from_name` is that audit attribution
        // survives a daemon restart. Re-deriving the id from the same name
        // must yield the same UUID written into earlier entries.
        let log = AuditLog::new();
        log.record_with_context(
            "agent-1",
            AuditAction::AgentMessage,
            "ping",
            "ok",
            Some(UserId::from_name("Alice")),
            Some("telegram".to_string()),
        );
        let recorded = log.recent(1)[0].user_id.unwrap();
        let rederived = UserId::from_name("Alice");
        assert_eq!(recorded, rederived);
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
                user_id TEXT,
                channel TEXT,
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
                user_id TEXT,
                channel TEXT,
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
                let hash =
                    compute_entry_hash(seq, ts, aid, &action, detail, outcome, None, None, &prev);
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
                user_id TEXT,
                channel TEXT,
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

    // ── Retention trim (M7) ──────────────────────────────────────────────
    //
    // These tests cover the per-action retention policy. The crucial
    // invariant is that the chain still verifies after a prefix is
    // dropped — that's what the in-memory `chain_anchor` exists to
    // prove. See `AuditLog::trim` for the design notes.

    /// Push an entry whose timestamp the test controls, by recording it
    /// normally and then back-dating the timestamp + recomputing hashes.
    /// The post-edit chain still verifies because we re-link properly.
    fn push_aged_entry(
        log: &AuditLog,
        agent_id: &str,
        action: AuditAction,
        detail: &str,
        outcome: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) {
        log.record(agent_id, action, detail, outcome);
        let mut entries = log.entries.lock().unwrap();
        let last_idx = entries.len() - 1;
        entries[last_idx].timestamp = timestamp.to_rfc3339();
        // Recompute the last entry's hash with the new timestamp + same prev_hash.
        let new_hash = compute_entry_hash(
            entries[last_idx].seq,
            &entries[last_idx].timestamp,
            &entries[last_idx].agent_id,
            &entries[last_idx].action,
            &entries[last_idx].detail,
            &entries[last_idx].outcome,
            entries[last_idx].user_id.as_ref(),
            entries[last_idx].channel.as_deref(),
            &entries[last_idx].prev_hash,
        );
        entries[last_idx].hash = new_hash.clone();
        drop(entries);
        // Update the tip so the next record links to the right hash.
        *log.tip.lock().unwrap() = new_hash;
    }

    #[test]
    fn test_trim_drops_old_entries_by_action() {
        let log = AuditLog::new();
        let now = chrono::Utc::now();
        let two_days_ago = now - chrono::Duration::days(2);
        let one_hour_ago = now - chrono::Duration::hours(1);

        push_aged_entry(
            &log,
            "agent-1",
            AuditAction::ToolInvoke,
            "old tool call",
            "ok",
            two_days_ago,
        );
        push_aged_entry(
            &log,
            "agent-1",
            AuditAction::ToolInvoke,
            "another old tool call",
            "ok",
            two_days_ago,
        );
        push_aged_entry(
            &log,
            "agent-1",
            AuditAction::RoleChange,
            "from=user to=admin",
            "ok",
            two_days_ago,
        );
        push_aged_entry(
            &log,
            "agent-1",
            AuditAction::ToolInvoke,
            "recent tool call",
            "ok",
            one_hour_ago,
        );

        let mut policy = AuditRetentionConfig::default();
        policy
            .retention_days_by_action
            .insert("ToolInvoke".to_string(), 1);
        // Note: RoleChange has no policy entry -> kept forever.

        let report = log.trim(&policy, now);
        // Trim is prefix-only: the first two ToolInvoke (2d old) drop;
        // then the third entry is RoleChange, which has no rule, so
        // the trim stops. The recent ToolInvoke survives because trim
        // halts at the first kept row.
        assert_eq!(report.total_dropped, 2);
        assert_eq!(report.dropped_by_action.get("ToolInvoke"), Some(&2));
        assert_eq!(log.len(), 2);
        assert!(
            log.verify_integrity().is_ok(),
            "chain must still verify after prefix trim"
        );

        let survivors = log.recent(10);
        assert!(matches!(survivors[0].action, AuditAction::RoleChange));
        assert!(matches!(survivors[1].action, AuditAction::ToolInvoke));
        assert_eq!(survivors[1].detail, "recent tool call");
    }

    #[test]
    fn test_trim_preserves_chain_via_anchor() {
        let log = AuditLog::new();
        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(30);

        for i in 0..5 {
            push_aged_entry(
                &log,
                "agent-1",
                AuditAction::ToolInvoke,
                &format!("old call {i}"),
                "ok",
                old_ts,
            );
        }
        // Recent entries that should survive.
        log.record("agent-1", AuditAction::ToolInvoke, "fresh", "ok");
        log.record("agent-1", AuditAction::ToolInvoke, "fresher", "ok");

        let mut policy = AuditRetentionConfig::default();
        policy
            .retention_days_by_action
            .insert("ToolInvoke".to_string(), 7);

        let dropped_predecessor_hash = log.entries.lock().unwrap()[4].hash.clone();
        let first_survivor_prev = log.entries.lock().unwrap()[5].prev_hash.clone();
        // Sanity: the first survivor's prev_hash IS the predecessor's
        // hash before trim — the anchor approach exploits exactly this.
        assert_eq!(dropped_predecessor_hash, first_survivor_prev);

        let report = log.trim(&policy, now);
        assert_eq!(report.total_dropped, 5);
        assert_eq!(
            report.new_chain_anchor.as_deref(),
            Some(dropped_predecessor_hash.as_str()),
            "anchor should be the last dropped entry's hash"
        );
        assert!(
            log.verify_integrity().is_ok(),
            "verify_integrity must succeed via anchor after prefix trim"
        );

        // Subsequent record() calls must keep the chain intact across
        // the trim boundary — the new entry links to the (unchanged)
        // tip, and verification still uses the anchor for the first
        // survivor.
        log.record("agent-1", AuditAction::ToolInvoke, "post-trim", "ok");
        assert!(log.verify_integrity().is_ok());
    }

    #[test]
    fn test_trim_records_self_audit_via_caller() {
        // The trim() method itself doesn't write a self-audit row —
        // that's the caller's job (the kernel periodic task) so trim()
        // stays a pure data-mutation primitive that's easy to test.
        // This test exercises the contract the kernel relies on:
        // record() AFTER trim() lands a RetentionTrim row that
        // survives by construction (it's the newest entry).
        let log = AuditLog::new();
        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(3);

        for _ in 0..3 {
            push_aged_entry(
                &log,
                "agent-1",
                AuditAction::ToolInvoke,
                "noise",
                "ok",
                old_ts,
            );
        }
        let mut policy = AuditRetentionConfig::default();
        policy
            .retention_days_by_action
            .insert("ToolInvoke".to_string(), 1);

        let report = log.trim(&policy, now);
        assert_eq!(report.total_dropped, 3);

        // Caller writes the self-audit row.
        let detail = serde_json::to_string(&report.dropped_by_action).unwrap();
        log.record("system", AuditAction::RetentionTrim, detail, "ok");

        let entries = log.recent(10);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].action, AuditAction::RetentionTrim));
        assert!(entries[0].detail.contains("ToolInvoke"));
        assert!(log.verify_integrity().is_ok());
    }

    #[test]
    fn test_max_in_memory_cap_enforced() {
        let log = AuditLog::new();
        // 200 RoleChange entries (no per-action retention rule) so only
        // the cap applies. Use recent timestamps so no per-action rule
        // could possibly drop them anyway.
        for i in 0..200 {
            log.record(
                "agent-1",
                AuditAction::RoleChange,
                format!("change #{i}"),
                "ok",
            );
        }
        assert_eq!(log.len(), 200);

        let policy = AuditRetentionConfig {
            max_in_memory_entries: Some(100),
            ..Default::default()
        };

        let report = log.trim(&policy, chrono::Utc::now());
        assert_eq!(report.total_dropped, 100);
        assert_eq!(log.len(), 100);
        assert!(log.verify_integrity().is_ok());

        // The most recent 100 entries must survive — verify by
        // checking the tail's detail string.
        let survivors = log.recent(100);
        assert_eq!(survivors.first().unwrap().detail, "change #100");
        assert_eq!(survivors.last().unwrap().detail, "change #199");
    }

    #[test]
    fn test_default_config_is_no_op() {
        let log = AuditLog::new();
        log.record("agent-1", AuditAction::ToolInvoke, "x", "ok");
        log.record("agent-1", AuditAction::ToolInvoke, "y", "ok");

        let policy = AuditRetentionConfig::default();
        let report = log.trim(&policy, chrono::Utc::now());
        assert!(report.is_empty());
        assert_eq!(report.total_dropped, 0);
        assert!(report.new_chain_anchor.is_none());
        assert_eq!(log.len(), 2);
        assert!(log.chain_anchor.lock().unwrap().is_none());
    }

    #[test]
    fn test_trim_persists_to_db_and_recovers_anchor_on_reload() {
        // The chain_anchor is in-memory only — but when the daemon
        // restarts we recompute it from the surviving rows. Verify
        // that round-trip works: trim, drop the AuditLog, reopen
        // against the same DB, and check verify_integrity() passes
        // because with_db() recovered the anchor from the survivors'
        // first prev_hash.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                user_id TEXT,
                channel TEXT,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));

        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(30);

        let log = AuditLog::with_db(Arc::clone(&db));
        for i in 0..5 {
            push_aged_entry(
                &log,
                "agent-1",
                AuditAction::ToolInvoke,
                &format!("old {i}"),
                "ok",
                old_ts,
            );
        }
        // Persist the back-dated rows by re-syncing — push_aged_entry
        // mutates in-memory only, so re-write the DB rows manually.
        {
            let entries = log.entries.lock().unwrap();
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM audit_entries", []).unwrap();
            for e in entries.iter() {
                conn.execute(
                    "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        e.seq as i64,
                        &e.timestamp,
                        &e.agent_id,
                        e.action.to_string(),
                        &e.detail,
                        &e.outcome,
                        e.user_id.map(|u| u.to_string()),
                        e.channel.as_deref(),
                        &e.prev_hash,
                        &e.hash,
                    ],
                )
                .unwrap();
            }
        }
        log.record("agent-1", AuditAction::RoleChange, "keep me", "ok");

        let mut policy = AuditRetentionConfig::default();
        policy
            .retention_days_by_action
            .insert("ToolInvoke".to_string(), 7);

        let report = log.trim(&policy, now);
        assert_eq!(report.total_dropped, 5);
        let anchor_after_trim = report.new_chain_anchor.clone().unwrap();
        drop(log);

        // Reopen — anchor must be reconstructed from the survivor's
        // prev_hash so verify_integrity() succeeds.
        let log2 = AuditLog::with_db(Arc::clone(&db));
        assert_eq!(log2.len(), 1);
        let recovered = log2.chain_anchor.lock().unwrap().clone();
        assert_eq!(
            recovered.as_deref(),
            Some(anchor_after_trim.as_str()),
            "with_db() should recover the anchor from the surviving prefix"
        );
        assert!(
            log2.verify_integrity().is_ok(),
            "verify_integrity must succeed after restart with anchor recovered"
        );
    }

    #[test]
    fn test_trim_drops_all_persists_consistently_across_restart() {
        // Regression: when every entry in the log has a per-action
        // retention rule and is older than its window, pass-2 advances
        // drop_count all the way to total. The DB delete must remove
        // every row (matching the empty in-memory state) — leaving the
        // tail behind would orphan a row whose prev_hash points at a
        // dropped predecessor, breaking verify_integrity on the next
        // boot. The next record() (typically the self-audit
        // RetentionTrim row) re-anchors against chain_anchor.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                user_id TEXT,
                channel TEXT,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));

        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(30);

        let log = AuditLog::with_db(Arc::clone(&db));
        for i in 0..4 {
            push_aged_entry(
                &log,
                "agent-1",
                AuditAction::ToolInvoke,
                &format!("noise {i}"),
                "ok",
                old_ts,
            );
        }
        // Re-sync the back-dated rows into the DB (push_aged_entry
        // mutates in-memory only).
        {
            let entries = log.entries.lock().unwrap();
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM audit_entries", []).unwrap();
            for e in entries.iter() {
                conn.execute(
                    "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        e.seq as i64,
                        &e.timestamp,
                        &e.agent_id,
                        e.action.to_string(),
                        &e.detail,
                        &e.outcome,
                        e.user_id.map(|u| u.to_string()),
                        e.channel.as_deref(),
                        &e.prev_hash,
                        &e.hash,
                    ],
                )
                .unwrap();
            }
        }

        let mut policy = AuditRetentionConfig::default();
        policy
            .retention_days_by_action
            .insert("ToolInvoke".to_string(), 1);

        // Every entry is ToolInvoke, every entry is 30 days old, rule
        // is 1 day -> pass-2 drops all four.
        let report = log.trim(&policy, now);
        assert_eq!(report.total_dropped, 4);
        assert_eq!(log.len(), 0);

        // No orphan row left in the DB.
        let db_count: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM audit_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            db_count, 0,
            "drop-everything trim must clear DB, not leave the tail row behind"
        );

        // Caller records the self-audit row — the kernel periodic task
        // does this after every non-empty trim.
        log.record("system", AuditAction::RetentionTrim, "all", "ok");
        assert!(log.verify_integrity().is_ok());
        drop(log);

        // Restart: only the RetentionTrim row exists. Anchor must be
        // recovered from its prev_hash so verify_integrity walks
        // cleanly across the trim boundary.
        let log2 = AuditLog::with_db(Arc::clone(&db));
        assert_eq!(log2.len(), 1);
        assert!(
            log2.verify_integrity().is_ok(),
            "verify_integrity must succeed after restart when trim dropped every prior entry"
        );
    }

    #[test]
    fn test_prune_updates_chain_anchor_so_verify_passes() {
        // Regression: the legacy day-based `prune` runs in parallel
        // with the new per-action `trim`. After this PR introduced
        // chain_anchor as the seed for verify_integrity(), prune had to
        // start updating it too — otherwise dropping an old prefix
        // would leave the surviving first entry with prev_hash pointing
        // at a now-deleted predecessor while the anchor stayed None,
        // and verify_integrity() would fail with "chain break at seq N"
        // on the very next call.
        let log = AuditLog::new();
        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(120);

        for i in 0..3 {
            push_aged_entry(
                &log,
                "agent-1",
                AuditAction::ToolInvoke,
                &format!("ancient {i}"),
                "ok",
                old_ts,
            );
        }
        // Recent entries that should survive a 90-day retention.
        log.record("agent-1", AuditAction::RoleChange, "fresh", "ok");
        log.record("agent-1", AuditAction::ToolInvoke, "fresher", "ok");

        let last_dropped_hash = log.entries.lock().unwrap()[2].hash.clone();

        let pruned = log.prune(90);
        assert_eq!(pruned, 3);
        assert_eq!(log.len(), 2);
        let anchor = log.chain_anchor.lock().unwrap().clone();
        assert_eq!(
            anchor.as_deref(),
            Some(last_dropped_hash.as_str()),
            "prune must set chain_anchor to the last dropped entry's hash"
        );
        assert!(
            log.verify_integrity().is_ok(),
            "verify_integrity must succeed via chain_anchor after prune"
        );
    }

    #[test]
    fn test_prune_drops_all_persists_consistently_across_restart() {
        // Regression: parity with the trim drop-everything edge case.
        // When every entry is expired, prune must clear the DB tail
        // too — otherwise an orphan row survives in SQLite while the
        // in-memory log is empty, and the next boot's
        // verify_integrity() trips at the orphan.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                user_id TEXT,
                channel TEXT,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));

        let now = chrono::Utc::now();
        let old_ts = now - chrono::Duration::days(120);

        let log = AuditLog::with_db(Arc::clone(&db));
        for i in 0..3 {
            push_aged_entry(
                &log,
                "agent-1",
                AuditAction::ToolInvoke,
                &format!("ancient {i}"),
                "ok",
                old_ts,
            );
        }
        // Re-sync back-dated rows into the DB.
        {
            let entries = log.entries.lock().unwrap();
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM audit_entries", []).unwrap();
            for e in entries.iter() {
                conn.execute(
                    "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        e.seq as i64,
                        &e.timestamp,
                        &e.agent_id,
                        e.action.to_string(),
                        &e.detail,
                        &e.outcome,
                        e.user_id.map(|u| u.to_string()),
                        e.channel.as_deref(),
                        &e.prev_hash,
                        &e.hash,
                    ],
                )
                .unwrap();
            }
        }

        let pruned = log.prune(90);
        assert_eq!(pruned, 3);
        assert_eq!(log.len(), 0);

        let db_count: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM audit_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            db_count, 0,
            "drop-everything prune must clear DB, not leave the tail row behind"
        );

        log.record("system", AuditAction::RoleChange, "post-prune", "ok");
        assert!(log.verify_integrity().is_ok());
        drop(log);

        let log2 = AuditLog::with_db(Arc::clone(&db));
        assert_eq!(log2.len(), 1);
        assert!(
            log2.verify_integrity().is_ok(),
            "verify_integrity must succeed after restart when prune dropped every prior entry"
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

    /// Regression for the chain-break-on-restart class of bugs
    /// (#4078 reproduction): when a SQLite INSERT fails, the in-memory
    /// chain MUST NOT advance.  Previous behaviour (#4050) pushed the
    /// entry into the in-memory buffer regardless and tracked the
    /// failed seq for later in-process retry, but the retry queue lived
    /// only in memory — restart before recovery left an on-disk row
    /// whose `prev_hash` pointed at a never-persisted hash, and every
    /// subsequent boot logged `chain break at seq N`.
    #[test]
    fn test_db_failure_does_not_advance_in_memory_chain() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                user_id TEXT,
                channel TEXT,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            )",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));

        let log = AuditLog::with_db(Arc::clone(&db));
        log.record("a", AuditAction::ToolInvoke, "first", "ok");
        assert_eq!(log.len(), 1);
        let tip_after_first = log.tip_hash();

        // Provoke a transient persistence failure by dropping the
        // table.  The next record() will hit `no such table:
        // audit_entries` from `conn.execute()`.
        db.lock()
            .unwrap()
            .execute("DROP TABLE audit_entries", [])
            .unwrap();

        log.record("a", AuditAction::ToolInvoke, "would-be-lost", "ok");

        assert_eq!(
            log.len(),
            1,
            "in-memory chain must not advance when the DB INSERT fails"
        );
        assert_eq!(
            log.tip_hash(),
            tip_after_first,
            "tip must not advance when the DB INSERT fails"
        );

        // Recreate the table to simulate the operator fixing the DB.
        db.lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE audit_entries (
                    seq INTEGER PRIMARY KEY,
                    timestamp TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    action TEXT NOT NULL,
                    detail TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    user_id TEXT,
                    channel TEXT,
                    prev_hash TEXT NOT NULL,
                    hash TEXT NOT NULL
                )",
            )
            .unwrap();
        // The single seq=0 row from before the drop is gone, but the
        // in-memory entries vector still holds it.  Re-insert by
        // recording a fresh event: we expect seq=1 (entries.last+1).
        // The DB will end up with a single seq=1 row — that's a known
        // gap (the DROP wiped seq=0), but the chain is internally
        // consistent: seq=1's prev_hash = hash(seq=0), and with_db()
        // recovers that as chain_anchor (first entry's prev_hash ≠
        // genesis → anchor = that hash), so verify_integrity() passes.
        log.record("a", AuditAction::ToolInvoke, "after-recovery", "ok");
        assert_eq!(log.len(), 2);
        assert!(log.verify_integrity().is_ok());

        // Restart simulation: a fresh AuditLog reading from the DB
        // sees only the post-recovery row, and verify_integrity must
        // succeed because the chain anchor recovery from `prev_hash`
        // handles the dropped seq=0 prefix.
        drop(log);
        let log2 = AuditLog::with_db(db);
        assert_eq!(
            log2.len(),
            1,
            "DB should hold only the successfully-persisted row"
        );
        assert!(
            log2.verify_integrity().is_ok(),
            "reloaded chain must verify since no broken entry ever reached disk"
        );
    }
}
