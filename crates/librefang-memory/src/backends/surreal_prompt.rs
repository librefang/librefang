//! SurrealDB-backed implementation of [`PromptBackend`].
//!
//! Persists prompt versions and A/B experiments to the four tables defined in
//! `010_prompt_management.surql`:
//! - `prompt_versions`
//! - `prompt_experiments`
//! - `experiment_variants`
//! - `experiment_metrics`
//!
//! All SurrealQL queries use parameterised bindings (`.bind()`) — no
//! caller-supplied strings are interpolated directly into query text.

use std::sync::Arc;

use chrono::Utc;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

use librefang_storage::SurrealSession;

use librefang_types::agent::{
    AgentId, ExperimentStatus, ExperimentVariantMetrics, PromptExperiment, PromptVersion,
};
use librefang_types::error::{LibreFangError, LibreFangResult};

use crate::backend::PromptBackend;

/// Run a future on the current Tokio runtime or spin up a temporary one.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
        Err(_) => tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(f),
    }
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn parse_prompt_version(row: &serde_json::Value) -> Option<PromptVersion> {
    let id = row.get("id").and_then(|v| {
        // SurrealDB returns id as "prompt_versions:uuid"
        let s = v.as_str()?;
        let id_part = s.split(':').last().unwrap_or(s);
        id_part.parse::<Uuid>().ok()
    })?;
    let agent_id: AgentId = row
        .get("agent_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())?;
    Some(PromptVersion {
        id,
        agent_id,
        version: row.get("version").and_then(|v| v.as_i64()).unwrap_or(0) as u32,
        content_hash: row
            .get("content_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        system_prompt: row
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        tools: vec![],
        variables: vec![],
        created_at: {
            let s = row
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            s.parse().unwrap_or_else(|_| Utc::now())
        },
        created_by: row
            .get("created_by")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        is_active: row
            .get("is_active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        description: row
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_experiment(row: &serde_json::Value) -> Option<PromptExperiment> {
    let id = row.get("id").and_then(|v| {
        let s = v.as_str()?;
        let id_part = s.split(':').last().unwrap_or(s);
        id_part.parse::<Uuid>().ok()
    })?;
    let agent_id: AgentId = row
        .get("agent_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())?;
    // Status is stored as lowercase string (serde rename_all = "lowercase")
    let status_str = row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("draft");
    let status: ExperimentStatus =
        serde_json::from_str(&format!("\"{status_str}\"")).unwrap_or_default();

    Some(PromptExperiment {
        id,
        name: row
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        agent_id,
        status,
        traffic_split: Default::default(),
        success_criteria: Default::default(),
        started_at: row
            .get("started_at")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok()),
        ended_at: row
            .get("ended_at")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok()),
        created_at: {
            let s = row
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            s.parse().unwrap_or_else(|_| Utc::now())
        },
        variants: vec![],
    })
}

// ── SurrealPromptStore ────────────────────────────────────────────────────────

/// SurrealDB implementation of [`PromptBackend`].
#[derive(Clone)]
pub struct SurrealPromptStore {
    db: Arc<Surreal<Any>>,
}

impl SurrealPromptStore {
    /// Open against an existing [`SurrealSession`].
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
        }
    }

    /// Wrap an existing connected SurrealDB instance.
    pub fn new(db: Arc<Surreal<Any>>) -> Self {
        Self { db }
    }
}

impl PromptBackend for SurrealPromptStore {
    fn create_version(&self, version: PromptVersion) -> LibreFangResult<()> {
        let row = serde_json::json!({
            "agent_id": version.agent_id.to_string(),
            "version": version.version as i64,
            "content_hash": version.content_hash,
            "system_prompt": version.system_prompt,
            "tools": serde_json::Value::Array(vec![]),
            "variables": serde_json::Value::Array(vec![]),
            "created_at": version.created_at.to_rfc3339(),
            "created_by": version.created_by,
            "is_active": version.is_active,
            "description": version.description,
        });
        let id = version.id.to_string();
        block_on(async {
            self.db
                .create::<Option<serde_json::Value>>(("prompt_versions", id))
                .content(row)
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB create_version: {e}")))?;
            Ok(())
        })
    }

    fn list_versions(&self, agent_id: AgentId) -> LibreFangResult<Vec<PromptVersion>> {
        let agent = agent_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT * FROM prompt_versions \
                     WHERE agent_id = $agent_id ORDER BY version DESC",
                )
                .bind(("agent_id", agent))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB list_versions: {e}")))?;
            let rows: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB list_versions: {e}")))?;
            Ok(rows
                .iter()
                .filter_map(|r| parse_prompt_version(r))
                .collect())
        })
    }

    fn get_version(&self, id: Uuid) -> LibreFangResult<Option<PromptVersion>> {
        block_on(async {
            let row: Option<serde_json::Value> = self
                .db
                .select(("prompt_versions", id.to_string()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB get_version: {e}")))?;
            Ok(row.as_ref().and_then(|r| parse_prompt_version(r)))
        })
    }

    fn get_active_version(&self, agent_id: AgentId) -> LibreFangResult<Option<PromptVersion>> {
        let agent = agent_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT * FROM prompt_versions \
                     WHERE agent_id = $agent_id AND is_active = true LIMIT 1",
                )
                .bind(("agent_id", agent))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB get_active_version: {e}"))
                })?;
            let rows: Vec<serde_json::Value> = res.take(0).map_err(|e| {
                LibreFangError::Memory(format!("SurrealDB get_active_version: {e}"))
            })?;
            Ok(rows.iter().next().and_then(|r| parse_prompt_version(r)))
        })
    }

    fn set_active_version(&self, id: Uuid, agent_id: AgentId) -> LibreFangResult<()> {
        let agent = agent_id.to_string();
        let version_id = id.to_string();
        block_on(async {
            // Deactivate all versions for the agent first
            self.db
                .query(
                    "UPDATE prompt_versions SET is_active = false \
                     WHERE agent_id = $agent_id",
                )
                .bind(("agent_id", agent))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB deactivate_versions: {e}"))
                })?;
            // Activate the target version using the SDK update API (no string interpolation)
            self.db
                .update::<Option<serde_json::Value>>(("prompt_versions", version_id))
                .merge(serde_json::json!({"is_active": true}))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB set_active_version: {e}"))
                })?;
            Ok(())
        })
    }

    fn delete_version(&self, id: Uuid) -> LibreFangResult<()> {
        block_on(async {
            self.db
                .delete::<Option<serde_json::Value>>(("prompt_versions", id.to_string()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB delete_version: {e}")))?;
            Ok(())
        })
    }

    fn prune_old_versions(&self, agent_id: AgentId, max_versions: u32) -> LibreFangResult<()> {
        let agent = agent_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT count() AS cnt FROM prompt_versions \
                     WHERE agent_id = $agent_id GROUP ALL",
                )
                .bind(("agent_id", agent.clone()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB count_versions: {e}")))?;
            let rows: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB count_versions: {e}")))?;
            let count = rows
                .first()
                .and_then(|r| r.get("cnt"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u32;

            if count <= max_versions {
                return Ok(());
            }

            let excess = (count - max_versions) as i64;
            self.db
                .query(
                    "DELETE (SELECT id FROM prompt_versions \
                     WHERE agent_id = $agent_id AND is_active = false \
                     ORDER BY version ASC LIMIT $excess)",
                )
                .bind(("agent_id", agent))
                .bind(("excess", excess))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB prune_versions: {e}")))?;
            Ok(())
        })
    }

    fn create_version_if_changed(
        &self,
        agent_id: AgentId,
        system_prompt: &str,
        created_by: &str,
    ) -> LibreFangResult<bool> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(system_prompt.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        if let Some(active) = self.get_active_version(agent_id)? {
            if active.content_hash == content_hash {
                return Ok(false);
            }
        }

        let next_version = self.get_latest_version_number(agent_id)? + 1;
        let version = PromptVersion {
            id: Uuid::new_v4(),
            agent_id,
            version: next_version,
            content_hash,
            system_prompt: system_prompt.to_string(),
            tools: vec![],
            variables: vec![],
            created_at: Utc::now(),
            created_by: created_by.to_string(),
            is_active: true,
            description: Some(format!("Auto-tracked v{next_version}")),
        };
        let new_id = version.id;
        self.create_version(version)?;
        self.set_active_version(new_id, agent_id)?;
        Ok(true)
    }

    fn get_latest_version_number(&self, agent_id: AgentId) -> LibreFangResult<u32> {
        let agent = agent_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT math::max(version) AS max_v FROM prompt_versions \
                     WHERE agent_id = $agent_id",
                )
                .bind(("agent_id", agent))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB get_latest_version_number: {e}"))
                })?;
            let rows: Vec<serde_json::Value> = res.take(0).map_err(|e| {
                LibreFangError::Memory(format!("SurrealDB get_latest_version_number: {e}"))
            })?;
            Ok(rows
                .first()
                .and_then(|r| r.get("max_v"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u32)
        })
    }

    fn create_experiment(&self, experiment: PromptExperiment) -> LibreFangResult<()> {
        // Serialize status to its lowercase string form
        let status_str = serde_json::to_string(&experiment.status)
            .unwrap_or_else(|_| "\"draft\"".to_string())
            .trim_matches('"')
            .to_string();
        let row = serde_json::json!({
            "name": experiment.name,
            "agent_id": experiment.agent_id.to_string(),
            "status": status_str,
            "traffic_split": serde_json::json!({}),
            "success_criteria": serde_json::json!({}),
            "started_at": experiment.started_at.map(|t| t.to_rfc3339()),
            "ended_at": experiment.ended_at.map(|t| t.to_rfc3339()),
            "created_at": experiment.created_at.to_rfc3339(),
        });
        let id = experiment.id.to_string();
        block_on(async {
            self.db
                .create::<Option<serde_json::Value>>(("prompt_experiments", id))
                .content(row)
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB create_experiment: {e}")))?;
            Ok(())
        })
    }

    fn list_experiments(&self, agent_id: AgentId) -> LibreFangResult<Vec<PromptExperiment>> {
        let agent = agent_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query("SELECT * FROM prompt_experiments WHERE agent_id = $agent_id")
                .bind(("agent_id", agent))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB list_experiments: {e}")))?;
            let rows: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB list_experiments: {e}")))?;
            Ok(rows.iter().filter_map(|r| parse_experiment(r)).collect())
        })
    }

    fn get_experiment(&self, id: Uuid) -> LibreFangResult<Option<PromptExperiment>> {
        block_on(async {
            let row: Option<serde_json::Value> = self
                .db
                .select(("prompt_experiments", id.to_string()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB get_experiment: {e}")))?;
            Ok(row.as_ref().and_then(|r| parse_experiment(r)))
        })
    }

    fn update_experiment_status(&self, id: Uuid, status: ExperimentStatus) -> LibreFangResult<()> {
        let status_str = serde_json::to_string(&status)
            .unwrap_or_else(|_| "\"draft\"".to_string())
            .trim_matches('"')
            .to_string();
        let exp_id = id.to_string();
        block_on(async {
            // Use the SDK update API — no string interpolation into SurrealQL
            self.db
                .update::<Option<serde_json::Value>>(("prompt_experiments", exp_id))
                .merge(serde_json::json!({"status": status_str}))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB update_experiment_status: {e}"))
                })?;
            Ok(())
        })
    }

    fn get_running_experiment(
        &self,
        agent_id: AgentId,
    ) -> LibreFangResult<Option<PromptExperiment>> {
        let agent = agent_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT * FROM prompt_experiments \
                     WHERE agent_id = $agent_id AND status = 'running' LIMIT 1",
                )
                .bind(("agent_id", agent))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB get_running_experiment: {e}"))
                })?;
            let rows: Vec<serde_json::Value> = res.take(0).map_err(|e| {
                LibreFangError::Memory(format!("SurrealDB get_running_experiment: {e}"))
            })?;
            Ok(rows.iter().next().and_then(|r| parse_experiment(r)))
        })
    }

    fn record_request(
        &self,
        experiment_id: Uuid,
        variant_id: Uuid,
        latency_ms: u64,
        cost_usd: f64,
        success: bool,
    ) -> LibreFangResult<()> {
        let exp_id = experiment_id.to_string();
        let var_id = variant_id.to_string();
        let now = Utc::now().to_rfc3339();
        let (success_delta, failed_delta): (i64, i64) = if success { (1, 0) } else { (0, 1) };
        block_on(async {
            let existing_id = format!("{}_{}", exp_id, var_id);
            let check = self
                .db
                .select::<Option<serde_json::Value>>(("experiment_metrics", existing_id.clone()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB record_request: {e}")))?;

            if check.is_some() {
                // Use type::thing() to construct the record reference without string interpolation
                self.db
                    .query(
                        "UPDATE type::thing('experiment_metrics', $id) SET \
                         total_requests += 1, \
                         successful_requests += $succ, \
                         failed_requests += $fail, \
                         total_latency_ms += $lat, \
                         total_cost_usd += $cost, \
                         last_updated = $now",
                    )
                    .bind(("id", existing_id.clone()))
                    .bind(("succ", success_delta))
                    .bind(("fail", failed_delta))
                    .bind(("lat", latency_ms as i64))
                    .bind(("cost", cost_usd))
                    .bind(("now", now.clone()))
                    .await
                    .map_err(|e| {
                        LibreFangError::Memory(format!("SurrealDB record_request update: {e}"))
                    })?;
            } else {
                let row = serde_json::json!({
                    "experiment_id": exp_id,
                    "variant_id": var_id,
                    "total_requests": 1_i64,
                    "successful_requests": success_delta,
                    "failed_requests": failed_delta,
                    "total_latency_ms": latency_ms as i64,
                    "total_cost_usd": cost_usd,
                    "last_updated": now,
                });
                self.db
                    .create::<Option<serde_json::Value>>(("experiment_metrics", existing_id))
                    .content(row)
                    .await
                    .map_err(|e| {
                        LibreFangError::Memory(format!("SurrealDB record_request insert: {e}"))
                    })?;
            }
            Ok(())
        })
    }

    fn get_variant_metrics(
        &self,
        variant_id: Uuid,
    ) -> LibreFangResult<Option<ExperimentVariantMetrics>> {
        let var_id = variant_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT * FROM experiment_metrics \
                     WHERE variant_id = $variant_id LIMIT 1",
                )
                .bind(("variant_id", var_id))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB get_variant_metrics: {e}"))
                })?;
            let rows: Vec<serde_json::Value> = res.take(0).map_err(|e| {
                LibreFangError::Memory(format!("SurrealDB get_variant_metrics: {e}"))
            })?;
            Ok(rows.into_iter().next().map(|r| {
                let total_requests = r
                    .get("total_requests")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u64;
                let successful_requests = r
                    .get("successful_requests")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u64;
                let failed_requests = r
                    .get("failed_requests")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u64;
                let total_latency_ms = r
                    .get("total_latency_ms")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u64;
                let total_cost_usd = r
                    .get("total_cost_usd")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
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
                ExperimentVariantMetrics {
                    variant_id,
                    variant_name: r
                        .get("variant_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    total_requests,
                    successful_requests,
                    failed_requests,
                    success_rate,
                    avg_latency_ms,
                    avg_cost_usd,
                    total_cost_usd,
                }
            }))
        })
    }

    fn get_experiment_metrics(
        &self,
        experiment_id: Uuid,
    ) -> LibreFangResult<Vec<ExperimentVariantMetrics>> {
        let exp_id = experiment_id.to_string();
        block_on(async {
            let mut res = self
                .db
                .query(
                    "SELECT * FROM experiment_metrics \
                     WHERE experiment_id = $experiment_id",
                )
                .bind(("experiment_id", exp_id))
                .await
                .map_err(|e| {
                    LibreFangError::Memory(format!("SurrealDB get_experiment_metrics: {e}"))
                })?;
            let rows: Vec<serde_json::Value> = res.take(0).map_err(|e| {
                LibreFangError::Memory(format!("SurrealDB get_experiment_metrics: {e}"))
            })?;
            Ok(rows
                .into_iter()
                .filter_map(|r| {
                    let var_id_str = r.get("variant_id")?.as_str()?;
                    let variant_id = var_id_str.parse::<Uuid>().ok()?;
                    let total_requests = r
                        .get("total_requests")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as u64;
                    let successful_requests = r
                        .get("successful_requests")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as u64;
                    let failed_requests = r
                        .get("failed_requests")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as u64;
                    let total_latency_ms = r
                        .get("total_latency_ms")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as u64;
                    let total_cost_usd = r
                        .get("total_cost_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
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
                    Some(ExperimentVariantMetrics {
                        variant_id,
                        variant_name: r
                            .get("variant_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        total_requests,
                        successful_requests,
                        failed_requests,
                        success_rate,
                        avg_latency_ms,
                        avg_cost_usd,
                        total_cost_usd,
                    })
                })
                .collect())
        })
    }
}
