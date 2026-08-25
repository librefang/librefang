use super::*;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

/// A pool over one in-memory database with the `audit_entries` schema applied.
///
/// `max_size(1)` is load-bearing: `SqliteConnectionManager::memory()` hands every new connection its own empty database, so the pool must hand back the same one for the rows to survive between `pool.get()` calls and the reopen these tests perform.
fn schema_pool() -> Pool<SqliteConnectionManager> {
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    pool.get()
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
    pool
}

fn insert(conn: &Connection, entry: &AuditEntry) {
    conn.execute(
        "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, user_id, channel, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            entry.seq as i64,
            &entry.timestamp,
            &entry.agent_id,
            entry.action.to_string(),
            &entry.detail,
            &entry.outcome,
            Option::<String>::None,
            Option::<String>::None,
            &entry.prev_hash,
            &entry.hash,
        ],
    )
    .unwrap();
}

/// Build one well-formed entry chained onto `prev_hash`.
fn entry_at(seq: u64, prev_hash: &str) -> AuditEntry {
    build_entry(
        seq,
        &format!("2026-08-24T00:00:{seq:02}Z"),
        "agent-1",
        &AuditAction::ToolInvoke,
        &format!("event {seq}"),
        "ok",
        None,
        None,
        prev_hash.to_string(),
    )
}

/// Persist `len` correctly linked entries starting at seq 0 and return them.
fn seed_chain(conn: &Connection, len: u64) -> Vec<AuditEntry> {
    let mut prev = genesis_hash();
    let mut out = Vec::new();
    for seq in 0..len {
        let entry = entry_at(seq, &prev);
        insert(conn, &entry);
        prev = entry.hash.clone();
        out.push(entry);
    }
    out
}

/// The #7702 shape: a second writer's rows land above the first writer's, carrying a `prev_hash` from a snapshot that is no longer the tail.
///
/// Returns the whole table in seq order; the fork begins at `fork_at`.
fn seed_forked_chain(
    conn: &Connection,
    len: u64,
    fork_at: u64,
    stale_prev: &str,
) -> Vec<AuditEntry> {
    let mut entries = seed_chain(conn, fork_at);
    let mut prev = stale_prev.to_string();
    for seq in fork_at..len {
        let entry = entry_at(seq, &prev);
        insert(conn, &entry);
        prev = entry.hash.clone();
        entries.push(entry);
    }
    entries
}

fn row_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM audit_entries", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn diagnose_chain_returns_none_for_an_intact_chain() {
    let pool = schema_pool();
    let conn = pool.get().unwrap();
    seed_chain(&conn, 5);
    assert_eq!(diagnose_chain(&conn).unwrap(), None);
}

#[test]
fn diagnose_chain_reports_the_fork_as_a_prev_hash_mismatch() {
    let pool = schema_pool();
    let conn = pool.get().unwrap();
    // Rows 0..4 are one chain; rows 5..7 are a second writer's, chained onto row 2 because that was its stale snapshot's tail.
    let base = seed_chain(&conn, 5);
    let stale_prev = base[2].hash.clone();
    let mut prev = stale_prev.clone();
    for seq in 5..8 {
        let entry = entry_at(seq, &prev);
        insert(&conn, &entry);
        prev = entry.hash;
    }

    let found = diagnose_chain(&conn).unwrap().expect("a break");
    assert_eq!(found.seq, 5);
    assert_eq!(found.kind, ChainBreakKind::PrevHashMismatch);
    assert_eq!(found.expected, base[4].hash);
    assert_eq!(found.found, stale_prev);
}

#[test]
fn diagnose_chain_distinguishes_an_edited_row_from_a_fork() {
    let pool = schema_pool();
    let conn = pool.get().unwrap();
    let entries = seed_chain(&conn, 4);
    // Rewrite one row's content while leaving every stored hash alone: the linkage still lines up, so only the per-row recomputation can see it.
    conn.execute(
        "UPDATE audit_entries SET detail = 'tampered' WHERE seq = 2",
        [],
    )
    .unwrap();

    let found = diagnose_chain(&conn).unwrap().expect("a break");
    assert_eq!(found.seq, 2);
    assert_eq!(found.kind, ChainBreakKind::HashMismatch);
    assert_eq!(found.found, entries[2].hash);
    assert_ne!(found.expected, entries[2].hash);
}

#[test]
fn diagnose_chain_accepts_a_pruned_prefix() {
    // A retention prune removes a prefix, leaving a suffix whose first row points at a row that is gone. That is the documented steady state, not a break.
    let pool = schema_pool();
    let conn = pool.get().unwrap();
    seed_chain(&conn, 6);
    conn.execute("DELETE FROM audit_entries WHERE seq < 3", [])
        .unwrap();
    assert_eq!(diagnose_chain(&conn).unwrap(), None);
}

#[test]
fn reanchor_leaves_a_log_that_reopens_and_verifies() {
    // The point of the whole exercise: after repair, `with_db_anchored` — the real boot path, anchor file included — must return `Ok` from `verify_integrity`.
    let dir = tempfile::tempdir().unwrap();
    let anchor_path = dir.path().join("audit.anchor");
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();

    let (break_point, base) = {
        let conn = pool.get().unwrap();
        let base = seed_chain(&conn, 5);
        conn.execute("DELETE FROM audit_entries", []).unwrap();
        let entries = seed_forked_chain(&conn, 8, 5, &base[2].hash);
        assert_eq!(entries.len(), 8);
        // Seed the anchor the way a daemon would have left it: naming the tip the forked table actually holds.
        AuditLog::write_anchor(&anchor_path, 8, &entries[7].hash).unwrap();
        let break_point = diagnose_chain(&conn).unwrap().expect("a break");
        (break_point, base)
    };

    // Sanity: the broken table does not verify through the real boot path.
    {
        let broken = AuditLog::with_db_anchored(pool.clone(), anchor_path.clone());
        assert!(broken.verify_integrity().is_err());
    }

    let report = {
        let mut conn = pool.get().unwrap();
        reanchor_after_break(&mut conn, &archive_dir, Some(&anchor_path), &break_point).unwrap()
    };

    assert_eq!(report.severed_rows, 3);
    assert_eq!(report.marker_seq, 5);
    // Five surviving rows plus the marker.
    assert_eq!(report.persisted_rows, 6);
    assert!(report.anchor_written);

    let repaired = AuditLog::with_db_anchored(pool.clone(), anchor_path.clone());
    assert_eq!(
        repaired.verify_integrity(),
        Ok(()),
        "a repaired chain must verify through the same path the daemon boots with"
    );
    assert_eq!(repaired.tip_hash(), report.tip);
    assert_eq!(repaired.persisted_len(), 6);

    // The surviving prefix is untouched, not rewritten.
    let survivors = repaired.recent(6);
    for (index, original) in base.iter().enumerate() {
        assert_eq!(survivors[index].hash, original.hash);
        assert_eq!(survivors[index].detail, original.detail);
    }
    assert!(matches!(survivors[5].action, AuditAction::ChainReanchored));
}

#[test]
fn reanchor_writes_an_anchor_in_the_format_the_reader_accepts() {
    // The anchor line is `"<seq> <hash>\n"`. A reimplementation that writes `"seq:hash"` yields a single token, `read_anchor` rejects it, and the next boot fails verification on top of the break the repair just closed.
    let dir = tempfile::tempdir().unwrap();
    let anchor_path = dir.path().join("audit.anchor");
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();

    let break_point = {
        let conn = pool.get().unwrap();
        let base = seed_chain(&conn, 4);
        conn.execute("DELETE FROM audit_entries", []).unwrap();
        seed_forked_chain(&conn, 6, 4, &base[1].hash);
        diagnose_chain(&conn).unwrap().expect("a break")
    };
    let report = {
        let mut conn = pool.get().unwrap();
        reanchor_after_break(&mut conn, &archive_dir, Some(&anchor_path), &break_point).unwrap()
    };

    let raw = std::fs::read_to_string(&anchor_path).unwrap();
    assert_eq!(raw, format!("{} {}\n", report.persisted_rows, report.tip));
    let parsed = AuditLog::read_anchor(&anchor_path)
        .unwrap()
        .expect("a record");
    assert_eq!(parsed.seq, report.persisted_rows as u64);
    assert_eq!(parsed.hash, report.tip);
    // Nothing left behind by the atomic rename.
    assert!(!anchor_path.with_extension("anchor.tmp").exists());
}

#[test]
fn reanchor_anchors_the_row_count_not_the_marker_seq() {
    // On a database whose prefix has already been pruned, `COUNT(*)` and `marker_seq + 1` diverge. The anchor contract is the population, so anchoring the seq raises a spurious mismatch on the next boot.
    let dir = tempfile::tempdir().unwrap();
    let anchor_path = dir.path().join("audit.anchor");
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();

    let break_point = {
        let conn = pool.get().unwrap();
        let base = seed_chain(&conn, 5);
        conn.execute("DELETE FROM audit_entries", []).unwrap();
        seed_forked_chain(&conn, 8, 5, &base[2].hash);
        // The daily retention prune, which the 90-day default runs unattended.
        conn.execute("DELETE FROM audit_entries WHERE seq < 2", [])
            .unwrap();
        diagnose_chain(&conn).unwrap().expect("a break")
    };

    let report = {
        let mut conn = pool.get().unwrap();
        reanchor_after_break(&mut conn, &archive_dir, Some(&anchor_path), &break_point).unwrap()
    };

    // Rows 2, 3 and 4 survive and the marker takes seq 5: four rows, not six.
    assert_eq!(report.marker_seq, 5);
    assert_eq!(report.persisted_rows, 4);
    let parsed = AuditLog::read_anchor(&anchor_path)
        .unwrap()
        .expect("a record");
    assert_eq!(
        parsed.seq, 4,
        "the anchor must carry the persisted row count, not marker_seq + 1"
    );

    let repaired = AuditLog::with_db_anchored(pool.clone(), anchor_path.clone());
    assert_eq!(repaired.verify_integrity(), Ok(()));
}

#[test]
fn reanchor_archives_every_severed_row_and_commits_the_digest() {
    let dir = tempfile::tempdir().unwrap();
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();

    let (break_point, severed) = {
        let conn = pool.get().unwrap();
        let base = seed_chain(&conn, 5);
        conn.execute("DELETE FROM audit_entries", []).unwrap();
        let all = seed_forked_chain(&conn, 8, 5, &base[2].hash);
        let severed = all[5..].to_vec();
        (diagnose_chain(&conn).unwrap().expect("a break"), severed)
    };
    let report = {
        let mut conn = pool.get().unwrap();
        reanchor_after_break(&mut conn, &archive_dir, None, &break_point).unwrap()
    };

    assert!(!report.anchor_written);
    let body = std::fs::read(&report.archive_path).unwrap();
    assert_eq!(hex::encode(Sha256::digest(&body)), report.archive_sha256);

    let archived: Vec<AuditEntry> = String::from_utf8(body)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(archived.len(), 3);
    for (restored, original) in archived.iter().zip(severed.iter()) {
        assert_eq!(restored.seq, original.seq);
        assert_eq!(restored.hash, original.hash);
        assert_eq!(restored.detail, original.detail);
    }

    // The marker commits the digest, so altering the archive afterwards no longer matches what the chain vouches for.
    let repaired = AuditLog::with_db_anchored(pool.clone(), dir.path().join("unused.anchor"));
    let marker = repaired.recent(1).pop().expect("the marker");
    let detail: serde_json::Value = serde_json::from_str(&marker.detail).unwrap();
    assert_eq!(detail["archive_sha256"], report.archive_sha256);
    assert_eq!(detail["severed_rows"], 3);
    assert_eq!(detail["break_seq"], 5);
    assert_eq!(detail["break_kind"], "PrevHashMismatch");
    assert_eq!(
        detail["archive"],
        report
            .archive_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    );
}

#[test]
fn reanchor_refuses_a_break_that_moved_and_mutates_nothing() {
    // The caller diagnoses outside any lock. Acting on that value without re-reading it under the write lock is the same "derive state, then mutate" pattern that forked the chain in the first place.
    let dir = tempfile::tempdir().unwrap();
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();
    let mut conn = pool.get().unwrap();
    let base = seed_chain(&conn, 5);
    conn.execute("DELETE FROM audit_entries", []).unwrap();
    seed_forked_chain(&conn, 8, 5, &base[2].hash);
    let before = row_count(&conn);

    let stale = ChainBreak {
        seq: 6,
        kind: ChainBreakKind::PrevHashMismatch,
        expected: base[4].hash.clone(),
        found: base[2].hash.clone(),
    };
    let err = reanchor_after_break(&mut conn, &archive_dir, None, &stale).unwrap_err();
    assert!(err.contains("moved since it was diagnosed"), "{err}");
    assert_eq!(row_count(&conn), before);
    assert_eq!(diagnose_chain(&conn).unwrap().expect("a break").seq, 5);
    assert!(!archive_dir.exists(), "no archive on the refusal path");
}

#[test]
fn reanchor_refuses_when_the_chain_already_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();
    let mut conn = pool.get().unwrap();
    let base = seed_chain(&conn, 5);
    let before = row_count(&conn);

    let imaginary = ChainBreak {
        seq: 3,
        kind: ChainBreakKind::PrevHashMismatch,
        expected: base[2].hash.clone(),
        found: base[0].hash.clone(),
    };
    let err = reanchor_after_break(&mut conn, &archive_dir, None, &imaginary).unwrap_err();
    assert!(err.contains("verifies now"), "{err}");
    assert_eq!(row_count(&conn), before);
}

#[test]
fn plan_reanchor_counts_what_the_repair_would_touch() {
    let pool = schema_pool();
    let conn = pool.get().unwrap();
    let base = seed_chain(&conn, 5);
    conn.execute("DELETE FROM audit_entries", []).unwrap();
    seed_forked_chain(&conn, 8, 5, &base[2].hash);

    let plan = plan_reanchor(&conn).unwrap().expect("a plan");
    assert_eq!(plan.break_point.seq, 5);
    assert_eq!(plan.rows_before, 8);
    assert_eq!(plan.severed_rows, 3);
    assert_eq!(plan.surviving_rows, 5);

    conn.execute("DELETE FROM audit_entries WHERE seq >= 5", [])
        .unwrap();
    assert_eq!(plan_reanchor(&conn).unwrap(), None);
}

#[test]
fn a_first_row_hash_mismatch_severs_everything_into_the_archive() {
    // Nothing survives to chain onto, so the marker restarts from the genesis sentinel — but every row is still readable in the archive, which is what separates this from `audit-reset`.
    let dir = tempfile::tempdir().unwrap();
    let anchor_path = dir.path().join("audit.anchor");
    let archive_dir = dir.path().join("audit-archive");
    let pool = schema_pool();

    let break_point = {
        let conn = pool.get().unwrap();
        seed_chain(&conn, 3);
        conn.execute(
            "UPDATE audit_entries SET outcome = 'rewritten' WHERE seq = 0",
            [],
        )
        .unwrap();
        diagnose_chain(&conn).unwrap().expect("a break")
    };
    assert_eq!(break_point.seq, 0);
    assert_eq!(break_point.kind, ChainBreakKind::HashMismatch);

    let report = {
        let mut conn = pool.get().unwrap();
        reanchor_after_break(&mut conn, &archive_dir, Some(&anchor_path), &break_point).unwrap()
    };
    assert_eq!(report.severed_rows, 3);
    assert_eq!(report.marker_seq, 0);
    assert_eq!(report.persisted_rows, 1);

    let archived = std::fs::read_to_string(&report.archive_path).unwrap();
    assert_eq!(archived.lines().count(), 3);

    let repaired = AuditLog::with_db_anchored(pool.clone(), anchor_path.clone());
    assert_eq!(repaired.verify_integrity(), Ok(()));
}
