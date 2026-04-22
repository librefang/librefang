//! SurrealDB migration runner.
//!
//! Inspired by `surreal-memory-server`'s migration runner: keep applied
//! versions in a `_schema_version` table so re-runs are idempotent and
//! drift is detectable. This crate intentionally hosts only the runner —
//! migrations themselves live alongside the modules that own the data.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::{engine::any::Any, Surreal};
use thiserror::Error;
use tracing::{debug, info, warn};

/// Name of the table the runner uses to track applied migrations.
pub const APPLIED_TABLE: &str = "_schema_version";

/// A single migration.
///
/// Combined with [`apply_pending`] this gives idempotent, append-only
/// schema evolution. Once a migration is in the released history, its
/// `sql` is frozen — the runner will refuse to apply a migration whose
/// recorded checksum does not match.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Strictly increasing version number, starting at 1.
    pub version: u32,
    /// Short human label used in logs and the `_schema_version` row.
    pub name: &'static str,
    /// Idempotent SurrealQL DDL. MUST use `IF NOT EXISTS` everywhere.
    pub sql: &'static str,
}

/// Errors returned by the migration runner.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// The runner could not query or write to `_schema_version`.
    #[error("schema bootstrap failed: {0}")]
    Bootstrap(String),
    /// A migration script returned an error.
    #[error("migration v{version} ({name}) failed: {message}")]
    Apply {
        /// Migration version that failed.
        version: u32,
        /// Migration name that failed.
        name: &'static str,
        /// SurrealDB-side error message.
        message: String,
    },
    /// A previously-applied migration's checksum no longer matches the
    /// current source. This indicates the migration history was edited in
    /// place, which the runner refuses to silently accept.
    #[error(
        "migration v{version} ({name}) has drifted: checksum {found} on disk, \
         {expected} recorded; migrations are append-only — add a new version \
         instead of editing v{version}"
    )]
    ChecksumDrift {
        /// Migration version with the mismatch.
        version: u32,
        /// Migration name with the mismatch.
        name: &'static str,
        /// Checksum recorded in `_schema_version`.
        expected: String,
        /// Checksum computed from the current source.
        found: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct AppliedRow {
    version: u32,
    name: String,
    checksum: String,
    applied_at: String,
}

/// Apply every migration in `migrations` whose `version` has not yet been
/// recorded in [`APPLIED_TABLE`].
///
/// Migrations are applied in `version` order. The function is safe to call
/// on every daemon boot — already-applied migrations are skipped after
/// their checksum is verified.
///
/// # Errors
///
/// - [`MigrationError::Bootstrap`] if the runner cannot create or query
///   `_schema_version`.
/// - [`MigrationError::Apply`] if a SurrealQL script fails.
/// - [`MigrationError::ChecksumDrift`] if a previously-applied migration's
///   source has been edited in place.
pub async fn apply_pending(
    db: &Surreal<Any>,
    migrations: &[Migration],
) -> Result<Vec<u32>, MigrationError> {
    bootstrap_schema(db).await?;
    let applied = load_applied(db).await?;

    // Detect drift on every already-recorded migration, even ones we won't
    // re-apply this run, so a tampered migration is caught at startup.
    for m in migrations {
        if let Some(row) = applied.iter().find(|r| r.version == m.version) {
            let found = checksum(m.sql);
            if found != row.checksum {
                return Err(MigrationError::ChecksumDrift {
                    version: m.version,
                    name: m.name,
                    expected: row.checksum.clone(),
                    found,
                });
            }
        }
    }

    let mut applied_versions = Vec::new();
    let mut sorted = migrations.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|m| m.version);

    for m in sorted {
        if applied.iter().any(|r| r.version == m.version) {
            debug!(
                version = m.version,
                name = m.name,
                "migration already applied"
            );
            continue;
        }
        run_migration(db, m).await?;
        applied_versions.push(m.version);
    }

    if applied_versions.is_empty() {
        debug!("no pending migrations");
    } else {
        info!(versions = ?applied_versions, "applied migrations");
    }
    Ok(applied_versions)
}

async fn bootstrap_schema(db: &Surreal<Any>) -> Result<(), MigrationError> {
    let ddl = format!(
        "DEFINE TABLE IF NOT EXISTS {APPLIED_TABLE} SCHEMALESS;
         DEFINE INDEX IF NOT EXISTS {APPLIED_TABLE}_version_idx ON {APPLIED_TABLE} \
            COLUMNS version UNIQUE;"
    );
    db.query(ddl)
        .await
        .map_err(|e| MigrationError::Bootstrap(e.to_string()))?;
    Ok(())
}

async fn load_applied(db: &Surreal<Any>) -> Result<Vec<AppliedRow>, MigrationError> {
    // Use a query rather than `select(table)` so we get back JSON we can
    // deserialise without imposing `SurrealValue` on `AppliedRow`.
    let q = format!("SELECT version, name, checksum, applied_at FROM {APPLIED_TABLE}");
    let rows: Vec<serde_json::Value> = db
        .query(q)
        .await
        .map_err(|e| MigrationError::Bootstrap(e.to_string()))?
        .take(0)
        .map_err(|e| MigrationError::Bootstrap(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        match serde_json::from_value::<AppliedRow>(row) {
            Ok(r) => out.push(r),
            Err(e) => warn!(error = %e, "skipping malformed _schema_version row"),
        }
    }
    Ok(out)
}

async fn run_migration(db: &Surreal<Any>, m: &Migration) -> Result<(), MigrationError> {
    db.query(m.sql).await.map_err(|e| MigrationError::Apply {
        version: m.version,
        name: m.name,
        message: e.to_string(),
    })?;
    record_applied(db, m).await
}

async fn record_applied(db: &Surreal<Any>, m: &Migration) -> Result<(), MigrationError> {
    let row = serde_json::json!({
        "version": m.version,
        "name": m.name,
        "checksum": checksum(m.sql),
        // Stored as ISO-8601 string to dodge SurrealDB 3.0's auto-coercion of
        // datetime fields on schemaless tables (same pitfall the agent
        // registry hit in Phase 5).
        "applied_at": chrono::Utc::now().to_rfc3339(),
    });
    let id = format!("v{}", m.version);
    let _: Option<serde_json::Value> = db
        .upsert((APPLIED_TABLE, id.as_str()))
        .content(row)
        .await
        .map_err(|e| MigrationError::Apply {
        version: m.version,
        name: m.name,
        message: e.to_string(),
    })?;
    Ok(())
}

fn checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    hex::encode(hasher.finalize())
}
