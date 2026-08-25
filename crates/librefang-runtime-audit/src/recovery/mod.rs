//! Offline repair for an `audit_entries` table whose Merkle chain has already broken (#7702).
//!
//! # Why a broken chain needs a tool at all
//!
//! [`AuditLog::verify_integrity`](crate::AuditLog::verify_integrity) walks the persisted rows in `seq` order and stops at the first row whose `prev_hash` does not name its predecessor.
//! Once that break exists, every later boot reports the same failure and the tamper-evidence property is degraded for the whole table, not just for the rows around the break.
//! The only recovery LibreFang shipped before this module was `librefang security audit-reset`, whose single mode is `DELETE FROM audit_entries` — it restores verification by destroying the evidence, which is the opposite of what an audit trail is for.
//!
//! # What repair can and cannot do
//!
//! A Merkle chain admits exactly one predecessor per row.
//! When two independently derived chains have been merged into one table — the shape #7847 diagnosed and then prevented — no rewrite short of forging hashes makes both segments members of the same chain.
//! So repair necessarily severs one segment, and the only question is whether the severed rows are destroyed or preserved.
//!
//! [`reanchor_after_break`] preserves them.
//! The rows from the break forward are written verbatim to a JSON Lines archive *before* anything is deleted, the archive's SHA-256 is committed into the chain via a [`AuditAction::ChainReanchored`](crate::AuditAction::ChainReanchored) marker entry, and only then are the rows removed.
//! The pre-break history keeps verifying, the post-break rows remain readable on disk, and the marker makes the repair itself an audited event whose archive cannot be altered without breaking the chain it is recorded in.
//!
//! # Relationship to the prevention fix
//!
//! #7847 made `record_with_context` derive `seq` and `prev_hash` inside the same `BEGIN IMMEDIATE` transaction as the INSERT, so a stale-snapshot append can no longer fork the chain.
//! That protects logs going forward; it does nothing for a log that is already broken.
//! This module is the repair half.

use crate::{
    build_entry, compute_entry_hash, compute_entry_hash_legacy, decode_audit_row, genesis_hash,
    AuditAction, AuditEntry, AuditLog, AUDIT_ROW_COLUMNS,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Directory, relative to the daemon's `data_dir`, that holds the JSON Lines archives [`reanchor_after_break`] writes.
///
/// Exported so callers place archives where the operator documentation says they are rather than each spelling the name themselves.
pub const ARCHIVE_DIR_NAME: &str = "audit-archive";

/// Which of the two chain invariants a [`ChainBreak`] violates.
///
/// The distinction matters operationally: a `PrevHashMismatch` means the *linkage* between two rows is wrong (the fork shape #7847 describes), while a `HashMismatch` means a row's own stored hash does not match its content — the fingerprint of an edit to the row itself.
/// Both are reported at the first offending row, in the same order [`AuditLog::verify_integrity`](crate::AuditLog::verify_integrity) checks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainBreakKind {
    /// The row's `prev_hash` does not name the hash of the row before it.
    PrevHashMismatch,
    /// The row's stored `hash` is not the hash its own content produces.
    HashMismatch,
}

impl ChainBreakKind {
    /// Stable identifier used in the marker entry's detail payload and in CLI output.
    pub fn as_str(self) -> &'static str {
        match self {
            ChainBreakKind::PrevHashMismatch => "PrevHashMismatch",
            ChainBreakKind::HashMismatch => "HashMismatch",
        }
    }
}

impl std::fmt::Display for ChainBreakKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The first place the persisted chain stops verifying.
///
/// `expected` / `found` carry the two hashes that disagree, so an operator can compare them against the `WARN` the daemon logged on boot without re-deriving anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainBreak {
    /// `seq` of the first row that fails to verify.
    pub seq: u64,
    /// Which invariant it violates.
    pub kind: ChainBreakKind,
    /// The hash the walk expected at this point.
    pub expected: String,
    /// The hash the row actually carries.
    pub found: String,
}

impl std::fmt::Display for ChainBreak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ChainBreakKind::PrevHashMismatch => write!(
                f,
                "chain break at seq {}: expected prev_hash {} but found {}",
                self.seq, self.expected, self.found
            ),
            ChainBreakKind::HashMismatch => write!(
                f,
                "hash mismatch at seq {}: expected {} but found {}",
                self.seq, self.expected, self.found
            ),
        }
    }
}

/// What [`reanchor_after_break`] would do, computed without mutating anything.
///
/// Produced by [`plan_reanchor`] so the CLI can render a dry run whose numbers come from the same query the repair uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReanchorPlan {
    /// The break the repair would act on.
    pub break_point: ChainBreak,
    /// Rows currently in `audit_entries`.
    pub rows_before: usize,
    /// Rows at or after the break — archived, then removed.
    pub severed_rows: usize,
    /// Rows below the break — kept, and still verifying.
    pub surviving_rows: usize,
}

/// Outcome of a completed repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReanchorReport {
    /// The break that was repaired.
    pub break_point: ChainBreak,
    /// How many rows were archived and removed.
    pub severed_rows: usize,
    /// Where the severed rows were written.
    pub archive_path: PathBuf,
    /// SHA-256 of the archive file's bytes, also committed into the marker entry.
    pub archive_sha256: String,
    /// `seq` assigned to the `ChainReanchored` marker.
    pub marker_seq: u64,
    /// Hash of the marker entry — the chain's new tip.
    pub tip: String,
    /// `COUNT(*)` taken inside the repair transaction, after the delete and the marker INSERT.
    ///
    /// This — not `marker_seq + 1` — is what the anchor file's first column holds, because the anchor contract is the persisted row *population*.
    /// The two diverge on any database whose prefix has been pruned by retention, which the 90-day default does unattended.
    pub persisted_rows: usize,
    /// Whether the external anchor file was rewritten (false when no anchor path was supplied).
    pub anchor_written: bool,
}

/// Walk the persisted chain and return the first break, or `None` when it verifies.
///
/// The walk is seeded from the first row's own `prev_hash`, exactly as [`AuditLog::with_db`](crate::AuditLog::with_db) seeds `chain_anchor` on boot.
/// A retention prune legitimately removes a prefix, and the surviving suffix then starts at a non-genesis `prev_hash` that names the last dropped row; treating that as a break would report every pruned database as corrupt.
/// One consequence worth stating: the first row can never fail the `prev_hash` check, so a `PrevHashMismatch` always has at least one surviving predecessor.
pub fn diagnose_chain(conn: &Connection) -> Result<Option<ChainBreak>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {AUDIT_ROW_COLUMNS} FROM audit_entries ORDER BY seq ASC"
        ))
        .map_err(|e| format!("cannot prepare audit chain scan: {e}"))?;
    let rows = stmt
        .query_map([], decode_audit_row)
        .map_err(|e| format!("cannot scan audit_entries: {e}"))?;

    let mut expected_prev: Option<String> = None;
    for (index, row) in rows.enumerate() {
        let entry =
            row.map_err(|e| format!("cannot decode audit row at ordered index {index}: {e}"))?;
        let expected = expected_prev
            .get_or_insert_with(|| entry.prev_hash.clone())
            .clone();

        if entry.prev_hash != expected {
            return Ok(Some(ChainBreak {
                seq: entry.seq,
                kind: ChainBreakKind::PrevHashMismatch,
                expected,
                found: entry.prev_hash,
            }));
        }

        // Accept the current (delimited, v2) layout, falling back to the pre-delimiter (v1) layout, on the same terms `verify_integrity` does.
        // Rejecting a v1 row here would make the repair tool report a break on every log old enough to predate the delimiter fix.
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
        let legacy = || {
            compute_entry_hash_legacy(
                entry.seq,
                &entry.timestamp,
                &entry.agent_id,
                &entry.action,
                &entry.detail,
                &entry.outcome,
                entry.user_id.as_ref(),
                entry.channel.as_deref(),
                &entry.prev_hash,
            )
        };
        if recomputed != entry.hash && legacy() != entry.hash {
            return Ok(Some(ChainBreak {
                seq: entry.seq,
                kind: ChainBreakKind::HashMismatch,
                expected: recomputed,
                found: entry.hash,
            }));
        }

        expected_prev = Some(entry.hash);
    }

    Ok(None)
}

/// Describe the repair [`reanchor_after_break`] would perform, without touching the database.
///
/// Returns `None` when the chain verifies and there is nothing to repair.
pub fn plan_reanchor(conn: &Connection) -> Result<Option<ReanchorPlan>, String> {
    let Some(break_point) = diagnose_chain(conn)? else {
        return Ok(None);
    };
    let rows_before = count_rows(conn)?;
    let severed_rows = conn
        .query_row(
            "SELECT COUNT(*) FROM audit_entries WHERE seq >= ?1",
            rusqlite::params![break_point.seq as i64],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| format!("cannot count severed rows: {e}"))?;
    let severed_rows = usize::try_from(severed_rows)
        .map_err(|_| format!("severed row count is negative: {severed_rows}"))?;
    Ok(Some(ReanchorPlan {
        break_point,
        rows_before,
        severed_rows,
        surviving_rows: rows_before.saturating_sub(severed_rows),
    }))
}

/// Repair the chain at `expected`, preserving the severed rows.
///
/// Ordering is chosen so no step can destroy evidence it has not already copied:
///
/// 1. `BEGIN IMMEDIATE`, which takes the same RESERVED lock the append path takes — no other writer, in this process or any other, can commit between the diagnosis and the delete.
/// 2. Re-diagnose **inside** that transaction and abort unless the break matches `expected` field for field. The caller diagnosed earlier, outside any lock; acting on that stale value is exactly the "derive state, then mutate without re-reading" pattern that produced the bug being repaired.
/// 3. Write the rows at or after the break to `archive_dir` as JSON Lines and fsync them.
/// 4. Delete those rows, append the `ChainReanchored` marker chained onto the last survivor, and take `COUNT(*)` — all still inside the transaction, so the count cannot be stale by the time it reaches the anchor.
/// 5. Commit, then rewrite the anchor file through [`AuditLog::write_anchor`], the same temp-file-plus-rename writer the daemon uses, so the anchor the next boot reads is in the format the reader accepts.
///
/// A failure after step 3 leaves the archive file behind with the database untouched.
/// That is deliberate: a stray archive costs disk, a missing one costs the evidence.
pub fn reanchor_after_break(
    conn: &mut Connection,
    archive_dir: &Path,
    anchor_path: Option<&Path>,
    expected: &ChainBreak,
) -> Result<ReanchorReport, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("cannot open the audit repair transaction: {e}"))?;

    let current = diagnose_chain(&tx)
        .map_err(|e| format!("re-diagnosis inside the repair transaction failed: {e}"))?;
    match current {
        None => {
            return Err(
                "the chain verifies now — the break was repaired or the database changed since it was diagnosed; nothing was modified".to_string(),
            );
        }
        Some(ref found) if found != expected => {
            return Err(format!(
                "the break moved since it was diagnosed: expected `{expected}`, found `{found}`; nothing was modified"
            ));
        }
        Some(_) => {}
    }

    let severed = severed_entries(&tx, expected.seq)?;
    let (archive_path, archive_sha256) = write_archive(archive_dir, expected.seq, &severed)?;

    tx.execute(
        "DELETE FROM audit_entries WHERE seq >= ?1",
        rusqlite::params![expected.seq as i64],
    )
    .map_err(|e| format!("cannot remove the severed rows: {e}"))?;

    let tail = tx
        .query_row(
            "SELECT seq, hash FROM audit_entries ORDER BY seq DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| format!("cannot read the surviving tail: {e}"))?;

    // A `HashMismatch` on the very first row severs everything, leaving an empty table.
    // The marker then starts a fresh chain from the genesis sentinel — the archive still holds every row, so this is a re-anchor rather than the data loss `audit-reset` performs.
    let (marker_seq, marker_prev) = match tail {
        Some((tail_seq, tail_hash)) => {
            let tail_seq = u64::try_from(tail_seq)
                .map_err(|_| format!("surviving tail row carries a negative seq: {tail_seq}"))?;
            (tail_seq.saturating_add(1), tail_hash)
        }
        None => (0, genesis_hash()),
    };

    let detail = marker_detail(expected, &archive_path, &archive_sha256, severed.len());
    let marker = build_entry(
        marker_seq,
        &chrono::Utc::now().to_rfc3339(),
        "system",
        &AuditAction::ChainReanchored,
        &detail,
        "ok",
        None,
        None,
        marker_prev,
    );

    tx.execute(
        "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            marker.seq as i64,
            &marker.timestamp,
            &marker.agent_id,
            marker.action.to_string(),
            &marker.detail,
            &marker.outcome,
            Option::<String>::None,
            Option::<String>::None,
            &marker.prev_hash,
            &marker.hash,
        ],
    )
    .map_err(|e| format!("cannot record the re-anchor marker: {e}"))?;

    // Inside the transaction, and after both mutations, so the number written to the anchor is the population the next boot will actually load.
    let persisted_rows = count_rows(&tx)?;

    tx.commit()
        .map_err(|e| format!("cannot commit the audit repair: {e}"))?;

    let mut anchor_written = false;
    if let Some(path) = anchor_path {
        AuditLog::write_anchor(path, persisted_rows as u64, &marker.hash).map_err(|e| {
            format!(
                "the database was repaired but the anchor at {} could not be rewritten: {e}",
                path.display()
            )
        })?;
        anchor_written = true;
    }

    Ok(ReanchorReport {
        break_point: expected.clone(),
        severed_rows: severed.len(),
        archive_path,
        archive_sha256,
        marker_seq,
        tip: marker.hash,
        persisted_rows,
        anchor_written,
    })
}

/// Number of rows in `audit_entries`, read through whatever handle is passed (a bare connection for a dry run, the repair transaction otherwise).
fn count_rows(conn: &Connection) -> Result<usize, String> {
    let count = conn
        .query_row("SELECT COUNT(*) FROM audit_entries", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| format!("cannot count audit rows: {e}"))?;
    usize::try_from(count).map_err(|_| format!("audit row count is negative: {count}"))
}

fn severed_entries(conn: &Connection, from_seq: u64) -> Result<Vec<AuditEntry>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {AUDIT_ROW_COLUMNS} FROM audit_entries WHERE seq >= ?1 ORDER BY seq ASC"
        ))
        .map_err(|e| format!("cannot prepare the severed-row read: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![from_seq as i64], decode_audit_row)
        .map_err(|e| format!("cannot read the severed rows: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("cannot decode a severed row: {e}"))?);
    }
    Ok(out)
}

/// Serialise the severed rows to `<archive_dir>/audit-severed-<seq>-<timestamp>.jsonl` and return the path plus the SHA-256 of its bytes.
///
/// The file is written to a `.tmp` sibling and renamed, and both the file and its directory are fsynced before the rename returns, so a crash cannot leave a half-written archive that the marker entry vouches for.
fn write_archive(
    archive_dir: &Path,
    break_seq: u64,
    entries: &[AuditEntry],
) -> Result<(PathBuf, String), String> {
    use std::io::Write;

    std::fs::create_dir_all(archive_dir).map_err(|e| {
        format!(
            "cannot create the archive directory {}: {e}",
            archive_dir.display()
        )
    })?;

    let mut body = Vec::new();
    for entry in entries {
        let line = serde_json::to_vec(entry)
            .map_err(|e| format!("cannot serialise severed row seq {}: {e}", entry.seq))?;
        body.extend_from_slice(&line);
        body.push(b'\n');
    }
    let digest = hex::encode(Sha256::digest(&body));

    // Second-resolution UTC keeps the name sortable and shell-safe; the break seq disambiguates repeated repairs within one second.
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = archive_dir.join(format!("audit-severed-{break_seq}-{stamp}.jsonl"));
    let tmp = path.with_extension("jsonl.tmp");

    let mut file = std::fs::File::create(&tmp)
        .map_err(|e| format!("cannot create the archive {}: {e}", tmp.display()))?;
    file.write_all(&body)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot write the archive {}: {e}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("cannot publish the archive {}: {e}", path.display()))?;
    // Durability of the rename itself; without it a crash can leave the directory entry unrecorded even though the file's contents were synced.
    if let Ok(dir) = std::fs::File::open(archive_dir) {
        let _ = dir.sync_all();
    }

    Ok((path, digest))
}

/// The `ChainReanchored` marker's detail payload.
///
/// JSON with a fixed key order so the string — and therefore the entry hash — is reproducible from the same inputs.
/// It names the archive by file name rather than absolute path: the archive is expected to travel with the database, and an absolute path from the repairing host is noise on any other one.
fn marker_detail(
    break_point: &ChainBreak,
    archive_path: &Path,
    archive_sha256: &str,
    severed_rows: usize,
) -> String {
    let archive = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    serde_json::json!({
        "archive": archive,
        "archive_sha256": archive_sha256,
        "break_expected": break_point.expected,
        "break_found": break_point.found,
        "break_kind": break_point.kind.as_str(),
        "break_seq": break_point.seq,
        "severed_rows": severed_rows,
    })
    .to_string()
}

#[cfg(test)]
mod tests;
