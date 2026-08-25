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
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

fn lock_audit_recover<'a, T>(mutex: &'a Mutex<T>, state: &'static str) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::warn!(state, "audit log lock poisoned; recovering inner state");
        // `into_inner()` only unwraps this guard; it does not reset the mutex's poison flag.
        // Without `clear_poison()`, every future access through this helper would re-enter this branch and re-log forever for that same lock.
        mutex.clear_poison();
        poisoned.into_inner()
    })
}

/// Default hard cap on the number of audit entries kept in memory when no
/// operator-supplied `max_in_memory_entries` is configured.
///
/// When `record_with_context` appends an entry that would push the in-memory
/// buffer above this ceiling, the oldest entries are drained from the front so
/// only the most recent `MAX_AUDIT_ENTRIES` survive. This prevents unbounded
/// memory growth in long-running daemons that lack a configured retention
/// policy. The cap applies only to the in-memory window; entries have already
/// been persisted to SQLite before the drain, so forensic completeness is
/// preserved on disk.
///
/// When an operator sets `audit.retention.max_in_memory_entries`, that value
/// (multiplied by `MAX_IN_MEMORY_SOFT_CAP_NUMERATOR / DENOMINATOR`, i.e.
/// `× 1.5`) takes precedence — see [`AuditLog::set_max_in_memory_entries`]
/// and `record_with_context`. This default exists only as a fallback for
/// deployments that have not opted in to a configured cap.
const MAX_AUDIT_ENTRIES: usize = 10_000;

/// Numerator of the soft-cap multiplier applied to the configured
/// `max_in_memory_entries`. The full ratio is
/// `MAX_IN_MEMORY_SOFT_CAP_NUMERATOR / MAX_IN_MEMORY_SOFT_CAP_DENOMINATOR`,
/// i.e. 1.5× by default. The soft cap is enforced inside
/// `record_with_context` so memory stays bounded between scheduled trim
/// cycles (#5665) — without it, an operator that sets
/// `max_in_memory_entries = 5000` would still grow to the 10_000 hard
/// default until the next `trim()` tick fired.
const MAX_IN_MEMORY_SOFT_CAP_NUMERATOR: usize = 3;
const MAX_IN_MEMORY_SOFT_CAP_DENOMINATOR: usize = 2;

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
    /// Bug #7702: an operator repaired a broken chain with [`recovery::reanchor_after_break`].
    /// The entry is the first row of the repaired chain, linked to the last row that still verified, and its detail is a JSON document naming the break, the number of rows severed, and the archive file holding them together with that archive's SHA-256.
    /// Committing the digest here is what makes the preserved rows tamper-evident too: altering the archive after the repair no longer matches the hash the chain vouches for.
    ChainReanchored,
}

impl AuditAction {
    /// The canonical string form of this variant, byte-identical to its
    /// derived `Debug` output (i.e. the variant name). This is the value
    /// persisted in the `audit_entries.action` column and folded into the
    /// per-entry hash via [`Display`], so it must stay stable — renaming a
    /// variant invalidates every persisted hash that mentions it.
    ///
    /// The exhaustive `match` (no wildcard arm) makes the compiler force
    /// coverage: adding a variant to the enum fails to compile until it is
    /// mapped here and in [`FromStr`], which is what prevents the reload
    /// path from silently coercing an unmapped variant to `ToolInvoke`.
    fn as_str(&self) -> &'static str {
        match self {
            AuditAction::ToolInvoke => "ToolInvoke",
            AuditAction::CapabilityCheck => "CapabilityCheck",
            AuditAction::AgentSpawn => "AgentSpawn",
            AuditAction::AgentKill => "AgentKill",
            AuditAction::AgentMessage => "AgentMessage",
            AuditAction::MemoryAccess => "MemoryAccess",
            AuditAction::FileAccess => "FileAccess",
            AuditAction::NetworkAccess => "NetworkAccess",
            AuditAction::ShellExec => "ShellExec",
            AuditAction::AuthAttempt => "AuthAttempt",
            AuditAction::WireConnect => "WireConnect",
            AuditAction::ConfigChange => "ConfigChange",
            AuditAction::DreamConsolidation => "DreamConsolidation",
            AuditAction::UserLogin => "UserLogin",
            AuditAction::RoleChange => "RoleChange",
            AuditAction::PermissionDenied => "PermissionDenied",
            AuditAction::BudgetExceeded => "BudgetExceeded",
            AuditAction::RetentionTrim => "RetentionTrim",
            AuditAction::A2aDiscovered => "A2aDiscovered",
            AuditAction::A2aTrusted => "A2aTrusted",
            AuditAction::ChainReanchored => "ChainReanchored",
        }
    }
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a persisted `action` string does not correspond to any
/// known [`AuditAction`] variant. Surfaced by the reload path so an unknown
/// value is logged by name rather than silently coerced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAuditAction(pub String);

impl std::fmt::Display for UnknownAuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown audit action {:?}", self.0)
    }
}

impl std::error::Error for UnknownAuditAction {}

impl std::str::FromStr for AuditAction {
    type Err = UnknownAuditAction;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
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
            "A2aDiscovered" => AuditAction::A2aDiscovered,
            "A2aTrusted" => AuditAction::A2aTrusted,
            "ChainReanchored" => AuditAction::ChainReanchored,
            other => return Err(UnknownAuditAction(other.to_string())),
        })
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
// than the readability cost of nine plain arguments. As of the delimiter
// fix this writes the v2 layout (all fields tagged); pre-fix entries are
// verified via `compute_entry_hash_legacy`.
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
    // Every field is prefixed with a `\x1f`-delimited tag so byte content
    // cannot be shifted across a field boundary without changing the digest.
    // Without this, the free-form `agent_id` / `detail` / `outcome` strings
    // were hashed back-to-back: `agent_id="a", detail="bc"` and
    // `agent_id="ab", detail="c"` produced identical hashes, letting an
    // attacker with `audit_entries` write access rewrite the field
    // decomposition (e.g. reattribute an action to another agent) while
    // keeping the stored hash — and thus the Merkle link — valid. The
    // `user_id` / `channel` fields already used this scheme; this extends it
    // to the original six. New entries are written with this (v2) layout;
    // [`compute_entry_hash_legacy`] verifies entries written before the
    // change (see `verify_integrity`).
    let mut hasher = Sha256::new();
    hasher.update(b"\x1fseq=");
    hasher.update(seq.to_string().as_bytes());
    hasher.update(b"\x1ftimestamp=");
    hasher.update(timestamp.as_bytes());
    hasher.update(b"\x1fagent_id=");
    hasher.update(agent_id.as_bytes());
    hasher.update(b"\x1faction=");
    hasher.update(action.to_string().as_bytes());
    hasher.update(b"\x1fdetail=");
    hasher.update(detail.as_bytes());
    hasher.update(b"\x1foutcome=");
    hasher.update(outcome.as_bytes());
    if let Some(uid) = user_id {
        hasher.update(b"\x1fuser_id=");
        hasher.update(uid.0.as_bytes());
    }
    if let Some(ch) = channel {
        hasher.update(b"\x1fchannel=");
        hasher.update(ch.as_bytes());
    }
    hasher.update(b"\x1fprev_hash=");
    hasher.update(prev_hash.as_bytes());
    hex::encode(hasher.finalize())
}

/// Pre-delimiter (v1) hash layout: the original six fields concatenated with
/// no separators, then the optionally-tagged `user_id` / `channel`, then a
/// bare `prev_hash`. Retained only so `verify_integrity` can still validate
/// entries written before the delimiter fix — never used to *write* new
/// entries.
///
/// Falling back to this on verify does not weaken tamper-evidence: any edit
/// that changes a stored hash still breaks the linked-list `prev_hash` chain
/// (and, recomputed forward, the external anchor tip). The delimiter fix
/// closes the one residual gap — a field reshuffle that left the hash
/// unchanged — for every entry written under the v2 layout.
#[allow(clippy::too_many_arguments)]
fn compute_entry_hash_legacy(
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
    /// Soft ceiling on the in-memory entry count enforced inside
    /// `record_with_context`, expressed as the operator's configured
    /// `audit.retention.max_in_memory_entries` (#5665). `0` means
    /// "not configured" and falls back to the hard `MAX_AUDIT_ENTRIES`
    /// default. When non-zero, `record_with_context` drops the oldest
    /// non-anchor prefix once `entries.len()` exceeds
    /// `configured × MAX_IN_MEMORY_SOFT_CAP_NUMERATOR /
    /// MAX_IN_MEMORY_SOFT_CAP_DENOMINATOR` so the in-memory window
    /// stays bounded between scheduled `trim()` cycles. Atomic so
    /// `set_max_in_memory_entries` can update it without taking the
    /// `entries` mutex — important because the setter is called from
    /// boot before any append-path contention exists.
    max_in_memory_entries: AtomicUsize,
    /// Running count of rows currently persisted in the `audit_entries`
    /// table — the authoritative population `verify_integrity` compares
    /// the external anchor's `seq` against. It is NOT the same as
    /// `entries.len()`: the soft cap in `record_with_context` drains the
    /// oldest in-memory entries without deleting the corresponding DB
    /// rows, so `entries.len()` tracks the (bounded) in-memory window
    /// while this counts every row still on disk. Seeded from the row
    /// count loaded in `with_db`, incremented on every successful INSERT,
    /// and decremented by the exact number of rows `trim()` / `prune()`
    /// DELETE. Using `entries.len()` for the anchor `seq` desynced it from
    /// the DB after a soft-cap eviction and raised a spurious "audit
    /// anchor mismatch" on the next restart, because the reload
    /// repopulates the full window. Every mutation and read happens under
    /// the `entries` mutex, so `Relaxed` ordering is sufficient.
    persisted_rows: AtomicUsize,
    /// Any failure encountered while reloading persisted rows.
    /// A partially decoded audit trail must never verify as intact merely because the malformed row was skipped by the SQLite iterator.
    load_error: Mutex<Option<String>>,
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

/// The sentinel `prev_hash` of the first entry in a chain: 64 zero characters.
///
/// A chain that has had its prefix trimmed starts at the hash of the last dropped entry instead, so "is this the genesis?" is a real question at several boundaries — boot reload, reconciliation, verification and repair all ask it.
pub(crate) fn genesis_hash() -> String {
    "0".repeat(64)
}

/// A tip hash recovered from the anchor file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnchorRecord {
    seq: u64,
    hash: String,
}

/// The `audit_entries` columns [`decode_audit_row`] expects, in order.
/// Shared by the boot reload in [`AuditLog::with_db`] and the reconciliation re-read in [`AuditLog::read_reconcile_window`] so the two can never drift into decoding different column positions.
const AUDIT_ROW_COLUMNS: &str =
    "seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash";

/// Decode one `audit_entries` row selected with [`AUDIT_ROW_COLUMNS`].
///
/// Schema v22 added the `user_id` / `channel` columns; rows persisted before that migration return NULL for both, which deserialises to `None` and keeps the original hash intact (the hash function omits absent fields, see `compute_entry_hash`).
fn decode_audit_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let action_str: String = row.get(3)?;
    // Decode via `FromStr` (exhaustive over every variant).
    // A genuinely unknown string means the row was written by
    // a newer daemon whose enum this binary does not know; we
    // log it by name rather than silently coercing, because
    // any coercion recomputes a different `action.to_string()`
    // than the persisted one and would trip `verify_integrity`
    // with a false hash mismatch on every subsequent boot.
    let action = action_str.parse::<AuditAction>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let seq_raw: i64 = row.get(0)?;
    let seq =
        u64::try_from(seq_raw).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, seq_raw))?;
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
}

/// Build an entry and its hash from a `(seq, prev_hash)` pair.
///
/// Kept separate from the append path because the pair is only known inside the write transaction (#7702) — the hash covers `seq` and `prev_hash`, so the entry cannot be assembled before the predecessor is read.
#[allow(clippy::too_many_arguments)]
fn build_entry(
    seq: u64,
    timestamp: &str,
    agent_id: &str,
    action: &AuditAction,
    detail: &str,
    outcome: &str,
    user_id: Option<&UserId>,
    channel: Option<&str>,
    prev_hash: String,
) -> AuditEntry {
    let hash = compute_entry_hash(
        seq, timestamp, agent_id, action, detail, outcome, user_id, channel, &prev_hash,
    );
    AuditEntry {
        seq,
        timestamp: timestamp.to_string(),
        agent_id: agent_id.to_string(),
        action: action.clone(),
        detail: detail.to_string(),
        outcome: outcome.to_string(),
        user_id: user_id.copied(),
        channel: channel.map(str::to_string),
        prev_hash,
        hash,
    }
}

/// Outcome of one append attempt.
enum Append {
    /// The row is committed (or this is pure in-memory mode, where memory IS the store).
    /// Boxed so the committed variant does not inflate every `Append` value by the size of a whole entry.
    /// `reconcile` is `Some` when the durable tail had moved underneath this process and the in-memory window must be replaced before the entry is installed.
    Committed {
        entry: Box<AuditEntry>,
        reconcile: Option<Reconcile>,
    },
    /// Nothing reached disk.
    /// Chain state must not advance — see the note on `record_with_context`.
    /// The hash is the one the entry would have carried, returned only to keep the success path's signature.
    Dropped { would_be_hash: String },
}

/// In-memory state re-derived from the database when an append found the durable tail ahead of (or behind) this process's snapshot.
struct Reconcile {
    /// Tail rows as the write transaction saw them, ordered by `seq` ascending and ending at the row the pending entry chains onto.
    window: Vec<AuditEntry>,
    /// Hash of the row preceding `window`, or `None` when the window starts at the genesis sentinel.
    /// Same rule `with_db` applies on boot.
    anchor: Option<String>,
    /// `COUNT(*)` taken inside the same transaction, before the pending INSERT.
    persisted_rows: usize,
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
            max_in_memory_entries: AtomicUsize::new(0),
            persisted_rows: AtomicUsize::new(0),
            load_error: Mutex::new(None),
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

        if log.db.is_none() {
            tracing::error!(
                path = ?anchor_path,
                "Audit anchor verification and initialization skipped because database reload was incomplete"
            );
            return log;
        }

        // Compare against the anchor file (if any) and warn loudly on
        // divergence. The call to `verify_integrity` below will also
        // return `Err` in that case so `/api/audit/verify` surfaces it.
        match Self::read_anchor(&anchor_path) {
            Ok(Some(record)) => {
                let current_tip = lock_audit_recover(&log.tip, "tip").clone();
                let current_seq = lock_audit_recover(&log.entries, "entries").len() as u64;
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
                let tip = lock_audit_recover(&log.tip, "tip").clone();
                let seq = lock_audit_recover(&log.entries, "entries").len() as u64;
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

    /// Update the soft cap on the in-memory window enforced inside
    /// `record_with_context` (#5665).
    ///
    /// `entries` is the operator-supplied
    /// `audit.retention.max_in_memory_entries` from `config.toml`. `0`
    /// disables the configured cap and falls back to the hard
    /// `MAX_AUDIT_ENTRIES` default. The effective ceiling enforced on
    /// every append is `entries × MAX_IN_MEMORY_SOFT_CAP_NUMERATOR /
    /// MAX_IN_MEMORY_SOFT_CAP_DENOMINATOR` (i.e. 1.5× by default), so
    /// the buffer can grow that far between scheduled `trim()` cycles
    /// before the append path itself drops the oldest non-anchor
    /// prefix.
    ///
    /// Audit retention config does NOT hot-reload (see
    /// `config_reload.rs: build_reload_plan`), so this is typically
    /// called once at boot from `with_db_anchored`'s caller. It is
    /// nonetheless atomic so a future hot-reload path or test
    /// scaffolding can flip the cap mid-run safely.
    pub fn set_max_in_memory_entries(&self, entries: usize) {
        self.max_in_memory_entries.store(entries, Ordering::Relaxed);
    }

    /// Soft cap enforced inside `record_with_context`. Returns the
    /// operator-configured value multiplied by the 1.5× safety
    /// headroom, or [`MAX_AUDIT_ENTRIES`] when no cap is configured.
    fn effective_soft_cap(&self) -> usize {
        let configured = self.max_in_memory_entries.load(Ordering::Relaxed);
        if configured == 0 {
            MAX_AUDIT_ENTRIES
        } else {
            configured.saturating_mul(MAX_IN_MEMORY_SOFT_CAP_NUMERATOR)
                / MAX_IN_MEMORY_SOFT_CAP_DENOMINATOR
        }
    }

    /// Creates an audit log backed by a database connection.
    ///
    /// On construction, loads all existing entries from the `audit_entries`
    /// table and verifies the Merkle chain integrity. New entries are written
    /// to both the in-memory chain and the database.
    pub fn with_db(pool: Pool<SqliteConnectionManager>) -> Self {
        let mut entries = Vec::new();
        let mut tip = "0".repeat(64);
        let mut load_error = None;
        let mut persisted_count = 0;

        // Load existing entries from database. Schema v22 added the
        // `user_id` / `channel` columns; rows persisted before that
        // migration return NULL for both, which deserialises to `None`
        // and keeps the original hash intact (the hash function omits
        // absent fields, see `compute_entry_hash`).
        match pool.get() {
            Ok(db) => {
                let result = db.prepare(&format!(
                    "SELECT {AUDIT_ROW_COLUMNS} FROM audit_entries ORDER BY seq ASC"
                ));
                match result {
                    Ok(mut stmt) => {
                        let rows = stmt.query_map([], decode_audit_row);
                        match rows {
                            Ok(rows) => {
                                for (row_index, result) in rows.enumerate() {
                                    persisted_count += 1;
                                    match result {
                                        Ok(entry) => {
                                            tip = entry.hash.clone();
                                            entries.push(entry);
                                        }
                                        Err(error) => {
                                            let message = format!(
                                                "failed to decode audit row at ordered index {row_index}: {error}"
                                            );
                                            tracing::error!(%error, row_index, "Audit reload skipped a malformed row");
                                            load_error.get_or_insert(message);
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                load_error = Some(format!("failed to query audit rows: {error}"));
                            }
                        }
                    }
                    Err(error) => {
                        load_error = Some(format!("failed to prepare audit reload query: {error}"));
                    }
                }
            }
            Err(error) => {
                load_error = Some(format!(
                    "failed to acquire audit database connection: {error}"
                ));
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

        // A partial reload cannot safely derive the next persisted sequence or
        // chain tip. Keep the decoded rows available for inspection, but detach
        // the database so later records cannot overwrite or extend an unknown
        // on-disk tail.
        let db = if load_error.is_some() {
            tracing::error!(
                "Audit database reload was incomplete; durable appends are disabled for this process"
            );
            None
        } else {
            Some(pool)
        };

        let log = Self {
            entries: Mutex::new(entries),
            tip: Mutex::new(tip),
            db,
            anchor_path: None,
            chain_anchor: Mutex::new(recovered_anchor),
            max_in_memory_entries: AtomicUsize::new(0),
            // Count every row yielded by SQLite, including malformed rows that could not be admitted to the in-memory chain.
            persisted_rows: AtomicUsize::new(persisted_count),
            load_error: Mutex::new(load_error.clone()),
        };

        // Verify chain integrity on load. Logged at WARN: the message itself
        // recommends `audit-reset` for the dev case and the loaded chain
        // remains queryable, so this is an alert-worthy condition for
        // compliance operators (who keep a custom WARN→pager rule) but
        // not a daemon error in the dev / single-laptop case where this
        // path fires routinely after every untracked restart. Keeping it
        // at ERROR (the original level) made `grep ERROR daemon.log`
        // useless on dev hosts (#5478).
        if count > 0 || load_error.is_some() {
            if let Err(e) = log.verify_integrity() {
                tracing::warn!(
                    "Audit trail integrity check failed on boot: {e}. \
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
    /// The entry is appended to the chain with the durable tail as its
    /// `prev_hash`, and the tip is advanced to the new hash.
    /// If a database connection is available, the entry is also persisted.
    ///
    /// # The predecessor is read inside the write transaction (#7702)
    ///
    /// `seq` and `prev_hash` come from `SELECT seq, hash FROM audit_entries ORDER BY seq DESC LIMIT 1` issued inside the same `BEGIN IMMEDIATE` transaction as the INSERT — never from this process's in-memory snapshot.
    /// Deriving them before the transaction opened was correct only by coincidence: `seq` and `prev_hash` came from the *same* stale snapshot, so a second writer's INSERT collided on the `seq INTEGER PRIMARY KEY` and failed closed.
    /// That interlock evaporates the moment the row occupying the stale `seq` is deleted while higher rows survive, which the default 90-day retention prune (`DELETE FROM audit_entries WHERE seq < ?1`) does on a schedule with no operator involvement.
    /// The stale writer's INSERT then succeeds carrying a `prev_hash` that names a row which is no longer its predecessor, and the chain forks at a single sequence number — everything below it verifying, one break, everything above it verifying.
    ///
    /// Reading the tail under the RESERVED lock the transaction already holds closes that window by construction: no other writer can commit between the read and the INSERT, because `BEGIN IMMEDIATE` admits one writer at a time across every pooled connection and every process on the file.
    ///
    /// It also un-wedges the loser of a collision.
    /// Previously `entries.last()` never advanced on the drop path and nothing re-read the database, so a process that lost one race retried the same `seq` forever and silently discarded every subsequent audit event for its lifetime.
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

        let mut entries = lock_audit_recover(&self.entries, "entries");
        let mut tip = lock_audit_recover(&self.tip, "tip");

        // What this process believes the tail to be.
        // It is authoritative only in pure in-memory mode and when the table holds no rows at all: after a `trim` / `prune` that dropped everything, `tip` carries the hash of the last dropped entry and the chain has to continue from it rather than restart at the genesis sentinel.
        // Everywhere else it is a hint, and `append_durable` overrules it with what the database actually holds.
        //
        // Derive the next seq from the last entry, not `entries.len()`,
        // because a retention trim may have dropped a prefix — using
        // `len()` would re-issue a seq the surviving entries already hold.
        let mem_seq = entries.last().map(|e| e.seq + 1).unwrap_or(0);
        let mem_prev = tip.clone();

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
        // We invert the trade-off: a transient DB write failure drops the audit event and leaves chain state untouched.
        // The ERROR log inside `append_durable` is the operator's signal to investigate.
        // The next call re-derives its predecessor from the database and tries again with a fresh timestamp.
        let appended = match self.db.as_ref() {
            // Pure in-memory mode: memory IS the source of truth.
            None => Append::Committed {
                entry: Box::new(build_entry(
                    mem_seq,
                    &timestamp,
                    &agent_id,
                    &action,
                    &detail,
                    &outcome,
                    user_id.as_ref(),
                    channel.as_deref(),
                    mem_prev,
                )),
                reconcile: None,
            },
            Some(db) => match db.get() {
                Ok(mut conn) => self.append_durable(
                    &mut conn,
                    mem_seq,
                    &mem_prev,
                    &timestamp,
                    &agent_id,
                    &action,
                    &detail,
                    &outcome,
                    user_id.as_ref(),
                    channel.as_deref(),
                ),
                Err(e) => {
                    metrics::counter!(
                        "librefang_memory_pool_get_failed_total",
                        "store" => "audit",
                        "op" => "record",
                    )
                    .increment(1);
                    tracing::error!(
                        seq = mem_seq,
                        "Audit DB pool get failed ({e:?}); chain NOT advanced."
                    );
                    Append::Dropped {
                        would_be_hash: compute_entry_hash(
                            mem_seq,
                            &timestamp,
                            &agent_id,
                            &action,
                            &detail,
                            &outcome,
                            user_id.as_ref(),
                            channel.as_deref(),
                            &mem_prev,
                        ),
                    }
                }
            },
        };

        let entry = match appended {
            Append::Committed { entry, reconcile } => {
                if let Some(reconcile) = reconcile {
                    // The durable tail moved underneath us, so the window we were holding is no longer a suffix of what is on disk.
                    // Replace it with the rows the write transaction actually saw.
                    // `chain_anchor` becomes the hash of the row before the window (exactly the rule `with_db` uses on boot), so `verify_integrity` walks a true suffix instead of reporting a break that exists only in this process.
                    *entries = reconcile.window;
                    {
                        let mut anchor = lock_audit_recover(&self.chain_anchor, "chain_anchor");
                        *anchor = reconcile.anchor;
                    }
                    self.persisted_rows
                        .store(reconcile.persisted_rows, Ordering::Relaxed);
                }
                entry
            }
            Append::Dropped { would_be_hash } => {
                // Drop locks without mutating; the caller's (discarded) return value is the uncommitted hash, mirroring the success path's signature.
                return would_be_hash;
            }
        };

        let hash = entry.hash.clone();
        entries.push(*entry);
        *tip = hash.clone();

        // The row is now committed to SQLite (or this is pure in-memory
        // mode, where memory IS the store), so it counts toward the
        // persisted population that anchors the external witness. This
        // is deliberately BEFORE the soft-cap drain below: the drain
        // shrinks the in-memory window but leaves the DB rows in place,
        // so `persisted_rows` must keep counting them.
        if self.db.is_some() {
            self.persisted_rows.fetch_add(1, Ordering::Relaxed);
        }

        // Soft cap: if the in-memory buffer grew beyond the configured
        // ceiling (1.5× `max_in_memory_entries` when set, otherwise the
        // hard `MAX_AUDIT_ENTRIES` default), drain the oldest prefix.
        // This pins memory between scheduled `trim()` cycles (#5665) —
        // without it, an operator that sets `max_in_memory_entries =
        // 5000` would still grow to 10_000 (the old hard default)
        // until the next `trim_interval_secs` tick fired.
        //
        // Every entry in `entries` is now known to be persisted on
        // disk (the only path that pushes is the success branch
        // above), so dropping the prefix loses no forensic data — a
        // restart would reload the same rows from SQLite anyway. We
        // update `chain_anchor` to the hash of the last dropped entry
        // so `verify_integrity()` keeps working across the trim
        // boundary.
        let soft_cap = self.effective_soft_cap();
        if entries.len() > soft_cap {
            let overflow = entries.len() - soft_cap;
            let new_anchor = entries[overflow - 1].hash.clone();
            {
                let mut anchor = lock_audit_recover(&self.chain_anchor, "chain_anchor");
                *anchor = Some(new_anchor);
            }
            entries.drain(..overflow);
        }

        // Advance the external anchor so a later DB rewrite is detectable.
        // The anchor stores the persisted-row count — NOT `entries.len()`
        // — so `verify_integrity` compares it against the population that
        // survives a restart. The soft-cap drain above can shrink
        // `entries.len()` below the DB row count; writing the shrunken
        // in-memory length here would desync the anchor `seq` from the
        // rows `with_db` reloads and raise a spurious "audit anchor
        // mismatch" on the next boot. Failures are logged but not
        // propagated — the entry is already in SQLite, and refusing the
        // append because of a filesystem hiccup would lose an audit
        // record, which is strictly worse than an anchor that trails by
        // one tick.
        if self.db.is_some() {
            if let Some(ref anchor_path) = self.anchor_path {
                let count = self.persisted_rows.load(Ordering::Relaxed) as u64;
                if let Err(e) = Self::write_anchor(anchor_path, count, &hash) {
                    tracing::warn!(
                        path = ?anchor_path,
                        "Failed to update audit anchor (entry still persisted): {e}"
                    );
                }
            }
        }

        hash
    }

    /// Derive the predecessor and append one row inside a single `BEGIN IMMEDIATE` transaction (#7702).
    ///
    /// `mem_seq` / `mem_prev` are this process's snapshot.
    /// They are used only when the table holds no rows at all, where they carry the post-trim re-anchoring state the database itself cannot supply; whenever a tail row exists it wins, because it is the only value still true at INSERT time.
    #[allow(clippy::too_many_arguments)]
    fn append_durable(
        &self,
        conn: &mut rusqlite::Connection,
        mem_seq: u64,
        mem_prev: &str,
        timestamp: &str,
        agent_id: &str,
        action: &AuditAction,
        detail: &str,
        outcome: &str,
        user_id: Option<&UserId>,
        channel: Option<&str>,
    ) -> Append {
        // The hash this entry *would* have carried, for the error paths that
        // give up before a predecessor is known. Callers discard it; it exists
        // so the drop paths keep the success path's signature.
        let unpersisted = || Append::Dropped {
            would_be_hash: compute_entry_hash(
                mem_seq, timestamp, agent_id, action, detail, outcome, user_id, channel, mem_prev,
            ),
        };

        // IMMEDIATE acquires a RESERVED lock at the SQLite layer; under WAL
        // the cost over a bare INSERT is negligible (a single fcntl on the
        // lock byte page) but it means at most one writer sits between the
        // tail read and the INSERT at any instant — across pooled
        // connections, background jobs and separate daemon processes alike.
        // That is the invariant the Merkle chain depends on.
        let tx = match conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(
                    seq = mem_seq,
                    error = %e,
                    "Audit DB BEGIN IMMEDIATE failed; chain NOT advanced."
                );
                return unpersisted();
            }
        };

        // The durable predecessor, read under the write lock this transaction
        // already holds. `ORDER BY seq DESC LIMIT 1` is a single seek to the
        // last leaf of the `seq INTEGER PRIMARY KEY` btree.
        let tail = tx
            .query_row(
                "SELECT seq, hash FROM audit_entries ORDER BY seq DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional();

        let (seq, prev_hash, diverged) = match tail {
            Ok(Some((tail_seq, tail_hash))) => {
                let Ok(tail_seq) = u64::try_from(tail_seq) else {
                    tracing::error!(
                        tail_seq,
                        "Audit tail row carries a negative seq; chain NOT advanced."
                    );
                    return unpersisted();
                };
                let next = tail_seq.saturating_add(1);
                let diverged = next != mem_seq || tail_hash != mem_prev;
                (next, tail_hash, diverged)
            }
            // Empty table. This is not a divergence: `trim` / `prune` can
            // legitimately delete every row, and the chain then continues
            // from the in-memory tip (the hash of the last dropped entry) so
            // the recovered `chain_anchor` still links the surviving suffix.
            // Restarting at the genesis sentinel here would silently rewrite
            // history instead.
            Ok(None) => (mem_seq, mem_prev.to_string(), false),
            Err(e) => {
                tracing::error!(
                    seq = mem_seq,
                    error = %e,
                    "Audit tail read failed; chain NOT advanced."
                );
                return unpersisted();
            }
        };

        // Re-read the window before the INSERT, so it holds exactly the rows
        // this entry chains onto and `COUNT(*)` excludes the row we are about
        // to add (the caller increments `persisted_rows` for that one).
        let reconcile = if diverged {
            tracing::warn!(
                expected_seq = mem_seq,
                durable_seq = seq,
                expected_prev = %mem_prev,
                durable_prev = %prev_hash,
                "Audit chain tail moved underneath this process — another writer holds the same \
                 database. Chaining onto the durable tail and re-reading the in-memory window."
            );
            Some(self.read_reconcile_window(&tx, &prev_hash))
        } else {
            None
        };

        let entry = build_entry(
            seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash,
        );

        let inserted = tx.execute(
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

        match inserted.and_then(|_| tx.commit()) {
            Ok(_) => Append::Committed {
                entry: Box::new(entry),
                reconcile,
            },
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
                Append::Dropped {
                    would_be_hash: entry.hash,
                }
            }
        }
    }

    /// Read a bounded tail window plus the authoritative row count inside the append transaction, for the case where the durable tail no longer matches this process's in-memory view.
    ///
    /// `prev_hash` is the tail hash the pending entry chains onto.
    /// It doubles as the fallback anchor when the window cannot be read, in which case the window is emptied rather than left stale — an empty window anchored at the real predecessor still verifies, a stale one does not.
    fn read_reconcile_window(&self, tx: &rusqlite::Transaction<'_>, prev_hash: &str) -> Reconcile {
        let genesis = "0".repeat(64);
        let degraded = |error: rusqlite::Error| {
            tracing::warn!(
                %error,
                "Audit window re-read failed; continuing with an empty in-memory window anchored \
                 at the durable tail. The rows are still on disk and reload on the next boot."
            );
            Reconcile {
                window: Vec::new(),
                anchor: (prev_hash != genesis).then(|| prev_hash.to_string()),
                persisted_rows: self.persisted_rows.load(Ordering::Relaxed),
            }
        };

        // Bound the re-read by the same ceiling the append path enforces, so
        // reconciliation can never allocate a larger window than steady-state
        // operation would.
        // `try_from` rather than `as`: a negative LIMIT means "unbounded" to
        // SQLite, so a wrapping cast would quietly remove the bound it exists
        // to impose.
        let limit = i64::try_from(self.effective_soft_cap().max(1)).unwrap_or(i64::MAX);
        let window = (|| -> rusqlite::Result<Vec<AuditEntry>> {
            let mut stmt = tx.prepare(&format!(
                "SELECT {AUDIT_ROW_COLUMNS} FROM audit_entries ORDER BY seq DESC LIMIT ?1"
            ))?;
            let mut rows = stmt
                .query_map(rusqlite::params![limit], decode_audit_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.reverse();
            Ok(rows)
        })();

        let mut window = match window {
            Ok(window) => window,
            Err(e) => return degraded(e),
        };

        let persisted_rows = match tx.query_row("SELECT COUNT(*) FROM audit_entries", [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(count) => usize::try_from(count).unwrap_or(0),
            Err(e) => return degraded(e),
        };

        // The window has to end at the row the pending entry chains onto,
        // otherwise the in-memory chain would not be contiguous. Under
        // `BEGIN IMMEDIATE` nothing can commit between the two reads, so a
        // mismatch means a read we should not trust rather than a race.
        if prev_hash != genesis && window.last().map(|e| e.hash.as_str()) != Some(prev_hash) {
            tracing::warn!(
                "Audit window re-read did not end at the durable tail; using an empty window."
            );
            window.clear();
        }

        let anchor = match window.first() {
            Some(first) if first.prev_hash != genesis => Some(first.prev_hash.clone()),
            Some(_) => None,
            None => (prev_hash != genesis).then(|| prev_hash.to_string()),
        };

        Reconcile {
            window,
            anchor,
            persisted_rows,
        }
    }

    /// Walks the entire chain and recomputes every hash to detect tampering.
    ///
    /// Returns `Ok(())` if the chain is intact, or `Err(msg)` describing
    /// the first inconsistency found.
    pub fn verify_integrity(&self) -> Result<(), String> {
        if let Some(error) = lock_audit_recover(&self.load_error, "load_error").as_ref() {
            return Err(format!("audit trail was only partially loaded: {error}"));
        }

        let entries = lock_audit_recover(&self.entries, "entries");
        // When the retention trim job has dropped a prefix, the first
        // surviving entry's `prev_hash` points at the last dropped
        // entry rather than the genesis sentinel. Seed the walk from
        // the chain anchor so the trim boundary verifies cleanly.
        let anchor = lock_audit_recover(&self.chain_anchor, "chain_anchor").clone();
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

            // Accept the current (delimited, v2) layout, falling back to the
            // pre-delimiter (v1) layout for entries written before the fix so
            // an upgrade does not raise false tamper alarms on existing logs.
            let matches = recomputed == entry.hash
                || compute_entry_hash_legacy(
                    entry.seq,
                    &entry.timestamp,
                    &entry.agent_id,
                    &entry.action,
                    &entry.detail,
                    &entry.outcome,
                    entry.user_id.as_ref(),
                    entry.channel.as_deref(),
                    &entry.prev_hash,
                ) == entry.hash;

            if !matches {
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
                                                             // `seq` in the anchor is the number of rows persisted
                                                             // at the time it was last written. Compare it against
                                                             // the persisted-row count, NOT `entries.len()`: the
                                                             // soft cap in `record_with_context` drains the
                                                             // in-memory window without deleting DB rows, so
                                                             // `entries.len()` under-counts the population a restart
                                                             // reloads. Using it here would raise a spurious anchor
                                                             // mismatch after the next boot even though the chain is
                                                             // intact.
                    let persisted = self.persisted_rows.load(Ordering::Relaxed) as u64;
                    if record.seq != persisted || record.hash != current_tip {
                        return Err(format!(
                            "audit anchor mismatch: anchor says seq={} tip={} \
                             but DB has rows={} tip={}",
                            record.seq, record.hash, persisted, current_tip
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
        lock_audit_recover(&self.tip, "tip").clone()
    }

    /// Returns the number of entries in the log.
    pub fn len(&self) -> usize {
        lock_audit_recover(&self.entries, "entries").len()
    }

    /// Returns the number of rows retained in persistent storage.
    ///
    /// This can exceed [`Self::len`] after the in-memory soft cap evicts an
    /// old prefix without deleting its SQLite rows. Callers that can only
    /// inspect the in-memory window use this value to disclose that their
    /// result is incomplete rather than presenting a partial audit history
    /// as exhaustive.
    pub fn persisted_len(&self) -> usize {
        self.persisted_rows.load(Ordering::Relaxed)
    }

    /// Returns the complete retained in-memory window and persisted row count from one consistent snapshot.
    ///
    /// Append, trim, and prune paths update both values while holding the entries mutex.
    /// Sampling the count under the same mutex prevents callers from misclassifying a concurrent append or eviction as a history gap.
    pub fn retained_snapshot(&self) -> (Vec<AuditEntry>, usize) {
        let entries = lock_audit_recover(&self.entries, "entries");
        let persisted_rows = self.persisted_rows.load(Ordering::Relaxed);
        (entries.clone(), persisted_rows)
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
        lock_audit_recover(&self.entries, "entries").is_empty()
    }

    /// Returns up to the most recent `n` entries (cloned).
    pub fn recent(&self, n: usize) -> Vec<AuditEntry> {
        let entries = lock_audit_recover(&self.entries, "entries");
        let start = entries.len().saturating_sub(n);
        entries[start..].to_vec()
    }

    /// Counts every retained non-success outcome for one agent.
    ///
    /// Persistent logs use the `audit_entries(agent_id, timestamp)` index
    /// instead of cloning and scanning the bounded in-memory window. An
    /// in-memory-only log has no durable history, so its retained entries are
    /// the authoritative source.
    pub fn count_agent_errors(&self, agent_id: &str) -> Result<u64, String> {
        let Some(pool) = self.db.as_ref() else {
            let entries = lock_audit_recover(&self.entries, "entries");
            return Ok(entries
                .iter()
                .filter(|entry| {
                    entry.agent_id == agent_id
                        && entry.outcome != "ok"
                        && entry.outcome != "success"
                })
                .count() as u64);
        };

        let db = pool
            .get()
            .map_err(|error| format!("failed to acquire audit database connection: {error}"))?;
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_entries \
                 WHERE agent_id = ?1 AND outcome != 'ok' AND outcome != 'success'",
                [agent_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("failed to count agent audit errors: {error}"))?;
        u64::try_from(count).map_err(|_| "agent audit error count was negative".to_string())
    }

    /// Returns one newest-first page of audit entries for an agent.
    ///
    /// Persistent logs are filtered and paginated in SQLite. Schema v41's
    /// `(agent_id, timestamp)` index makes the work proportional to the
    /// requested agent page rather than the global audit population.
    pub fn recent_for_agent(
        &self,
        agent_id: &str,
        outcome: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, String> {
        let Some(pool) = self.db.as_ref() else {
            let entries = lock_audit_recover(&self.entries, "entries");
            return Ok(entries
                .iter()
                .rev()
                .filter(|entry| entry.agent_id == agent_id)
                .filter(|entry| {
                    outcome.is_none_or(|value| entry.outcome.eq_ignore_ascii_case(value))
                })
                .skip(offset)
                .take(limit)
                .cloned()
                .collect());
        };

        let db = pool
            .get()
            .map_err(|error| format!("failed to acquire audit database connection: {error}"))?;
        let offset = i64::try_from(offset)
            .map_err(|_| "agent audit offset exceeds SQLite range".to_string())?;
        let limit = i64::try_from(limit)
            .map_err(|_| "agent audit limit exceeds SQLite range".to_string())?;
        let sql = if outcome.is_some() {
            "SELECT seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash \
             FROM audit_entries WHERE agent_id = ?1 AND outcome = ?2 COLLATE NOCASE \
             ORDER BY timestamp DESC, seq DESC LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash \
             FROM audit_entries WHERE agent_id = ?1 \
             ORDER BY timestamp DESC, seq DESC LIMIT ?2 OFFSET ?3"
        };
        let mut stmt = db
            .prepare(sql)
            .map_err(|error| format!("failed to prepare agent audit query: {error}"))?;
        let decode = |row: &rusqlite::Row<'_>| {
            let action_str: String = row.get(3)?;
            let action = action_str.parse::<AuditAction>().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            let seq_raw: i64 = row.get(0)?;
            let seq = u64::try_from(seq_raw)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, seq_raw))?;
            let user_id_str: Option<String> = row.get(6)?;
            Ok(AuditEntry {
                seq,
                timestamp: row.get(1)?,
                agent_id: row.get(2)?,
                action,
                detail: row.get(4)?,
                outcome: row.get(5)?,
                user_id: user_id_str.as_deref().and_then(|value| value.parse().ok()),
                channel: row.get(7)?,
                prev_hash: row.get(8)?,
                hash: row.get(9)?,
            })
        };
        let rows = match outcome {
            Some(outcome) => {
                stmt.query_map(rusqlite::params![agent_id, outcome, limit, offset], decode)
            }
            None => stmt.query_map(rusqlite::params![agent_id, limit, offset], decode),
        }
        .map_err(|error| format!("failed to query agent audit entries: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to decode agent audit entry: {error}"))
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
        let entries = lock_audit_recover(&self.entries, "entries");
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
        let mut entries = lock_audit_recover(&self.entries, "entries");

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
        // Persist-before-mutate: only drop the in-memory prefix if the DB
        // delete actually succeeded. If `db.get()` or the `DELETE` fails and we
        // trimmed memory anyway, a restart would reload the un-deleted rows —
        // resurrecting the entries retention was supposed to remove and
        // desyncing the anchor seq from the DB. On failure, keep memory intact
        // and report nothing dropped so the trim retries on the next tick.
        if let Some(ref db) = self.db {
            match db.get() {
                Ok(conn) => {
                    match conn.execute(
                        "DELETE FROM audit_entries WHERE seq < ?1",
                        rusqlite::params![first_survivor_seq as i64],
                    ) {
                        // Decrement the persisted-row count by the rows the
                        // DELETE actually removed — not `drop_count`, which
                        // only counts in-memory survivors. A prior soft-cap
                        // eviction can leave DB rows below the in-memory
                        // window, so `seq < first_survivor_seq` may delete
                        // more rows than were held in memory.
                        Ok(deleted) => {
                            self.persisted_rows.fetch_sub(deleted, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::error!(
                                "Audit trim DELETE failed ({e}); keeping the in-memory window \
                                 consistent with the DB — retrying on the next trim tick"
                            );
                            return TrimReport::default();
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Audit trim could not acquire a DB connection ({e}); keeping the \
                         in-memory window consistent with the DB — retrying on the next tick"
                    );
                    return TrimReport::default();
                }
            }
        }

        // Mutate in-memory state. Order matters: anchor before drain
        // so a concurrent verify_integrity (blocked on the entries
        // lock) sees a consistent (anchor, first_survivor) pair when
        // it acquires.
        {
            let mut anchor = lock_audit_recover(&self.chain_anchor, "chain_anchor");
            *anchor = report.new_chain_anchor.clone();
        }
        entries.drain(..drop_count);

        // Refresh the external anchor file so its `seq` column tracks
        // the new (post-trim) persisted-row count. The tip hash itself
        // does NOT change — trimming a prefix never moves the tail — but
        // the count does, and `verify_integrity` insists they agree.
        // Failing to rewrite the anchor here would surface as a spurious
        // "audit anchor mismatch" on the very next verification.
        if let Some(ref anchor_path) = self.anchor_path {
            let new_count = self.persisted_rows.load(Ordering::Relaxed) as u64;
            let tip = lock_audit_recover(&self.tip, "tip").clone();
            if let Err(e) = Self::write_anchor(anchor_path, new_count, &tip) {
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

        let mut entries = lock_audit_recover(&self.entries, "entries");
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

        let new_anchor = entries[drop_count - 1].hash.clone();

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
        // Persist-before-mutate: keep the in-memory window only if the DB
        // delete succeeded (see AuditLog::trim). On failure, leave memory and
        // the DB consistent and report nothing pruned so the next prune retries.
        if let Some(ref db) = self.db {
            match db.get() {
                Ok(conn) => {
                    match conn.execute(
                        "DELETE FROM audit_entries WHERE seq < ?1",
                        rusqlite::params![first_survivor_seq as i64],
                    ) {
                        // Decrement by the rows actually removed (see the
                        // matching note in `trim`): a prior soft-cap
                        // eviction can leave DB rows below the in-memory
                        // window, so the DELETE may remove more rows than
                        // `drop_count`.
                        Ok(deleted) => {
                            self.persisted_rows.fetch_sub(deleted, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::error!(
                                "Audit prune DELETE failed ({e}); keeping the in-memory window \
                                 consistent with the DB — retrying on the next prune"
                            );
                            return 0;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Audit prune could not acquire a DB connection ({e}); keeping the \
                         in-memory window consistent with the DB — retrying on the next prune"
                    );
                    return 0;
                }
            }
        }

        // Mutate in-memory state only after the DB delete succeeded
        // (mirrors `AuditLog::trim`). Order matters: anchor before drain
        // so a verify racing against this prune (blocked on the entries
        // lock) sees a consistent (anchor, first_survivor) pair on the
        // next acquire. Advancing the anchor before the DB block would
        // leave a failed DELETE with un-drained entries whose
        // `entries[0].prev_hash` no longer matches the anchor —
        // verify_integrity would then raise a spurious "chain break".
        {
            let mut anchor = lock_audit_recover(&self.chain_anchor, "chain_anchor");
            *anchor = Some(new_anchor);
        }
        entries.drain(..drop_count);

        // Refresh the external anchor file's `seq` column so the next
        // verify_integrity() does not trip the "anchor seq mismatch"
        // guard. The seq tracks the persisted-row count, not
        // `entries.len()` (see the note in `record_with_context`). Tip
        // itself does not move (we only drop a prefix).
        if let Some(ref anchor_path) = self.anchor_path {
            let new_count = self.persisted_rows.load(Ordering::Relaxed) as u64;
            let tip = lock_audit_recover(&self.tip, "tip").clone();
            if let Err(e) = Self::write_anchor(anchor_path, new_count, &tip) {
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

pub mod recovery;

#[cfg(test)]
mod tests;
