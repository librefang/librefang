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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    let bob = UserId::from_name("Bob");

    let log = AuditLog::with_db(pool.clone());
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
    let log2 = AuditLog::with_db(pool.clone());
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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    // Record entries with DB
    let log = AuditLog::with_db(pool.clone());
    log.record("agent-1", AuditAction::AgentSpawn, "spawn test", "ok");
    log.record("agent-1", AuditAction::ShellExec, "ls", "ok");
    assert_eq!(log.len(), 2);

    // Verify entries in database
    {
        let db_conn = pool.get().unwrap();
        let count: i64 = db_conn
            .query_row("SELECT COUNT(*) FROM audit_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    // Simulate restart: create new AuditLog from same DB
    let log2 = AuditLog::with_db(pool.clone());
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

fn setup_anchored_log() -> (AuditLog, Pool<SqliteConnectionManager>, std::path::PathBuf) {
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }
    let dir = tempfile::tempdir().unwrap();
    let anchor_path = dir.path().join("audit.anchor");
    // Leak the TempDir so the file survives for the duration of the
    // test — we return the PathBuf so the caller keeps owning the
    // cleanup via process exit. Keeping it simple avoids plumbing
    // the TempDir through every test helper.
    std::mem::forget(dir);
    let log = AuditLog::with_db_anchored(pool.clone(), anchor_path.clone());
    (log, pool, anchor_path)
}

#[test]
fn test_anchor_path_accessor_reflects_construction() {
    // The API layer surfaces `anchor_path()` to the dashboard so the
    // UI can show "anchor: ok" vs "anchor: none". Regress that the
    // accessor matches what was passed to `with_db_anchored` and is
    // None for the in-memory constructor.
    let in_memory = AuditLog::new();
    assert!(
        in_memory.anchor_path().is_none(),
        "AuditLog::new() must not advertise an anchor"
    );
    let (log, _db, path) = setup_anchored_log();
    assert_eq!(log.anchor_path(), Some(path.as_path()));
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
        let conn = db.get().unwrap();
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
    let log2 = AuditLog::with_db_anchored(db.clone(), anchor_path.clone());
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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    // First run — no anchor argument, build up some history.
    let log = AuditLog::with_db(pool.clone());
    log.record("agent-1", AuditAction::ToolInvoke, "read_file", "ok");
    log.record("agent-1", AuditAction::ShellExec, "ls", "ok");
    let current_tip = log.tip_hash();
    drop(log);

    // Second run — upgrade path: anchor file does not exist yet.
    let dir = tempfile::tempdir().unwrap();
    let anchor_path = dir.path().join("audit.anchor");
    assert!(!anchor_path.exists());
    let log2 = AuditLog::with_db_anchored(pool.clone(), anchor_path.clone());
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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    let now = chrono::Utc::now();
    let old_ts = now - chrono::Duration::days(30);

    let log = AuditLog::with_db(pool.clone());
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
        let conn = pool.get().unwrap();
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
    let log2 = AuditLog::with_db(pool.clone());
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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    let now = chrono::Utc::now();
    let old_ts = now - chrono::Duration::days(30);

    let log = AuditLog::with_db(pool.clone());
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
        let conn = pool.get().unwrap();
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
    let db_count: i64 = pool
        .get()
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
    let log2 = AuditLog::with_db(pool.clone());
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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    let now = chrono::Utc::now();
    let old_ts = now - chrono::Duration::days(120);

    let log = AuditLog::with_db(pool.clone());
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
        let conn = pool.get().unwrap();
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

    let db_count: i64 = pool
        .get()
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

    let log2 = AuditLog::with_db(pool.clone());
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
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    let log = AuditLog::with_db(pool.clone());
    log.record("a", AuditAction::ToolInvoke, "first", "ok");
    assert_eq!(log.len(), 1);
    let tip_after_first = log.tip_hash();

    // Provoke a transient persistence failure by dropping the
    // table.  The next record() will hit `no such table:
    // audit_entries` from `conn.execute()`.
    pool.get()
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
    let log2 = AuditLog::with_db(pool);
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

// -----------------------------------------------------------------
// since_seq — cursor-based delivery for the SSE log stream.
// -----------------------------------------------------------------

#[test]
fn since_seq_is_strictly_greater_than_cursor() {
    // The cursor's semantic is "the highest seq the consumer has
    // already received" — the SSE poll loop sets it from
    // `entries.last().seq` after delivering. since_seq(N) therefore
    // returns entries with seq > N, NOT seq >= N. since_seq(0) on a
    // log starting at seq=0 deliberately omits seq=0; that initial
    // backfill is handled separately by `recent(...)` in the
    // /api/logs/stream handler so the consumer's first batch isn't
    // missed.
    let log = AuditLog::new();
    for i in 0..5 {
        log.record("a", AuditAction::AgentSpawn, format!("entry-{i}"), "ok");
    }
    // Entries hold seq 0..4. cursor=0 returns 1..4 (4 items).
    let after_zero = log.since_seq(0);
    assert_eq!(
        after_zero.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn since_seq_returns_only_strictly_newer() {
    let log = AuditLog::new();
    for i in 0..5 {
        log.record("a", AuditAction::AgentSpawn, format!("entry-{i}"), "ok");
    }
    // Cursor at seq=2 must skip 0, 1, 2 and return 3, 4.
    let after_2 = log.since_seq(2);
    assert_eq!(
        after_2.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![3, 4]
    );
}

#[test]
fn since_seq_cursor_at_or_past_tail_is_empty() {
    let log = AuditLog::new();
    for i in 0..3 {
        log.record("a", AuditAction::AgentSpawn, format!("entry-{i}"), "ok");
    }
    // Last seq is 2; both equal-and-past cursors must return nothing
    // so the SSE poll loop doesn't re-emit the tail entry.
    assert!(log.since_seq(2).is_empty());
    assert!(log.since_seq(99).is_empty());
}

#[test]
fn since_seq_empty_log_is_empty() {
    let log = AuditLog::new();
    assert!(log.since_seq(0).is_empty());
    assert!(log.since_seq(42).is_empty());
}

#[test]
fn since_seq_does_not_drop_entries_when_window_outpaces_recent() {
    // Regression: the SSE handler used to call `recent(200)` and
    // skip via `entry.seq <= last_seq`, which silently dropped any
    // burst > 200 within one poll interval. `since_seq` must
    // deliver every entry produced after the cursor regardless of
    // burst size.
    let log = AuditLog::new();
    for i in 0..500 {
        log.record("a", AuditAction::AgentSpawn, format!("entry-{i}"), "ok");
    }
    // Simulate "client got up to seq=199 last poll".
    let delivered = log.since_seq(199);
    assert_eq!(delivered.len(), 300);
    assert_eq!(delivered.first().map(|e| e.seq), Some(200));
    assert_eq!(delivered.last().map(|e| e.seq), Some(499));
}

/// Concurrent `record_with_context` against a pooled SQLite must
/// never produce a chain fork.
///
/// Background (PR #4685 review): the pre-pool design serialised
/// the entire append path through `Arc<Mutex<Connection>>`,
/// which by side effect serialised both the in-memory
/// `tip` mutation and the INSERT that depends on the same
/// `prev_hash`. Once the substrate moved to an r2d2 pool, two
/// threads could in principle each grab a `tip` snapshot, build
/// entries with the same `prev_hash`, and both INSERT on
/// different pooled connections — that would produce a Merkle
/// chain fork (two rows sharing the same `prev_hash`). This
/// test fires `THREADS * PER_THREAD` concurrent appends through
/// a real (file-backed) pool with `max_size > 1` and asserts:
///
/// 1. **No writes lost** — total persisted rows equals the
///    number of `record_with_context` calls.
/// 2. **Linear chain** — every persisted entry's `prev_hash` is
///    the `hash` of the entry one row earlier (when ordered by
///    `seq`); exactly one row carries the genesis sentinel.
/// 3. **No `prev_hash` collisions** — no two rows share the same
///    `prev_hash`. This is the direct fork detector: a race that
///    re-uses a stale tip would surface as two rows with the
///    same parent.
/// 4. **`verify_integrity()` passes** after a fresh `with_db`
///    reload, which is the runtime's actual integrity gate.
///
/// The fix that makes this test pass is wrapping the INSERT in
/// `BEGIN IMMEDIATE` so the chain append is serialised at the
/// SQLite layer regardless of how many pool connections, threads,
/// or processes are racing.
#[test]
fn audit_chain_holds_under_concurrent_record() {
    use std::sync::Arc;
    use std::thread;

    const THREADS: usize = 8;
    const PER_THREAD: usize = 50;

    // File-backed DB so `max_size > 1` actually buys us multiple
    // simultaneous connections (`:memory:` is per-connection).
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("audit.db");
    let manager = SqliteConnectionManager::file(&db_path).with_init(|c| {
        c.execute_batch(
            "PRAGMA journal_mode=WAL;\
             PRAGMA busy_timeout=5000;\
             PRAGMA synchronous=NORMAL;",
        )
    });
    let pool = Pool::builder().max_size(8).build(manager).unwrap();
    {
        let conn = pool.get().unwrap();
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
    }

    let log = Arc::new(AuditLog::with_db(pool.clone()));

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let log = log.clone();
            thread::spawn(move || {
                for i in 0..PER_THREAD {
                    log.record_with_context(
                        format!("agent-{t}"),
                        AuditAction::ToolInvoke,
                        format!("op-{t}-{i}"),
                        "ok",
                        None,
                        Some("test".to_string()),
                    );
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let expected = THREADS * PER_THREAD;

    // Reload from the same pool to exercise the persistence path —
    // this is the same code `with_db` runs at daemon boot.
    let reloaded = AuditLog::with_db(pool.clone());

    // (1) No writes lost.
    assert_eq!(
        reloaded.len(),
        expected,
        "expected {expected} persisted rows, got {}",
        reloaded.len()
    );

    // (4) The integrity gate the runtime actually relies on.
    reloaded
        .verify_integrity()
        .expect("Merkle chain must verify after concurrent appends");

    // (2) + (3): inspect the rows directly to assert no parent collisions.
    // Pull every row ordered by seq and walk the chain explicitly.
    let conn = pool.get().unwrap();
    let mut stmt = conn
        .prepare("SELECT seq, prev_hash, hash FROM audit_entries ORDER BY seq ASC")
        .unwrap();
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(rows.len(), expected);

    // (3) prev_hash uniqueness — direct fork detector.
    let mut seen = std::collections::HashSet::with_capacity(rows.len());
    for (seq, prev_hash, _) in &rows {
        assert!(
            seen.insert(prev_hash.clone()),
            "two rows share prev_hash={prev_hash} — chain forked at seq {seq}"
        );
    }

    // (2) Linear chain — exactly one genesis row, every other
    // row's prev_hash equals the previous row's hash.
    let genesis = "0".repeat(64);
    let genesis_count = rows.iter().filter(|(_, p, _)| p == &genesis).count();
    assert_eq!(
        genesis_count, 1,
        "expected exactly one genesis row, got {genesis_count}"
    );
    for window in rows.windows(2) {
        let (_, _, prev_hash) = &window[0];
        let (next_seq, next_prev, _) = &window[1];
        assert_eq!(
            next_prev, prev_hash,
            "chain break at seq {next_seq}: prev_hash does not match prior row's hash"
        );
    }
}
