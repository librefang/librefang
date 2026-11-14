//! SurrealDB migration runner.
//!
//! Keeps applied
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
    let apply_error = |e: surrealdb::Error| MigrationError::Apply {
        version: m.version,
        name: m.name,
        message: e.to_string(),
    };
    // `query` only fails on transport errors; a statement SurrealDB rejects is reported per statement, so `check` it.
    // Without this a rejected DEFINE would be recorded as applied and never retried.
    db.query(m.sql)
        .await
        .map_err(apply_error)?
        .check()
        .map_err(apply_error)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};
    use crate::migrations::OPERATIONAL_MIGRATIONS;
    use crate::pool::{SurrealConnectionPool, SurrealSession};

    /// Every operational migration applies cleanly, a second run is a no-op, and editing an applied migration is refused.
    async fn exercise_runner(session: &SurrealSession) {
        let db = session.client();
        let first = apply_pending(db, OPERATIONAL_MIGRATIONS)
            .await
            .expect("every operational migration applies");
        assert_eq!(first.len(), OPERATIONAL_MIGRATIONS.len());
        let second = apply_pending(db, OPERATIONAL_MIGRATIONS)
            .await
            .expect("second run");
        assert!(second.is_empty(), "re-run applied {second:?}");

        let mut edited = OPERATIONAL_MIGRATIONS.to_vec();
        edited[0].sql = "DEFINE TABLE IF NOT EXISTS audit_entries SCHEMAFULL; -- edited in place";
        match apply_pending(db, &edited).await {
            Err(MigrationError::ChecksumDrift { version, .. }) => assert_eq!(version, 1),
            other => panic!("expected ChecksumDrift, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn operational_migrations_apply_idempotently_and_detect_drift_embedded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(dir.path().join("ops.surreal")),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = SurrealConnectionPool::new()
            .open(&cfg)
            .await
            .expect("open embedded");
        exercise_runner(&session).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn operational_migrations_apply_idempotently_and_detect_drift_remote() {
        let Ok(url) = std::env::var("LIBREFANG_TEST_SURREAL_URL") else {
            eprintln!("SKIP remote surreal: LIBREFANG_TEST_SURREAL_URL unset");
            return;
        };
        let username =
            std::env::var("LIBREFANG_TEST_SURREAL_USER").unwrap_or_else(|_| "root".into());
        let password_env = std::env::var("LIBREFANG_TEST_SURREAL_PASS_ENV")
            .unwrap_or_else(|_| "LIBREFANG_TEST_SURREAL_PASS".into());
        for url in crate::migrations::test_support::remote_urls(&url) {
            let database = format!("runner_{}", uuid::Uuid::new_v4().simple());
            eprintln!("remote surreal: migration runner against {url} db={database}");
            let session = SurrealConnectionPool::new()
                .open_remote(&RemoteSurrealConfig {
                    url: url.clone(),
                    namespace: "librefang_test".into(),
                    database: database.clone(),
                    username: username.clone(),
                    password_env: password_env.clone(),
                    tls_skip_verify: false,
                })
                .await
                .unwrap_or_else(|e| panic!("open remote {url}: {e}"));
            exercise_runner(&session).await;
            session
                .client()
                .query(format!("REMOVE DATABASE IF EXISTS {database}"))
                .await
                .expect("drop test database");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rejected_statement_fails_the_migration_instead_of_being_recorded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(dir.path().join("bad.surreal")),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = SurrealConnectionPool::new()
            .open(&cfg)
            .await
            .expect("open embedded");
        let bad = [Migration {
            version: 1,
            name: "rejected",
            // Parses, but the second DEFINE fails at execution ("already exists"), which `query` alone does not surface.
            sql: "DEFINE TABLE IF NOT EXISTS t SCHEMAFULL; DEFINE FIELD n ON t TYPE int; DEFINE FIELD n ON t TYPE int;",
        }];
        match apply_pending(session.client(), &bad).await {
            Err(MigrationError::Apply { version: 1, .. }) => {}
            other => panic!("expected Apply error, got {other:?}"),
        }
        let recorded: Vec<serde_json::Value> = session
            .client()
            .query(format!("SELECT version FROM {APPLIED_TABLE}"))
            .await
            .expect("query ledger")
            .take(0)
            .expect("ledger rows");
        assert!(
            recorded.is_empty(),
            "a failed migration was recorded: {recorded:?}"
        );
    }
}
