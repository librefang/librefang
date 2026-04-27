//! One-shot migration tooling for moving librefang data between backends.
//!
//! Phase 7 of the `surrealdb-storage-swap` plan introduces this module so
//! operators with an existing `librefang.db` (legacy SQLite stack) can
//! upgrade to the SurrealDB stack without losing audit history, hook
//! traces, circuit-breaker state, TOTP lockout rows, or the agent
//! registry.
//!
//! ## Scope
//!
//! Only the operational tables that are persisted by the matching
//! SurrealDB backends in Phase 5 + 6 are migrated:
//!
//! | SQLite table              | SurrealDB table             |
//! |---------------------------|-----------------------------|
//! | `audit_entries`           | `audit_entries`             |
//! | `hook_traces`             | `hook_traces`               |
//! | `circuit_breaker_states`  | `circuit_breaker_states`    |
//! | `totp_lockout`            | `totp_lockout`              |
//! | `agents`                  | `agents` (Surreal schemaless registry) |
//!
//! Rich semantic memory (the `memories` / `entities` / `relations`
//! tables in `librefang-memory`) lives in surreal-memory's own schema
//! and is intentionally out of scope here — operators who want to carry
//! semantic data forward should use surreal-memory's own importers.
//!
//! ## Idempotency
//!
//! The migrator is safe to re-run: every write goes through SurrealDB's
//! `upsert(record_id)`, so rerunning a migration over a partially
//! complete copy converges instead of duplicating rows.
//!
//! ## Receipts
//!
//! On a successful (non-dry-run) migration the runner writes a receipt
//! JSON file to the configured `receipt_dir`. The file name is
//! `migration-<kind>-<timestamp>.json`. Receipts are append-only and the
//! UI can list them under "Storage › History".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{StorageError, StorageResult};

#[cfg(all(feature = "sqlite-backend", feature = "surreal-backend"))]
mod sqlite_to_surreal;

#[cfg(feature = "sqlite-backend")]
mod sqlite_plan;

/// Migration source/target pair.
///
/// Currently only `SqliteToSurreal` is implemented; future variants can
/// be added as new backends are introduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationKind {
    /// Copy the legacy SQLite tables into the configured SurrealDB
    /// instance.
    SqliteToSurreal,
}

impl MigrationKind {
    /// Short, slug-style name used in receipt file names.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::SqliteToSurreal => "sqlite-to-surreal",
        }
    }
}

/// Options controlling how a migration runs.
#[derive(Debug, Clone, Default)]
pub struct MigrationOptions {
    /// When `true` the migrator only counts source rows and writes
    /// nothing to the target. No receipt file is written either —
    /// dry runs are reported back through [`MigrationReceipt`] in the
    /// returned struct only.
    pub dry_run: bool,
    /// Directory under which receipt JSON files are written. When
    /// `None` the receipt is returned in memory but never persisted.
    pub receipt_dir: Option<PathBuf>,
}

/// Source row counts collected during the planning pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationPlan {
    /// `table -> row count` discovered in the source database.
    pub source_rows: BTreeMap<String, u64>,
}

impl MigrationPlan {
    /// Total number of rows the migrator would copy if executed.
    #[must_use]
    pub fn total_rows(&self) -> u64 {
        self.source_rows.values().copied().sum()
    }
}

/// A persisted record describing one migration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReceipt {
    /// Schema kind (`sqlite-to-surreal`, …).
    pub kind: MigrationKind,
    /// Wall-clock time the migrator started.
    pub started_at: DateTime<Utc>,
    /// Wall-clock time the migrator finished.
    pub finished_at: DateTime<Utc>,
    /// Source description (e.g. SQLite path).
    pub source: String,
    /// Target description (e.g. `embedded:/path` or `wss://host/ns/db`).
    pub target: String,
    /// Whether this run was a dry run.
    pub dry_run: bool,
    /// `table -> row count` actually copied (or that would be copied
    /// for dry runs).
    pub copied: BTreeMap<String, u64>,
    /// Per-table errors (if any) encountered while copying. Present
    /// even on partial successes so operators see what was skipped.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<String, String>,
}

impl MigrationReceipt {
    /// `true` if the migrator copied every row in the plan without
    /// per-table errors.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Build the plan (row counts only) for migrating from a legacy SQLite
/// database.
///
/// Available whenever the `sqlite-backend` feature is enabled. No
/// SurrealDB connection is required for planning, which is what the
/// `--dry-run` flag in `librefang storage migrate` uses.
///
/// # Errors
///
/// Returns [`StorageError::BackendDisabled`] if the `sqlite-backend`
/// feature is not enabled, or [`StorageError::Backend`] if the source
/// database cannot be opened or queried.
pub fn plan_sqlite(sqlite_path: &Path) -> StorageResult<MigrationPlan> {
    #[cfg(feature = "sqlite-backend")]
    {
        sqlite_plan::plan(sqlite_path)
    }
    #[cfg(not(feature = "sqlite-backend"))]
    {
        let _ = sqlite_path;
        Err(StorageError::BackendDisabled { backend: "sqlite" })
    }
}

/// Run the SQLite → SurrealDB migration end-to-end.
///
/// On success returns a [`MigrationReceipt`]; if `opts.receipt_dir` is
/// `Some`, the same receipt is also written to disk before returning.
///
/// # Errors
///
/// - [`StorageError::BackendDisabled`] if either backend feature is
///   compiled out.
/// - [`StorageError::Backend`] for transport-level failures.
pub fn migrate_sqlite_to_surreal(
    sqlite_path: &Path,
    #[allow(unused_variables)] session: &crate::pool::SurrealSession,
    opts: &MigrationOptions,
) -> StorageResult<MigrationReceipt> {
    #[cfg(all(feature = "sqlite-backend", feature = "surreal-backend"))]
    {
        sqlite_to_surreal::run(sqlite_path, session, opts)
    }
    #[cfg(not(all(feature = "sqlite-backend", feature = "surreal-backend")))]
    {
        let _ = (sqlite_path, opts);
        Err(StorageError::BackendDisabled {
            backend: if cfg!(feature = "sqlite-backend") {
                "surreal"
            } else {
                "sqlite"
            },
        })
    }
}

/// Persist a receipt to the given directory.
///
/// The file name is derived from the receipt kind and `started_at`
/// timestamp so concurrent runs do not clobber each other.
///
/// # Errors
///
/// Returns [`StorageError::Backend`] if the file cannot be written.
pub fn write_receipt(dir: &Path, receipt: &MigrationReceipt) -> StorageResult<PathBuf> {
    std::fs::create_dir_all(dir)
        .map_err(|e| StorageError::Backend(format!("create receipt dir {}: {e}", dir.display())))?;
    // Sub-second precision so two back-to-back runs (which is the
    // common idempotency check) never clobber each other.
    let stamp = receipt.started_at.format("%Y%m%dT%H%M%S%.6fZ");
    let name = format!("migration-{}-{stamp}.json", receipt.kind.slug());
    let path = dir.join(name);
    let body = serde_json::to_string_pretty(receipt)
        .map_err(|e| StorageError::Backend(format!("serialise receipt: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| StorageError::Backend(format!("write receipt {}: {e}", path.display())))?;
    Ok(path)
}
