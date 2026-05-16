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
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;

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
    /// Bug #3786: an external A2A agent card was fetched into the pending
    /// list via `POST /api/a2a/discover`. Detail carries the discovery URL
    /// and the card's self-declared name (which is unverified at this
    /// point). The agent cannot receive tasks until promoted via
    /// `A2aTrusted`.
    A2aDiscovered,
    /// Bug #3786: a pending A2A agent was promoted into the trusted list
    /// by an operator via `POST /api/a2a/agents/{id}/approve`. Detail
    /// carries the URL and agent name. Subsequent `/api/a2a/send` and
    /// `/api/a2a/tasks/.../status` calls to that URL are now permitted.
    A2aTrusted,
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
    /// Optional connection pool for persistent storage.
    db: Option<Pool<SqliteConnectionManager>>,
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
    pub fn with_db_anchored(
        pool: Pool<SqliteConnectionManager>,
        anchor_path: std::path::PathBuf,
    ) -> Self {
        let mut log = Self::with_db(pool);
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
    pub fn with_db(pool: Pool<SqliteConnectionManager>) -> Self {
        let mut entries = Vec::new();
        let mut tip = "0".repeat(64);

        // Load existing entries from database. Schema v22 added the
        // `user_id` / `channel` columns; rows persisted before that
        // migration return NULL for both, which deserialises to `None`
        // and keeps the original hash intact (the hash function omits
        // absent fields, see `compute_entry_hash`).
        if let Ok(db) = pool.get() {
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
            db: Some(pool),
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
        // The append is wrapped in `BEGIN IMMEDIATE` so the chain stays
        // intact even if a future refactor narrows the `entries` /
        // `tip` Mutex scope, and so concurrent processes (or background
        // jobs holding their own pooled connection) cannot interleave
        // an append against the same `prev_hash`. IMMEDIATE acquires a
        // RESERVED lock at the SQLite layer; under WAL the cost over a
        // bare INSERT is negligible (a single fcntl on the lock byte
        // page) but it means at most one writer is between
        // `prev_hash` derivation and INSERT at any instant — which is
        // the invariant the Merkle chain depends on. See the
        // `audit_chain_holds_under_concurrent_record` test below for
        // the regression bound.
        let persisted = match self.db.as_ref() {
            None => true, // pure in-memory mode: memory IS the source of truth
            Some(db) => match db.get() {
                Ok(mut conn) => {
                    let tx_result =
                        conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate);
                    match tx_result {
                        Ok(tx) => {
                            let exec_result = tx.execute(
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
                            );
                            match exec_result.and_then(|_| tx.commit()) {
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
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                seq = entry.seq,
                                error = %e,
                                "Audit DB BEGIN IMMEDIATE failed; chain NOT advanced."
                            );
                            false
                        }
                    }
                }
                Err(e) => {
                    metrics::counter!(
                        "librefang_memory_pool_get_failed_total",
                        "store" => "audit",
                        "op" => "record",
                    )
                    .increment(1);
                    tracing::error!(
                        seq = entry.seq,
                        "Audit DB pool get failed ({e:?}); chain NOT advanced."
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
                let mut anchor = self.chain_anchor.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Returns the configured external tip-anchor path, if any.
    ///
    /// When `Some`, every audit append mirrors the new tip hash to this
    /// file (see [`Self::with_db_anchored`]) and `verify_integrity()`
    /// fails closed when the on-disk tip diverges from the in-DB tip.
    /// When `None`, the chain is self-consistent only — see SECURITY.md.
    pub fn anchor_path(&self) -> Option<&std::path::Path> {
        self.anchor_path.as_deref()
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

    /// Returns every entry with `seq > cursor`, in insertion order.
    ///
    /// Intended for cursor-based streaming consumers — e.g. the
    /// `/api/logs/stream` SSE endpoint — that need to deliver every
    /// entry produced since the last poll without dropping any when the
    /// production rate exceeds [`Self::recent`]'s sliding window.
    ///
    /// **Strictly greater than:** the cursor is the highest seq the
    /// consumer has already received, so `since_seq(N)` returns seq > N
    /// (never seq >= N). This means `since_seq(0)` skips an entry with
    /// seq=0 — that initial backfill must be handled separately via
    /// [`Self::recent`] before the cursor loop kicks in. The SSE
    /// handler does exactly that on its first poll.
    ///
    /// O(log n) seek + O(k) clone, where `k` is the number of returned
    /// entries. Relies on the invariant that entries are appended in
    /// strictly increasing `seq` order; `record_with_context` is the
    /// only mutator and it monotonically allocates `seq` before push.
    pub fn since_seq(&self, cursor: u64) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let idx = entries.partition_point(|e| e.seq <= cursor);
        entries[idx..].to_vec()
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
            if let Ok(conn) = db.get() {
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
            if let Ok(conn) = db.get() {
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
mod tests;
