//! Prompt versioning and A/B experiment storage.
//!
//! Provides SQLite-backed storage for prompt versions and experiments.

use chrono::{DateTime, Utc};
use librefang_types::agent::{
    AgentId, ExperimentStatus, ExperimentVariant, ExperimentVariantMetrics, PromptExperiment,
    PromptVersion,
};
use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, Row};
use std::str::FromStr;
use uuid::Uuid;

fn row_to_prompt_version(row: &Row) -> rusqlite::Result<PromptVersion> {
    let id: String = row.get(0)?;
    let agent_id: String = row.get(1)?;
    let tools: String = row.get(5)?;
    let variables: String = row.get(6)?;
    let created_at: String = row.get(7)?;
    let is_active: i32 = row.get(9)?;

    Ok(PromptVersion {
        id: Uuid::parse_str(&id).unwrap_or_default(),
        agent_id: AgentId::from_str(&agent_id).unwrap_or_default(),
        version: row.get::<_, i64>(2)? as u32,
        content_hash: row.get(3)?,
        system_prompt: row.get(4)?,
        tools: serde_json::from_str(&tools).unwrap_or_default(),
        variables: serde_json::from_str(&variables).unwrap_or_default(),
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        created_by: row.get(8)?,
        is_active: is_active != 0,
        description: row.get(10)?,
    })
}

fn row_to_prompt_experiment(row: &Row) -> rusqlite::Result<PromptExperiment> {
    let id: String = row.get(0)?;
    let agent_id: String = row.get(2)?;
    let status: String = row.get(3)?;
    let traffic_split: String = row.get(4)?;
    let success_criteria: String = row.get(5)?;
    let started_at: Option<String> = row.get(6)?;
    let ended_at: Option<String> = row.get(7)?;
    let created_at: String = row.get(8)?;

    Ok(PromptExperiment {
        id: Uuid::parse_str(&id).unwrap_or_default(),
        name: row.get(1)?,
        agent_id: AgentId::from_str(&agent_id).unwrap_or_default(),
        status: serde_json::from_str(&status).unwrap_or(ExperimentStatus::Draft),
        traffic_split: serde_json::from_str(&traffic_split).unwrap_or_default(),
        success_criteria: serde_json::from_str(&success_criteria).unwrap_or_default(),
        started_at: started_at.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .ok()
        }),
        ended_at: ended_at.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .ok()
        }),
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        variants: vec![],
    })
}

/// Load variants from DB for a given experiment. Must be called while holding the conn lock.
fn load_variants_for_experiment(
    conn: &Connection,
    experiment_id: &str,
) -> rusqlite::Result<Vec<ExperimentVariant>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, prompt_version_id, description FROM experiment_variants WHERE experiment_id = ?1",
    )?;
    let rows = stmt.query_map([experiment_id], |row| {
        let id: String = row.get(0)?;
        let prompt_version_id: String = row.get(2)?;
        Ok(ExperimentVariant {
            id: Uuid::parse_str(&id).unwrap_or_default(),
            name: row.get(1)?,
            prompt_version_id: Uuid::parse_str(&prompt_version_id).unwrap_or_default(),
            description: row.get(3)?,
        })
    })?;
    rows.collect()
}

#[derive(Clone)]
pub struct PromptStore {
    pool: Pool<SqliteConnectionManager>,
}

impl PromptStore {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Create a new PromptStore with its own dedicated connection pool.
    /// This avoids sharing a connection with UsageStore, preventing potential
    /// conflicts during concurrent writes.
    ///
    /// `pool_size` mirrors `config.toml: [memory] pool_size`; values of 0 are
    /// clamped up to 1 (r2d2 panics on `max_size = 0`).
    pub fn new_with_path<P: AsRef<std::path::Path>>(
        db_path: P,
        pool_size: u32,
    ) -> LibreFangResult<Self> {
        let manager = r2d2_sqlite::SqliteConnectionManager::file(db_path.as_ref()).with_init(|c| {
            c.execute_batch(
                "PRAGMA journal_mode=WAL; \
                     PRAGMA busy_timeout=5000; \
                     PRAGMA cache_size=-2000; \
                     PRAGMA mmap_size=0;",
            )
        });
        let pool = r2d2::Pool::builder()
            .max_size(pool_size.max(1))
            .build(manager)
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        Ok(Self { pool })
    }

    pub fn create_version(&self, version: PromptVersion) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "INSERT INTO prompt_versions (id, agent_id, version, content_hash, system_prompt, tools, variables, created_at, created_by, is_active, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                version.id.to_string(),
                version.agent_id.to_string(),
                version.version as i64,
                version.content_hash,
                version.system_prompt,
                serde_json::to_string(&version.tools).unwrap_or_default(),
                serde_json::to_string(&version.variables).unwrap_or_default(),
                version.created_at.to_rfc3339(),
                version.created_by,
                version.is_active as i32,
                version.description,
            ],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        Ok(())
    }

    pub fn list_versions(&self, agent_id: AgentId) -> LibreFangResult<Vec<PromptVersion>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT id, agent_id, version, content_hash, system_prompt, tools, variables, created_at, created_by, is_active, description
                      FROM prompt_versions WHERE agent_id = ?1 ORDER BY version DESC")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map([agent_id.to_string()], row_to_prompt_version)
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut versions = Vec::new();
        for row in rows.flatten() {
            versions.push(row);
        }
        Ok(versions)
    }

    pub fn get_version(&self, id: Uuid) -> LibreFangResult<Option<PromptVersion>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT id, agent_id, version, content_hash, system_prompt, tools, variables, created_at, created_by, is_active, description
                      FROM prompt_versions WHERE id = ?1")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let result = stmt
            .query_row([id.to_string()], row_to_prompt_version)
            .optional()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(result)
    }

    pub fn get_active_version(&self, agent_id: AgentId) -> LibreFangResult<Option<PromptVersion>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT id, agent_id, version, content_hash, system_prompt, tools, variables, created_at, created_by, is_active, description
                      FROM prompt_versions WHERE agent_id = ?1 AND is_active = 1 LIMIT 1")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let result = stmt
            .query_row([agent_id.to_string()], row_to_prompt_version)
            .optional()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(result)
    }

    pub fn set_active_version(&self, id: Uuid, agent_id: AgentId) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        tx.execute(
            "UPDATE prompt_versions SET is_active = 0 WHERE agent_id = ?1",
            [agent_id.to_string()],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        tx.execute(
            "UPDATE prompt_versions SET is_active = 1 WHERE id = ?1",
            [id.to_string()],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        tx.commit()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(())
    }

    pub fn delete_version(&self, id: Uuid) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "DELETE FROM prompt_versions WHERE id = ?1",
            [id.to_string()],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Delete oldest inactive versions if the agent exceeds the max count.
    pub fn prune_old_versions(&self, agent_id: AgentId, max_versions: u32) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        // Get total count for this agent
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM prompt_versions WHERE agent_id = ?1",
                [agent_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        if count > max_versions {
            let to_delete = count - max_versions;
            conn.execute(
                "DELETE FROM prompt_versions WHERE id IN (
                    SELECT id FROM prompt_versions
                    WHERE agent_id = ?1 AND is_active = 0
                    ORDER BY version ASC
                    LIMIT ?2
                )",
                rusqlite::params![agent_id.to_string(), to_delete],
            )
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    /// Create a new version only if the system prompt hash differs from the current active version.
    /// Returns true if a new version was created.
    pub fn create_version_if_changed(
        &self,
        agent_id: AgentId,
        system_prompt: &str,
        created_by: &str,
    ) -> LibreFangResult<bool> {
        let content_hash = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(system_prompt.as_bytes());
            format!("{:x}", hasher.finalize())
        };

        // Check if active version has the same hash
        if let Some(active) = self.get_active_version(agent_id)? {
            if active.content_hash == content_hash {
                return Ok(false);
            }
        }

        let next_version = self.get_latest_version_number(agent_id)? + 1;
        let version = PromptVersion {
            id: uuid::Uuid::new_v4(),
            agent_id,
            version: next_version,
            content_hash,
            system_prompt: system_prompt.to_string(),
            tools: vec![],
            variables: vec![],
            created_at: chrono::Utc::now(),
            created_by: created_by.to_string(),
            is_active: true,
            description: Some(format!("Auto-tracked v{next_version}")),
        };

        // Create first, then activate (set_active deactivates all others then activates this one)
        self.create_version(version.clone())?;
        self.set_active_version(version.id, agent_id)?;

        Ok(true)
    }

    pub fn get_latest_version_number(&self, agent_id: AgentId) -> LibreFangResult<u32> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT MAX(version) FROM prompt_versions WHERE agent_id = ?1")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let result: Option<u32> = stmt
            .query_row([agent_id.to_string()], |row| row.get(0))
            .optional()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(result.unwrap_or(0))
    }

    pub fn create_experiment(&self, experiment: PromptExperiment) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        conn.execute(
            "INSERT INTO prompt_experiments (id, name, agent_id, status, traffic_split, success_criteria, started_at, ended_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                experiment.id.to_string(),
                experiment.name,
                experiment.agent_id.to_string(),
                serde_json::to_string(&experiment.status).unwrap_or_default(),
                serde_json::to_string(&experiment.traffic_split).unwrap_or_default(),
                serde_json::to_string(&experiment.success_criteria).unwrap_or_default(),
                experiment.started_at.map(|dt| dt.to_rfc3339()),
                experiment.ended_at.map(|dt| dt.to_rfc3339()),
                experiment.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        for variant in &experiment.variants {
            conn.execute(
                "INSERT INTO experiment_variants (id, experiment_id, name, prompt_version_id, description)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    variant.id.to_string(),
                    experiment.id.to_string(),
                    variant.name,
                    variant.prompt_version_id.to_string(),
                    variant.description,
                ],
            )
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

            conn.execute(
                "INSERT INTO experiment_metrics (id, experiment_id, variant_id, total_requests, successful_requests, failed_requests, total_latency_ms, total_cost_usd, last_updated)
                 VALUES (?1, ?2, ?3, 0, 0, 0, 0, 0, ?4)",
                rusqlite::params![
                    Uuid::new_v4().to_string(),
                    experiment.id.to_string(),
                    variant.id.to_string(),
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        }

        Ok(())
    }

    pub fn list_experiments(&self, agent_id: AgentId) -> LibreFangResult<Vec<PromptExperiment>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT id, name, agent_id, status, traffic_split, success_criteria, started_at, ended_at, created_at
                      FROM prompt_experiments WHERE agent_id = ?1 ORDER BY created_at DESC")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map([agent_id.to_string()], row_to_prompt_experiment)
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut experiments = Vec::new();
        for mut exp in rows.flatten() {
            exp.variants =
                load_variants_for_experiment(&conn, &exp.id.to_string()).unwrap_or_default();
            experiments.push(exp);
        }
        Ok(experiments)
    }

    pub fn get_experiment(&self, id: Uuid) -> LibreFangResult<Option<PromptExperiment>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare("SELECT id, name, agent_id, status, traffic_split, success_criteria, started_at, ended_at, created_at
                      FROM prompt_experiments WHERE id = ?1")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let result = stmt
            .query_row([id.to_string()], row_to_prompt_experiment)
            .optional()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(result.map(|mut exp| {
            exp.variants =
                load_variants_for_experiment(&conn, &exp.id.to_string()).unwrap_or_default();
            exp
        }))
    }

    pub fn update_experiment_status(
        &self,
        id: Uuid,
        status: ExperimentStatus,
    ) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let now = Utc::now().to_rfc3339();
        let (started_at, ended_at) = match status {
            ExperimentStatus::Running => {
                let mut stmt = conn
                    .prepare("SELECT started_at FROM prompt_experiments WHERE id = ?1")
                    .map_err(|e| LibreFangError::Internal(e.to_string()))?;
                let has_started: Option<String> = stmt
                    .query_row([id.to_string()], |row| row.get(0))
                    .optional()
                    .map_err(|e| LibreFangError::Internal(e.to_string()))?;
                (has_started.or(Some(now.clone())), None)
            }
            ExperimentStatus::Completed => (None, Some(now.clone())),
            _ => (None, None),
        };

        let status_str = serde_json::to_string(&status).unwrap_or_default();
        conn.execute(
            "UPDATE prompt_experiments SET status = ?1, started_at = COALESCE(?2, started_at), ended_at = COALESCE(?3, ended_at) WHERE id = ?4",
            rusqlite::params![status_str, started_at, ended_at, id.to_string()],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(())
    }

    pub fn get_running_experiment(
        &self,
        agent_id: AgentId,
    ) -> LibreFangResult<Option<PromptExperiment>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let status_running = serde_json::to_string(&ExperimentStatus::Running).unwrap_or_default();

        let mut stmt = conn
            .prepare("SELECT id, name, agent_id, status, traffic_split, success_criteria, started_at, ended_at, created_at
                      FROM prompt_experiments WHERE agent_id = ?1 AND status = ?2 LIMIT 1")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let result = stmt
            .query_row(
                rusqlite::params![agent_id.to_string(), status_running],
                row_to_prompt_experiment,
            )
            .optional()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(result.map(|mut exp| {
            exp.variants =
                load_variants_for_experiment(&conn, &exp.id.to_string()).unwrap_or_default();
            exp
        }))
    }

    pub fn record_request(
        &self,
        experiment_id: Uuid,
        variant_id: Uuid,
        latency_ms: u64,
        cost_usd: f64,
        success: bool,
    ) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        conn.execute(
            "UPDATE experiment_metrics SET 
             total_requests = total_requests + 1,
             successful_requests = successful_requests + ?1,
             failed_requests = failed_requests + ?2,
             total_latency_ms = total_latency_ms + ?3,
             total_cost_usd = total_cost_usd + ?4,
             last_updated = ?5
             WHERE experiment_id = ?6 AND variant_id = ?7",
            rusqlite::params![
                if success { 1 } else { 0 },
                if success { 0 } else { 1 },
                latency_ms as i64,
                cost_usd,
                Utc::now().to_rfc3339(),
                experiment_id.to_string(),
                variant_id.to_string(),
            ],
        )
        .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(())
    }

    pub fn get_variant_metrics(
        &self,
        variant_id: Uuid,
    ) -> LibreFangResult<Option<ExperimentVariantMetrics>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let mut stmt = conn
            .prepare("SELECT em.variant_id, ev.name, em.total_requests, em.successful_requests, em.failed_requests, em.total_latency_ms, em.total_cost_usd
                      FROM experiment_metrics em
                      JOIN experiment_variants ev ON ev.id = em.variant_id
                      WHERE em.variant_id = ?1")
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let result = stmt
            .query_row([variant_id.to_string()], |row| {
                let total_requests: i64 = row.get(2)?;
                let successful_requests: i64 = row.get(3)?;
                let failed_requests: i64 = row.get(4)?;
                let total_latency_ms: i64 = row.get(5)?;
                let total_cost_usd: f64 = row.get(6)?;

                let success_rate = if total_requests > 0 {
                    (successful_requests as f64 / total_requests as f64) * 100.0
                } else {
                    0.0
                };

                let avg_latency_ms = if total_requests > 0 {
                    total_latency_ms as f64 / total_requests as f64
                } else {
                    0.0
                };

                let avg_cost_usd = if total_requests > 0 {
                    total_cost_usd / total_requests as f64
                } else {
                    0.0
                };

                Ok(ExperimentVariantMetrics {
                    variant_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    variant_name: row.get(1)?,
                    total_requests: total_requests as u64,
                    successful_requests: successful_requests as u64,
                    failed_requests: failed_requests as u64,
                    success_rate,
                    avg_latency_ms,
                    avg_cost_usd,
                    total_cost_usd,
                })
            })
            .optional()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        Ok(result)
    }

    pub fn get_experiment_metrics(
        &self,
        experiment_id: Uuid,
    ) -> LibreFangResult<Vec<ExperimentVariantMetrics>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let mut stmt = conn
            .prepare(
                "SELECT em.variant_id, ev.name, em.total_requests, em.successful_requests, em.failed_requests, em.total_latency_ms, em.total_cost_usd
                 FROM experiment_metrics em
                 JOIN experiment_variants ev ON ev.id = em.variant_id
                 WHERE em.experiment_id = ?1",
            )
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map([experiment_id.to_string()], |row| {
                let total_requests: i64 = row.get(2)?;
                let successful_requests: i64 = row.get(3)?;
                let failed_requests: i64 = row.get(4)?;
                let total_latency_ms: i64 = row.get(5)?;
                let total_cost_usd: f64 = row.get(6)?;

                let success_rate = if total_requests > 0 {
                    (successful_requests as f64 / total_requests as f64) * 100.0
                } else {
                    0.0
                };
                let avg_latency_ms = if total_requests > 0 {
                    total_latency_ms as f64 / total_requests as f64
                } else {
                    0.0
                };
                let avg_cost_usd = if total_requests > 0 {
                    total_cost_usd / total_requests as f64
                } else {
                    0.0
                };

                Ok(ExperimentVariantMetrics {
                    variant_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    variant_name: row.get(1)?,
                    total_requests: total_requests as u64,
                    successful_requests: successful_requests as u64,
                    failed_requests: failed_requests as u64,
                    success_rate,
                    avg_latency_ms,
                    avg_cost_usd,
                    total_cost_usd,
                })
            })
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut metrics = Vec::new();
        for row in rows {
            metrics.push(row.map_err(|e| LibreFangError::Internal(e.to_string()))?);
        }
        Ok(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::agent::{ExperimentStatus, ExperimentVariant, SuccessCriteria};

    fn create_test_store() -> PromptStore {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).unwrap();
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS prompt_versions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                content_hash TEXT,
                system_prompt TEXT NOT NULL,
                tools TEXT NOT NULL DEFAULT '[]',
                variables TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                created_by TEXT,
                is_active INTEGER NOT NULL DEFAULT 0,
                description TEXT
            );
            CREATE TABLE IF NOT EXISTS prompt_experiments (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT '\"draft\"',
                traffic_split TEXT NOT NULL DEFAULT '[]',
                success_criteria TEXT NOT NULL DEFAULT '{}',
                started_at TEXT,
                ended_at TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS experiment_variants (
                id TEXT PRIMARY KEY,
                experiment_id TEXT NOT NULL,
                name TEXT NOT NULL,
                prompt_version_id TEXT NOT NULL,
                description TEXT
            );
            CREATE TABLE IF NOT EXISTS experiment_metrics (
                id TEXT PRIMARY KEY,
                experiment_id TEXT NOT NULL,
                variant_id TEXT NOT NULL,
                total_requests INTEGER NOT NULL DEFAULT 0,
                successful_requests INTEGER NOT NULL DEFAULT 0,
                failed_requests INTEGER NOT NULL DEFAULT 0,
                total_latency_ms INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                last_updated TEXT NOT NULL
            );",
            )
            .unwrap();
        PromptStore::new(pool)
    }

    #[test]
    fn test_create_and_list_versions() {
        let store = create_test_store();
        let agent_id = AgentId::new();

        let version = PromptVersion {
            id: Uuid::new_v4(),
            agent_id,
            version: 1,
            content_hash: "abc123".to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            tools: vec![],
            variables: vec![],
            created_at: Utc::now(),
            created_by: "test".to_string(),
            is_active: false,
            description: Some("Test version".to_string()),
        };

        store.create_version(version.clone()).unwrap();

        let versions = store.list_versions(agent_id).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[0].system_prompt, "You are a helpful assistant.");
    }

    #[test]
    fn test_set_active_version() {
        let store = create_test_store();
        let agent_id = AgentId::new();
        let v1_id = Uuid::new_v4();
        let v2_id = Uuid::new_v4();

        store
            .create_version(PromptVersion {
                id: v1_id,
                agent_id,
                version: 1,
                content_hash: "abc".to_string(),
                system_prompt: "Version 1".to_string(),
                tools: vec![],
                variables: vec![],
                created_at: Utc::now(),
                created_by: "test".to_string(),
                is_active: false,
                description: None,
            })
            .unwrap();

        store
            .create_version(PromptVersion {
                id: v2_id,
                agent_id,
                version: 2,
                content_hash: "def".to_string(),
                system_prompt: "Version 2".to_string(),
                tools: vec![],
                variables: vec![],
                created_at: Utc::now(),
                created_by: "test".to_string(),
                is_active: false,
                description: None,
            })
            .unwrap();

        store.set_active_version(v2_id, agent_id).unwrap();

        let active = store.get_active_version(agent_id).unwrap();
        assert!(active.is_some());
        assert_eq!(active.unwrap().version, 2);
    }

    #[test]
    fn test_create_and_list_experiments() {
        let store = create_test_store();
        let agent_id = AgentId::new();

        let experiment = PromptExperiment {
            id: Uuid::new_v4(),
            name: "Test Experiment".to_string(),
            agent_id,
            status: ExperimentStatus::Draft,
            traffic_split: vec![50, 50],
            success_criteria: SuccessCriteria {
                require_user_helpful: false,
                require_no_tool_errors: false,
                require_non_empty: true,
                custom_min_score: None,
            },
            started_at: None,
            ended_at: None,
            created_at: Utc::now(),
            variants: vec![ExperimentVariant {
                id: Uuid::new_v4(),
                name: "Control".to_string(),
                prompt_version_id: Uuid::new_v4(),
                description: None,
            }],
        };

        store.create_experiment(experiment.clone()).unwrap();

        let experiments = store.list_experiments(agent_id).unwrap();
        assert_eq!(experiments.len(), 1);
        assert_eq!(experiments[0].name, "Test Experiment");
    }

    #[test]
    fn test_get_running_experiment() {
        let store = create_test_store();
        let agent_id = AgentId::new();

        let running_exp = PromptExperiment {
            id: Uuid::new_v4(),
            name: "Running Experiment".to_string(),
            agent_id,
            status: ExperimentStatus::Running,
            traffic_split: vec![50, 50],
            success_criteria: SuccessCriteria::default(),
            started_at: Some(Utc::now()),
            ended_at: None,
            created_at: Utc::now(),
            variants: vec![],
        };
        store.create_experiment(running_exp).unwrap();

        let draft_exp = PromptExperiment {
            id: Uuid::new_v4(),
            name: "Draft Experiment".to_string(),
            agent_id,
            status: ExperimentStatus::Draft,
            traffic_split: vec![50, 50],
            success_criteria: SuccessCriteria::default(),
            started_at: None,
            ended_at: None,
            created_at: Utc::now(),
            variants: vec![],
        };
        store.create_experiment(draft_exp).unwrap();

        let running = store.get_running_experiment(agent_id).unwrap();
        assert!(running.is_some());
        assert_eq!(running.unwrap().name, "Running Experiment");
    }
}
