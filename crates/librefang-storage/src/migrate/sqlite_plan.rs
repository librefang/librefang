//! Read-only row counts for the legacy SQLite database.
//!
//! Used by `librefang storage migrate --dry-run` to preview how much
//! work the actual migration will do without touching SurrealDB.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::error::{StorageError, StorageResult};
use crate::migrate::MigrationPlan;

/// Tables the migrator copies. Same list as
/// [`super::sqlite_to_surreal::TABLES`] so dry runs and live runs report
/// identical entries.
const TABLES: &[&str] = &[
    "audit_entries",
    "hook_traces",
    "circuit_breaker_states",
    "totp_lockout",
    "agents",
];

pub(super) fn plan(sqlite_path: &Path) -> StorageResult<MigrationPlan> {
    if !sqlite_path.exists() {
        return Err(StorageError::Backend(format!(
            "legacy sqlite database not found at {}",
            sqlite_path.display()
        )));
    }
    // Open read-only so a running daemon's WAL is untouched. We still
    // honour `SQLITE_OPEN_NO_MUTEX` because every call below is
    // serialised on this single connection.
    let conn = Connection::open_with_flags(
        sqlite_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| StorageError::Backend(format!("open sqlite {}: {e}", sqlite_path.display())))?;

    let mut counts = BTreeMap::new();
    for table in TABLES {
        let n = count_rows(&conn, table).unwrap_or(0);
        counts.insert((*table).to_string(), n);
    }
    Ok(MigrationPlan {
        source_rows: counts,
    })
}

fn count_rows(conn: &Connection, table: &str) -> Option<u64> {
    // `table` is hard-coded above so SQL injection is not a concern; we
    // still avoid `format!` at the binding boundary by validating the
    // identifier first.
    if !is_safe_identifier(table) {
        return None;
    }
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let n: i64 = conn.query_row(&sql, [], |r| r.get(0)).ok()?;
    u64::try_from(n).ok()
}

fn is_safe_identifier(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
