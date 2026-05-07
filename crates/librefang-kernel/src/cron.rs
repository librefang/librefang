//! Cron job scheduler engine for the LibreFang kernel.
//!
//! Manages scheduled jobs (recurring and one-shot) across all agents.
//! This is separate from `scheduler.rs` which handles agent resource tracking.
//!
//! The scheduler stores jobs in a `DashMap` for concurrent access, persists
//! them to a JSON file on disk, and exposes methods for the kernel tick loop
//! to query due jobs and record outcomes.

use chrono::{Duration, Utc};
use dashmap::DashMap;
use librefang_types::agent::{AgentId, SessionId, SessionMode};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::scheduler::{CronJob, CronJobId, CronSchedule};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, info, warn};

/// Maximum consecutive errors before a job is auto-disabled.
const MAX_CONSECUTIVE_ERRORS: u32 = 5;

// ---------------------------------------------------------------------------
// JobMeta — extra runtime state not stored in CronJob itself
// ---------------------------------------------------------------------------

/// Runtime metadata for a cron job that extends the base `CronJob` type.
///
/// The `CronJob` struct in `librefang-types` is intentionally lean (no
/// `one_shot`, `last_status`, or error tracking). The scheduler tracks
/// these operational details separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMeta {
    /// The underlying job definition.
    pub job: CronJob,
    /// Whether this job should be removed after a single successful execution.
    pub one_shot: bool,
    /// Human-readable status of the last execution (e.g. `"ok"` or `"error: ..."`).
    pub last_status: Option<String>,
    /// Number of consecutive failed executions.
    pub consecutive_errors: u32,
    /// True when the job was disabled automatically by the scheduler after
    /// repeated failures — as opposed to being manually disabled by the user.
    /// Only auto-disabled jobs are re-enabled on agent reassignment.
    #[serde(default)]
    pub auto_disabled: bool,
}

impl JobMeta {
    /// Wrap a `CronJob` with default metadata.
    pub fn new(job: CronJob, one_shot: bool) -> Self {
        Self {
            job,
            one_shot,
            last_status: None,
            consecutive_errors: 0,
            auto_disabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// CronScheduler
// ---------------------------------------------------------------------------

/// Cron job scheduler — manages scheduled jobs for all agents.
///
/// Thread-safe via `DashMap`. The kernel should call [`due_jobs`] on a
/// regular interval (e.g. every 10-30 seconds) to discover jobs that need
/// to fire, then call [`record_success`] or [`record_failure`] after
/// execution completes.
pub struct CronScheduler {
    /// All tracked jobs, keyed by their unique ID.
    jobs: DashMap<CronJobId, JobMeta>,
    /// Path to the persistence file (`<home>/cron_jobs.json`).
    persist_path: PathBuf,
    /// Daemon home directory (e.g. `~/.librefang`). Used to enforce the
    /// `<home>/scripts/` allowlist on `pre_script.argv[0]` at validation time.
    home_dir: PathBuf,
    /// Global cap on total jobs across all agents (atomic for hot-reload).
    max_total_jobs: AtomicUsize,
    /// Serializes `persist()` writes so concurrent callers (cron loop, API
    /// routes, spawned cron tasks) don't corrupt the tmp file by interleaving
    /// `O_TRUNC`/write/rename on the same path.
    persist_lock: std::sync::Mutex<()>,
}

impl CronScheduler {
    /// Create a new scheduler.
    ///
    /// `home_dir` is the LibreFang home directory; jobs are persisted to
    /// `<home_dir>/data/cron_jobs.json`. `max_total_jobs` caps the total
    /// number of jobs across all agents.
    pub fn new(home_dir: &Path, max_total_jobs: usize) -> Self {
        Self {
            jobs: DashMap::new(),
            persist_path: home_dir.join("data").join("cron_jobs.json"),
            home_dir: home_dir.to_path_buf(),
            max_total_jobs: AtomicUsize::new(max_total_jobs),
            persist_lock: std::sync::Mutex::new(()),
        }
    }

    /// Update the max total jobs limit (for hot-reload).
    pub fn set_max_total_jobs(&self, new_max: usize) {
        self.max_total_jobs.store(new_max, Ordering::Relaxed);
    }

    // -- Persistence --------------------------------------------------------

    /// Load persisted jobs from disk.
    ///
    /// Returns the number of jobs loaded. If the persistence file does not
    /// exist, returns `Ok(0)` without error.
    pub fn load(&self) -> LibreFangResult<usize> {
        if !self.persist_path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&self.persist_path)
            .map_err(|e| LibreFangError::Internal(format!("Failed to read cron jobs: {e}")))?;
        let metas: Vec<JobMeta> = serde_json::from_str(&data)
            .map_err(|e| LibreFangError::Internal(format!("Failed to parse cron jobs: {e}")))?;
        let count = metas.len();
        for meta in metas {
            self.jobs.insert(meta.job.id, meta);
        }
        info!(count, "Loaded cron jobs from disk");
        Ok(count)
    }

    /// Persist all jobs to disk via atomic write (write to `.tmp`, then rename).
    ///
    /// Serialized through `persist_lock` so concurrent callers can't both
    /// `O_TRUNC` the same `.tmp` path and produce a torn file before rename.
    pub fn persist(&self) -> LibreFangResult<()> {
        let _guard = self.persist_lock.lock().unwrap_or_else(|e| e.into_inner());
        let metas: Vec<JobMeta> = self.jobs.iter().map(|r| r.value().clone()).collect();
        let data = serde_json::to_string_pretty(&metas)
            .map_err(|e| LibreFangError::Internal(format!("Failed to serialize cron jobs: {e}")))?;
        if let Some(parent) = self.persist_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                LibreFangError::Internal(format!("Failed to create cron jobs dir: {e}"))
            })?;
        }
        let tmp_path = crate::persist_tmp_path(&self.persist_path);
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp_path).map_err(|e| {
                LibreFangError::Internal(format!("Failed to create cron jobs temp file: {e}"))
            })?;
            f.write_all(data.as_bytes()).map_err(|e| {
                LibreFangError::Internal(format!("Failed to write cron jobs temp file: {e}"))
            })?;
            f.sync_all().map_err(|e| {
                LibreFangError::Internal(format!("Failed to fsync cron jobs temp file: {e}"))
            })?;
        }
        std::fs::rename(&tmp_path, &self.persist_path).map_err(|e| {
            LibreFangError::Internal(format!("Failed to rename cron jobs file: {e}"))
        })?;
        debug!(count = metas.len(), "Persisted cron jobs");
        Ok(())
    }

    // -- CRUD ---------------------------------------------------------------

    /// Add a new job. Validates fields, computes the initial `next_run`,
    /// and inserts it into the scheduler.
    ///
    /// `one_shot` controls whether the job is removed after a single
    /// successful execution.
    pub fn add_job(&self, mut job: CronJob, one_shot: bool) -> LibreFangResult<CronJobId> {
        // Global limit
        let max_jobs = self.max_total_jobs.load(Ordering::Relaxed);
        if self.jobs.len() >= max_jobs {
            return Err(LibreFangError::Internal(format!(
                "Global cron job limit reached ({})",
                max_jobs
            )));
        }

        // Per-agent count
        let agent_count = self
            .jobs
            .iter()
            .filter(|r| r.value().job.agent_id == job.agent_id)
            .count();

        // CronJob.validate_with_home returns Result<(), String>.
        // Passing `Some(home_dir)` enables the `<home>/scripts/` allowlist
        // check on `pre_script.argv[0]` and the dangerous-env-key denylist
        // on `pre_script.env` (defends against `LD_PRELOAD`, `PATH`, etc.).
        job.validate_with_home(agent_count, Some(&self.home_dir))
            .map_err(LibreFangError::InvalidInput)?;

        // Compute initial next_run
        job.next_run = Some(compute_next_run(&job.schedule));

        // Defense-in-depth: At-schedules must always be one_shot regardless of
        // what the caller passed (#2808).
        let one_shot = one_shot || matches!(job.schedule, CronSchedule::At { .. });

        let id = job.id;
        self.jobs.insert(id, JobMeta::new(job, one_shot));
        Ok(id)
    }

    /// Remove a job by ID. Returns the removed `CronJob`.
    pub fn remove_job(&self, id: CronJobId) -> LibreFangResult<CronJob> {
        self.jobs
            .remove(&id)
            .map(|(_, meta)| meta.job)
            .ok_or_else(|| LibreFangError::Internal(format!("Cron job {id} not found")))
    }

    /// Enable or disable a job. Re-enabling resets errors and recomputes
    /// `next_run`.
    pub fn set_enabled(&self, id: CronJobId, enabled: bool) -> LibreFangResult<()> {
        match self.jobs.get_mut(&id) {
            Some(mut meta) => {
                meta.job.enabled = enabled;
                // Any explicit enable/disable by the user clears the auto_disabled
                // flag so the scheduler won't accidentally re-enable a job the
                // user deliberately turned off.
                meta.auto_disabled = false;
                if enabled {
                    meta.consecutive_errors = 0;
                    meta.job.next_run = Some(compute_next_run(&meta.job.schedule));
                }
                Ok(())
            }
            None => Err(LibreFangError::Internal(format!("Cron job {id} not found"))),
        }
    }

    /// Update a cron job's configuration in place.
    ///
    /// Supported fields in `updates`: `name`, `enabled`, `schedule`, `action`,
    /// `delivery`, `agent_id`.  Only provided (non-null) fields are patched;
    /// omitted fields keep their current values.
    pub fn update_job(
        &self,
        id: CronJobId,
        updates: &serde_json::Value,
    ) -> LibreFangResult<CronJob> {
        // Candidate-validate-swap: clone the current job, apply the
        // partial updates onto the candidate, run the same `validate(0)`
        // that `add_job` runs, and only after that passes do we swap the
        // candidate into place under the shard lock. This generalises
        // the #4732 bypass closure from "delivery / delivery_targets
        // re-validated on update" to "the entire CronJob shape is
        // re-validated on update" — name length, schedule cron-expr
        // syntax, every CronAction shape, and the SSRF / path checks all
        // gate update the same way they gate add. A pre-#4739 PUT
        // carrying e.g. an empty `name` plus a valid `delivery` was
        // accepted; now the same payload is rejected before any field
        // hits live state.
        //
        // Atomicity: `meta.job` is replaced once with a fully validated
        // candidate, so an `Err` at any step leaves the live row
        // untouched. The earlier in-place pattern could commit `delivery`
        // before failing on `delivery_targets`; that race is gone.
        let mut candidate = match self.jobs.get(&id) {
            Some(entry) => entry.value().job.clone(),
            None => {
                return Err(LibreFangError::Internal(format!("Cron job {id} not found")));
            }
        };

        let enabled_updated = updates["enabled"].as_bool();
        let schedule_updated = !updates["schedule"].is_null();

        if let Some(name) = updates["name"].as_str() {
            candidate.name = name.to_string();
        }
        if let Some(enabled) = enabled_updated {
            candidate.enabled = enabled;
        }
        if let Some(s) = updates["agent_id"].as_str() {
            candidate.agent_id = s
                .parse::<AgentId>()
                .map_err(|e| LibreFangError::Internal(format!("Invalid agent_id: {e}")))?;
        }
        if schedule_updated {
            candidate.schedule =
                serde_json::from_value::<CronSchedule>(updates["schedule"].clone())
                    .map_err(|e| LibreFangError::Internal(format!("Invalid schedule: {e}")))?;
        }
        if !updates["action"].is_null() {
            candidate.action = serde_json::from_value::<librefang_types::scheduler::CronAction>(
                updates["action"].clone(),
            )
            .map_err(|e| LibreFangError::Internal(format!("Invalid action: {e}")))?;
        }
        if !updates["delivery"].is_null() {
            candidate.delivery =
                serde_json::from_value::<librefang_types::scheduler::CronDelivery>(
                    updates["delivery"].clone(),
                )
                .map_err(|e| LibreFangError::Internal(format!("Invalid delivery: {e}")))?;
        }
        if !updates["delivery_targets"].is_null() {
            candidate.delivery_targets = serde_json::from_value::<
                Vec<librefang_types::scheduler::CronDeliveryTarget>,
            >(updates["delivery_targets"].clone())
            .map_err(|e| LibreFangError::Internal(format!("Invalid delivery_targets: {e}")))?;
        }

        // Run the same shape + SSRF validation `add_job` runs. We pass
        // `existing_count = 0` because this is an in-place update on an
        // existing job — capacity (MAX_JOBS_PER_AGENT) is unaffected by
        // an update that doesn't change `agent_id`. Cross-agent moves
        // are NOT capacity-checked here today; tracking under a
        // separate follow-up issue (#4732 followup).
        candidate
            .validate(0)
            .map_err(LibreFangError::InvalidInput)?;

        // Recompute next_run when the schedule shape changed, OR when
        // the job is being re-enabled (mirrors the prior in-place
        // semantics so an existing job that was paused with a stale
        // next_run gets a fresh tick on activation).
        if schedule_updated || matches!(enabled_updated, Some(true)) {
            candidate.next_run = Some(compute_next_run(&candidate.schedule));
        }

        match self.jobs.get_mut(&id) {
            Some(mut entry) => {
                let meta = entry.value_mut();
                if let Some(enabled) = enabled_updated {
                    // An explicit toggle from the user clears the
                    // auto_disabled flag regardless of direction.
                    meta.auto_disabled = false;
                    if enabled {
                        meta.consecutive_errors = 0;
                    }
                }
                meta.job = candidate;
                Ok(meta.job.clone())
            }
            None => Err(LibreFangError::Internal(format!("Cron job {id} not found"))),
        }
    }

    /// Replace the multi-destination delivery targets on an existing job.
    ///
    /// The schedule, action, and primary `delivery` field are left untouched;
    /// only the `delivery_targets` fan-out list is swapped in. Call
    /// [`Self::persist`] afterwards to write the change to disk.
    pub fn set_delivery_targets(
        &self,
        id: CronJobId,
        targets: Vec<librefang_types::scheduler::CronDeliveryTarget>,
    ) -> LibreFangResult<()> {
        // Validate before swapping so an SSRF-blocking webhook host or an
        // absolute LocalFile path is rejected at the same input boundary
        // `add_job` enforces (#4732).
        librefang_types::scheduler::validate_cron_delivery_targets(&targets)
            .map_err(LibreFangError::InvalidInput)?;
        match self.jobs.get_mut(&id) {
            Some(mut meta) => {
                meta.job.delivery_targets = targets;
                Ok(())
            }
            None => Err(LibreFangError::Internal(format!("Cron job {id} not found"))),
        }
    }

    // -- Queries ------------------------------------------------------------

    /// Get a single job by ID.
    pub fn get_job(&self, id: CronJobId) -> Option<CronJob> {
        self.jobs.get(&id).map(|r| r.value().job.clone())
    }

    /// Get the full metadata for a job (includes `one_shot`, `last_status`,
    /// `consecutive_errors`).
    pub fn get_meta(&self, id: CronJobId) -> Option<JobMeta> {
        self.jobs.get(&id).map(|r| r.value().clone())
    }

    /// List all jobs for a specific agent.
    pub fn list_jobs(&self, agent_id: AgentId) -> Vec<CronJob> {
        self.jobs
            .iter()
            .filter(|r| r.value().job.agent_id == agent_id)
            .map(|r| r.value().job.clone())
            .collect()
    }

    /// List all jobs across all agents.
    pub fn list_all_jobs(&self) -> Vec<CronJob> {
        self.jobs.iter().map(|r| r.value().job.clone()).collect()
    }

    /// Reassign all cron jobs from `old_agent_id` to `new_agent_id`.
    ///
    /// Used when a hand agent is respawned (e.g. after daemon restart) and
    /// gets a new UUID. Without this, persisted cron jobs would reference
    /// the stale old agent ID and fail silently.
    ///
    /// Returns the number of jobs reassigned.
    pub fn reassign_agent_jobs(&self, old_agent_id: AgentId, new_agent_id: AgentId) -> usize {
        let mut count = 0;
        for mut entry in self.jobs.iter_mut() {
            if entry.value().job.agent_id == old_agent_id {
                entry.value_mut().job.agent_id = new_agent_id;
                // Reset consecutive errors so the job gets a fresh start
                // with the new agent.
                entry.value_mut().consecutive_errors = 0;
                if !entry.value().job.enabled && entry.value().auto_disabled {
                    // Re-enable only jobs that were auto-disabled by the scheduler
                    // (stale agent ID → repeated failures). Jobs the user deliberately
                    // turned off have auto_disabled=false and are left alone.
                    entry.value_mut().job.enabled = true;
                    entry.value_mut().auto_disabled = false;
                    entry.value_mut().job.next_run =
                        Some(compute_next_run(&entry.value().job.schedule));
                }
                count += 1;
            }
        }
        if count > 0 {
            info!(
                old_agent = %old_agent_id,
                new_agent = %new_agent_id,
                count,
                "Reassigned cron jobs to new agent"
            );
        }
        count
    }

    /// Warn about cron fires that were missed while the daemon was offline.
    ///
    /// Should be called immediately after [`Self::load`] on daemon startup.
    /// Any enabled job whose `next_run` is more than 60 seconds in the past
    /// is considered to have missed at least one fire during downtime. The
    /// method logs a warning with the estimated missed-fire count and
    /// immediately reschedules the job to fire on the next tick (by setting
    /// `next_run = now`) so the scheduler can catch up without further delay.
    ///
    /// The 60-second grace window prevents false positives for jobs that
    /// were just about to fire when the daemon stopped.
    pub fn warn_missed_fires(&self) {
        let now = Utc::now();
        for mut entry in self.jobs.iter_mut() {
            let meta = entry.value_mut();
            if !meta.job.enabled {
                continue;
            }
            if let Some(next_run) = meta.job.next_run {
                let grace = Duration::seconds(60);
                if next_run < now - grace {
                    let overdue_secs = (now - next_run).num_seconds();
                    // Estimate how many fires were skipped based on schedule interval.
                    let interval_secs: i64 = match &meta.job.schedule {
                        CronSchedule::Every { every_secs } => *every_secs as i64,
                        CronSchedule::At { .. } => overdue_secs, // one-shot: effectively 1 missed fire
                        CronSchedule::Cron { .. } => {
                            // For cron expressions, approximate with the gap between
                            // `next_run` and what `next_run` would have been after one cycle.
                            let hypothetical_next =
                                compute_next_run_after(&meta.job.schedule, next_run);
                            (hypothetical_next - next_run).num_seconds().max(1)
                        }
                    };
                    let missed_count = (overdue_secs / interval_secs).max(1);
                    warn!(
                        agent_id = %meta.job.agent_id,
                        job_id = %meta.job.id,
                        missed_count,
                        overdue_secs,
                        "cron job missed fires during daemon downtime; firing now"
                    );
                    // Reschedule to fire immediately on the next tick.
                    meta.job.next_run = Some(now);
                }
            }
        }
    }

    /// Remove all cron jobs belonging to a specific agent.
    ///
    /// Used when an agent is deleted so its cron entries don't linger as
    /// orphans pointing at a dead UUID. Returns the number of jobs removed.
    pub fn remove_agent_jobs(&self, agent_id: AgentId) -> usize {
        let ids: Vec<CronJobId> = self
            .jobs
            .iter()
            .filter(|r| r.value().job.agent_id == agent_id)
            .map(|r| *r.key())
            .collect();
        let count = ids.len();
        for id in ids {
            self.jobs.remove(&id);
        }
        if count > 0 {
            info!(agent = %agent_id, count, "Removed cron jobs for deleted agent");
        }
        count
    }

    /// Total number of tracked jobs.
    pub fn total_jobs(&self) -> usize {
        self.jobs.len()
    }

    /// Return jobs whose `next_run` is at or before `now` and are enabled.
    ///
    /// **Important**: This also pre-advances each due job's `next_run` to the
    /// next scheduled time. This prevents the same job from being returned as
    /// "due" on subsequent tick iterations while it's still executing.
    pub fn due_jobs(&self) -> Vec<CronJob> {
        let now = Utc::now();
        let mut due = Vec::new();
        for mut entry in self.jobs.iter_mut() {
            let meta = entry.value_mut();
            if meta.job.enabled && meta.job.next_run.map(|t| t <= now).unwrap_or(false) {
                due.push(meta.job.clone());
                // Pre-advance next_run so the job won't fire again on the next
                // tick while it's still executing. Use `now` as the base so the
                // next fire time is computed strictly after the current moment.
                meta.job.next_run = Some(compute_next_run_after(&meta.job.schedule, now));
            }
        }
        due
    }

    /// Mark all enabled cron jobs for a given agent as due immediately.
    /// The next `due_jobs()` tick will pick them up.
    /// Called when a provider is configured so Hands resume without waiting.
    pub fn mark_due_now_by_agent(&self, agent_id: AgentId) {
        let now = Utc::now();
        for mut entry in self.jobs.iter_mut() {
            let meta = entry.value_mut();
            if meta.job.agent_id == agent_id && meta.job.enabled {
                meta.job.next_run = Some(now);
            }
        }
    }

    /// Log warnings for any cron jobs that should have fired between
    /// `since` (typically the daemon's previous shutdown time) and `now`
    /// but were missed due to the daemon being down.
    ///
    /// This is called once at daemon startup, after jobs are loaded from
    /// disk. It does **not** catch-up-fire the missed jobs — it only
    /// produces `warn!` log entries so operators can see what was skipped
    /// (Bug #3828).
    ///
    /// The function works by walking `next_run` backwards in time: for
    /// each enabled job whose `last_run` is older than `since`, it counts
    /// how many times the schedule would have fired in `[since, now)` and
    /// emits one warning per missed fire.  For `Every` schedules this is
    /// an exact count; for `Cron` expression schedules it is approximate
    /// (iterates up to 1440 times per job to avoid pathological inputs).
    /// `At` one-shot jobs that have already passed are silently ignored
    /// (they would have been removed on successful execution anyway).
    ///
    /// Distinct from [`Self::warn_missed_fires`] (no-arg), which both
    /// logs and reschedules overdue jobs for catch-up firing. Both were
    /// independently introduced as fixes for #3828 in PRs #3906 and
    /// #3923 and ended up colliding on the same name; this one is the
    /// since-windowed log-only variant.
    pub fn log_missed_fires_since(&self, since: chrono::DateTime<Utc>) {
        let now = Utc::now();
        if since >= now {
            return;
        }
        for entry in self.jobs.iter() {
            let meta = entry.value();
            if !meta.job.enabled {
                continue;
            }
            // One-shot At-schedules that already passed are expected to be
            // gone; silence them to avoid false-positive noise.
            if let CronSchedule::At { at } = &meta.job.schedule {
                if *at < since {
                    continue;
                }
            }
            // Find the first time this job *should* have fired after `since`.
            let mut cursor = compute_next_run_after(&meta.job.schedule, since);
            let mut missed_count = 0usize;
            // Safety cap: stop after 1440 iterations (≈1 minute-resolution
            // cron firing every minute for a day) to avoid spinning on
            // high-frequency schedules.
            const MAX_ITER: usize = 1440;
            while cursor < now && missed_count < MAX_ITER {
                missed_count += 1;
                cursor = compute_next_run_after(&meta.job.schedule, cursor);
            }
            if missed_count > 0 {
                warn!(
                    job = %meta.job.name,
                    job_id = %meta.job.id,
                    agent = %meta.job.agent_id,
                    missed = missed_count,
                    since = %since.format("%Y-%m-%dT%H:%M:%SZ"),
                    "Cron: missed {} fire(s) while daemon was down",
                    missed_count
                );
            }
        }
    }

    // -- Outcome recording --------------------------------------------------

    /// Record a successful execution for a job.
    ///
    /// Updates `last_run`, resets errors, and either removes the job (if
    /// one-shot) or advances `next_run`.
    pub fn record_success(&self, id: CronJobId) {
        // We need to check one_shot first, then potentially remove.
        let should_remove = {
            if let Some(mut meta) = self.jobs.get_mut(&id) {
                meta.job.last_run = Some(Utc::now());
                meta.last_status = Some("ok".to_string());
                meta.consecutive_errors = 0;
                // one_shot jobs get removed; recurring jobs keep the next_run
                // already pre-advanced by due_jobs() — no recompute needed.
                meta.one_shot
            } else {
                return;
            }
        };
        if should_remove {
            self.jobs.remove(&id);
        }
    }

    /// Record a skipped execution for a job (e.g. agent was Suspended).
    ///
    /// Sets `last_status` to `"skipped"` without touching error counters.
    ///
    /// For recurring jobs the job remains scheduled at its next_run.
    /// For one_shot jobs (At schedule, manual one-shot) the only
    /// scheduled fire has now passed, so the job is removed from the
    /// scheduler — otherwise compute_next_run_after pre-advances
    /// next_run to far-future and the job lingers in jobs.json
    /// forever, surfacing in /api/cron as inert garbage.  Audit of
    /// #3923 caught this; remove on skip the same way record_success
    /// does for one_shot.
    pub fn record_skipped(&self, id: CronJobId) {
        let should_remove = if let Some(mut meta) = self.jobs.get_mut(&id) {
            meta.last_status = Some("skipped: agent suspended".to_string());
            debug!(job_id = %id, "Cron job skipped (agent suspended)");
            meta.one_shot
        } else {
            false
        };
        if should_remove {
            self.jobs.remove(&id);
            debug!(job_id = %id, "Removed one-shot cron job after skip");
        }
    }

    /// Record a failed execution for a job.
    ///
    /// Increments the consecutive error counter. If it reaches
    /// [`MAX_CONSECUTIVE_ERRORS`], the job is automatically disabled.
    pub fn record_failure(&self, id: CronJobId, error_msg: &str) {
        let should_remove = if let Some(mut meta) = self.jobs.get_mut(&id) {
            meta.job.last_run = Some(Utc::now());
            meta.last_status = Some(format!(
                "error: {}",
                librefang_types::truncate_str(error_msg, 256)
            ));
            meta.consecutive_errors += 1;
            if meta.one_shot {
                // one_shot jobs (e.g. At-schedule) are removed after the first
                // failure too — there is no meaningful retry for a one-time job
                // whose scheduled moment has already passed (#2808).
                true
            } else if meta.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                warn!(
                    job_id = %id,
                    errors = meta.consecutive_errors,
                    "Auto-disabling cron job after repeated failures"
                );
                meta.job.enabled = false;
                meta.auto_disabled = true;
                false
            } else {
                meta.job.next_run = Some(compute_next_run_after(&meta.job.schedule, Utc::now()));
                false
            }
        } else {
            false
        };
        if should_remove {
            self.jobs.remove(&id);
        }
    }
}

// ---------------------------------------------------------------------------
// compute_next_run
// ---------------------------------------------------------------------------

/// Compute the next fire time for a schedule, based on `now`.
///
/// - `At { at }` — returns `at` directly.
/// - `Every { every_secs }` — returns `now + every_secs`.
/// - `Cron { expr, tz }` — parses the cron expression and computes the next
///   matching time. Supports standard 5-field (`min hour dom month dow`) and
///   6-field (`sec min hour dom month dow`) formats by converting to the
///   7-field format required by the `cron` crate.
pub fn compute_next_run(schedule: &CronSchedule) -> chrono::DateTime<Utc> {
    compute_next_run_after(schedule, Utc::now())
}

/// Compute the next fire time for a schedule, strictly after `after`.
///
/// Uses `after + 1 second` as the base time so the `cron` crate's
/// inclusive `.after()` always returns a strictly future time. Without
/// this offset, calling `compute_next_run` right after a job fires can
/// return the same minute (or even the same second), causing the
/// scheduler to re-fire immediately.
///
/// # DST safety
///
/// All fire times are stored and compared in UTC. `chrono::Local` is never
/// used internally — even when a job specifies a named `tz` (e.g.
/// `"America/New_York"`), the computation converts `after` to that timezone
/// only to honour the user's wall-clock intent, then immediately converts
/// the result back to UTC before storing it. This means the scheduler is
/// immune to DST transitions: a "09:00 daily" job in a DST-observing
/// timezone will naturally shift by one UTC hour at the clock change, but
/// will never fire twice or be skipped.
pub fn compute_next_run_after(
    schedule: &CronSchedule,
    after: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
    match schedule {
        // For `at` schedules, return the original time only if it's still
        // in the future. Otherwise the scheduler would see `next_run <= now`
        // forever and fire the job on every tick (every 15s) until the
        // process restarts. Push it to the far future so the job never
        // fires again. Issue #2337.
        CronSchedule::At { at } => {
            if *at > after {
                *at
            } else {
                after + Duration::days(36500)
            }
        }
        CronSchedule::Every { every_secs } => after + Duration::seconds(*every_secs as i64),
        CronSchedule::Cron { expr, tz } => {
            // Convert standard 5/6-field cron to 7-field for the `cron` crate.
            // Standard 5-field: min hour dom month dow
            // 6-field:          sec min hour dom month dow
            // cron crate:       sec min hour dom month dow year
            let trimmed = expr.trim();
            let fields: Vec<&str> = trimmed.split_whitespace().collect();
            let seven_field = match fields.len() {
                5 => format!("0 {trimmed} *"),
                6 => format!("{trimmed} *"),
                _ => expr.clone(),
            };

            // Add 1 second so `.after()` (inclusive) skips the current second.
            let base = after + Duration::seconds(1);

            match seven_field.parse::<cron::Schedule>() {
                Ok(sched) => {
                    // If a timezone is specified, compute the next fire time in
                    // that timezone so DST and local offsets are respected, then
                    // convert back to UTC for storage.
                    let next_utc = match tz.as_deref() {
                        Some(tz_str) if !tz_str.is_empty() && tz_str != "UTC" => {
                            match tz_str.parse::<chrono_tz::Tz>() {
                                Ok(timezone) => {
                                    let base_local = base.with_timezone(&timezone);
                                    let result = sched
                                        .after(&base_local)
                                        .next()
                                        .map(|dt| dt.with_timezone(&Utc));

                                    // Warn when a DST boundary is crossed: spring-forward may
                                    // silently skip a scheduled local time; fall-back picks the
                                    // earlier UTC occurrence.
                                    if let Some(next) = result {
                                        let next_local = next.with_timezone(&timezone);
                                        let base_utc_secs = (base_local.naive_utc()
                                            - base_local.naive_local())
                                        .num_seconds();
                                        let next_utc_secs = (next_local.naive_utc()
                                            - next_local.naive_local())
                                        .num_seconds();
                                        if base_utc_secs != next_utc_secs {
                                            warn!(
                                                expr = %expr,
                                                timezone = %tz_str,
                                                base_local = %base_local.format("%Y-%m-%dT%H:%M:%S%z"),
                                                adjusted_utc = %next.format("%Y-%m-%dT%H:%M:%SZ"),
                                                adjusted_local = %next_local.format("%Y-%m-%dT%H:%M:%S%z"),
                                                "Cron job next-fire crosses a DST boundary in \
                                                 timezone '{}'; scheduled local time may have been \
                                                 skipped (spring-forward) or moved to the first \
                                                 occurrence (fall-back). Next fire adjusted to {}",
                                                tz_str,
                                                next.format("%Y-%m-%dT%H:%M:%SZ"),
                                            );
                                        }
                                    }

                                    result
                                }
                                Err(_) => {
                                    warn!(
                                        "Invalid timezone '{}' in cron job, falling back to UTC",
                                        tz_str
                                    );
                                    sched.after(&base).next()
                                }
                            }
                        }
                        _ => sched.after(&base).next(),
                    };
                    next_utc.unwrap_or_else(|| after + Duration::hours(1))
                }
                Err(e) => {
                    warn!("Failed to parse cron expression '{}': {}", expr, e);
                    after + Duration::hours(1)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-fire session derivation
// ---------------------------------------------------------------------------

/// Compute `(session_mode_override, session_id_override)` for a cron fire.
///
/// `session_mode = Some(New)` means each fire must land on its own isolated
/// session: the channel-derived branch in `send_message_full` would otherwise
/// always route cron back to the persistent `(agent, "cron")` session
/// because the synthetic `SenderContext{channel:"cron"}` wins over a
/// session-mode override (see CLAUDE.md note on cron + session_mode). We
/// bypass that by handing `send_message_full` an explicit
/// `session_id_override` derived from the job id and fire timestamp via
/// [`SessionId::for_cron_run`], so the override path takes priority over
/// the channel branch.
///
/// `Persistent` (or `None` — historical default) returns `(None, None)`,
/// preserving the long-standing `(agent, "cron")` shared-session behaviour.
pub fn cron_fire_session_override(
    agent_id: AgentId,
    job_session_mode: Option<SessionMode>,
    job_id: CronJobId,
    fire_time: chrono::DateTime<chrono::Utc>,
) -> (Option<SessionMode>, Option<SessionId>) {
    if job_session_mode != Some(SessionMode::New) {
        return (None, None);
    }
    let run_key = format!(
        "{}:{}",
        job_id.0,
        fire_time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
    );
    (
        Some(SessionMode::New),
        Some(SessionId::for_cron_run(agent_id, &run_key)),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Timelike};
    use librefang_types::scheduler::{CronAction, CronDelivery};

    #[test]
    fn fire_session_override_persistent_returns_none() {
        let agent = AgentId::new();
        let job_id = CronJobId::new();
        let now = chrono::Utc::now();
        // Default (no per-job override) — historical persistent cron session.
        let (mode, sid) = cron_fire_session_override(agent, None, job_id, now);
        assert!(mode.is_none());
        assert!(sid.is_none());
        // Explicit Persistent same.
        let (mode, sid) =
            cron_fire_session_override(agent, Some(SessionMode::Persistent), job_id, now);
        assert!(mode.is_none());
        assert!(sid.is_none());
    }

    #[test]
    fn fire_session_override_new_yields_isolated_id() {
        let agent = AgentId::new();
        let job_id = CronJobId::new();
        let now = chrono::Utc::now();
        let (mode, sid) = cron_fire_session_override(agent, Some(SessionMode::New), job_id, now);
        assert_eq!(mode, Some(SessionMode::New));
        let sid = sid.expect("New must produce a session id override");
        // And it must NOT collide with the persistent (agent, "cron") session.
        assert_ne!(sid, SessionId::for_channel(agent, "cron"));
    }

    #[test]
    fn fire_session_override_new_distinguishes_two_fires() {
        let agent = AgentId::new();
        let job_id = CronJobId::new();
        // Two distinct timestamps representing two fires.
        let t1 = chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap();
        let t2 = chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 5, 0).unwrap();
        let (_, sid_a) = cron_fire_session_override(agent, Some(SessionMode::New), job_id, t1);
        let (_, sid_b) = cron_fire_session_override(agent, Some(SessionMode::New), job_id, t2);
        assert_ne!(
            sid_a, sid_b,
            "two fires of the same New-mode job must yield distinct session ids"
        );
    }

    #[test]
    fn fire_session_override_new_is_deterministic_per_fire() {
        // Reproducibility: same (agent, job_id, fire_time) must always derive
        // the same session id — useful for log correlation when a fire's
        // session id is referenced after the fact.
        let agent = AgentId::new();
        let job_id = CronJobId::new();
        let t = chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap();
        let (_, sid_a) = cron_fire_session_override(agent, Some(SessionMode::New), job_id, t);
        let (_, sid_b) = cron_fire_session_override(agent, Some(SessionMode::New), job_id, t);
        assert_eq!(sid_a, sid_b);
    }

    /// Regression for #3657: pin the exact session id a `New`-mode cron fire
    /// receives. The CLAUDE.md cron + session_mode note documents the
    /// derivation as `SessionId::for_cron_run(agent, "<job_id>:<rfc3339_fire>")`
    /// — if anyone changes the run_key shape (timestamp precision, separator,
    /// ordering) without updating the doc, this test fails loudly. It also
    /// guarantees the function actually routes through `for_cron_run` rather
    /// than re-deriving via `for_channel("cron")`, which was the bug the
    /// dispatcher change in #3597 fixed.
    #[test]
    fn fire_session_override_new_matches_for_cron_run_contract_3657() {
        let agent = AgentId::new();
        let job_id = CronJobId::new();
        let fire_time = chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap();
        let (mode, sid) =
            cron_fire_session_override(agent, Some(SessionMode::New), job_id, fire_time);
        assert_eq!(mode, Some(SessionMode::New));
        let expected_run_key = format!(
            "{}:{}",
            job_id.0,
            fire_time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        );
        assert_eq!(
            sid,
            Some(SessionId::for_cron_run(agent, &expected_run_key)),
            "New-mode cron fire must materialize SessionId::for_cron_run(agent, run_key); \
             see CLAUDE.md cron + session_mode note"
        );
    }

    /// Build a minimal valid `CronJob` with an `Every` schedule.
    fn make_job(agent_id: AgentId) -> CronJob {
        CronJob {
            id: CronJobId::new(),
            agent_id,
            name: "test-job".into(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 3600 },
            action: CronAction::SystemEvent {
                text: "ping".into(),
            },
            delivery: CronDelivery::None,
            delivery_targets: Vec::new(),
            peer_id: None,
            session_mode: None,
            created_at: Utc::now(),
            last_run: None,
            next_run: None,
        }
    }

    /// Create a scheduler backed by a temp directory.
    fn make_scheduler(max_total: usize) -> (CronScheduler, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let sched = CronScheduler::new(tmp.path(), max_total);
        (sched, tmp)
    }

    // -- test_add_job_and_list ----------------------------------------------

    #[test]
    fn test_add_job_and_list() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);

        let id = sched.add_job(job, false).unwrap();

        // Should appear in agent list
        let jobs = sched.list_jobs(agent);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, id);
        assert_eq!(jobs[0].name, "test-job");

        // Should appear in global list
        let all = sched.list_all_jobs();
        assert_eq!(all.len(), 1);

        // get_job should return it
        let fetched = sched.get_job(id).unwrap();
        assert_eq!(fetched.agent_id, agent);

        // next_run should have been computed
        assert!(fetched.next_run.is_some());
        assert_eq!(sched.total_jobs(), 1);
    }

    // -- test_remove_job ----------------------------------------------------

    #[test]
    fn test_remove_job() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        let removed = sched.remove_job(id).unwrap();
        assert_eq!(removed.name, "test-job");
        assert_eq!(sched.total_jobs(), 0);

        // Removing again should fail
        assert!(sched.remove_job(id).is_err());
    }

    // -- test_add_job_global_limit ------------------------------------------

    #[test]
    fn test_add_job_global_limit() {
        let (sched, _tmp) = make_scheduler(2);
        let agent = AgentId::new();

        let j1 = make_job(agent);
        let j2 = make_job(agent);
        let j3 = make_job(agent);

        sched.add_job(j1, false).unwrap();
        sched.add_job(j2, false).unwrap();

        // Third should hit global limit
        let err = sched.add_job(j3, false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("limit"),
            "Expected global limit error, got: {msg}"
        );
    }

    // -- test_add_job_per_agent_limit ---------------------------------------

    #[test]
    fn test_add_job_per_agent_limit() {
        // MAX_JOBS_PER_AGENT = 50 in librefang-types
        let (sched, _tmp) = make_scheduler(1000);
        let agent = AgentId::new();

        for i in 0..50 {
            let mut job = make_job(agent);
            job.name = format!("job-{i}");
            sched.add_job(job, false).unwrap();
        }

        // 51st should be rejected by validate()
        let mut overflow = make_job(agent);
        overflow.name = "overflow".into();
        let err = sched.add_job(overflow, false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("50"),
            "Expected per-agent limit error, got: {msg}"
        );
    }

    // -- test_record_success_removes_one_shot --------------------------------

    #[test]
    fn test_record_success_removes_one_shot() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, true).unwrap(); // one_shot = true

        assert_eq!(sched.total_jobs(), 1);

        sched.record_success(id);

        // One-shot job should have been removed
        assert_eq!(sched.total_jobs(), 0);
        assert!(sched.get_job(id).is_none());
    }

    #[test]
    fn test_record_success_keeps_recurring() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap(); // one_shot = false

        sched.record_success(id);

        // Recurring job should still be there
        assert_eq!(sched.total_jobs(), 1);
        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.last_status.as_deref(), Some("ok"));
        assert_eq!(meta.consecutive_errors, 0);
        assert!(meta.job.last_run.is_some());
    }

    // -- test_at_schedule_defaults_to_one_shot (#2808) ----------------------

    #[test]
    fn test_at_schedule_is_forced_one_shot() {
        // add_job with one_shot=false but At schedule → must be treated as one_shot
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let mut job = make_job(agent);
        job.schedule = CronSchedule::At {
            at: Utc::now() + chrono::Duration::seconds(60),
        };

        // Caller explicitly passes one_shot=false — defense-in-depth must override
        let id = sched.add_job(job, false).unwrap();
        assert_eq!(sched.total_jobs(), 1);
        let meta = sched.get_meta(id).unwrap();
        assert!(meta.one_shot, "At schedule must be forced to one_shot=true");

        sched.record_success(id);
        assert_eq!(
            sched.total_jobs(),
            0,
            "At-schedule job must be removed after success"
        );
    }

    #[test]
    fn test_at_schedule_one_shot_true_stays_one_shot() {
        // Explicit one_shot=true on At schedule must also be removed after firing
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let mut job = make_job(agent);
        job.schedule = CronSchedule::At {
            at: Utc::now() + chrono::Duration::seconds(60),
        };

        let id = sched.add_job(job, true).unwrap();
        sched.record_success(id);
        assert_eq!(sched.total_jobs(), 0);
    }

    #[test]
    fn test_at_schedule_removed_on_failure() {
        // one_shot At-schedule job must be removed on first failure too
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let mut job = make_job(agent);
        job.schedule = CronSchedule::At {
            at: Utc::now() + chrono::Duration::seconds(60),
        };

        let id = sched.add_job(job, false).unwrap();
        assert_eq!(sched.total_jobs(), 1);

        sched.record_failure(id, "something went wrong");
        assert_eq!(
            sched.total_jobs(),
            0,
            "At-schedule job must be removed after failure"
        );
    }

    // -- test_record_failure_auto_disable -----------------------------------

    #[test]
    fn test_record_failure_auto_disable() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        // Fail MAX_CONSECUTIVE_ERRORS - 1 times: should still be enabled
        for i in 0..(MAX_CONSECUTIVE_ERRORS - 1) {
            sched.record_failure(id, &format!("error {i}"));
            let meta = sched.get_meta(id).unwrap();
            assert!(
                meta.job.enabled,
                "Job should still be enabled after {} failures",
                i + 1
            );
            assert_eq!(meta.consecutive_errors, i + 1);
        }

        // One more failure should auto-disable
        sched.record_failure(id, "final error");
        let meta = sched.get_meta(id).unwrap();
        assert!(
            !meta.job.enabled,
            "Job should be auto-disabled after {MAX_CONSECUTIVE_ERRORS} failures"
        );
        assert_eq!(meta.consecutive_errors, MAX_CONSECUTIVE_ERRORS);
        assert!(
            meta.last_status.as_ref().unwrap().starts_with("error:"),
            "last_status should record the error"
        );
    }

    // -- test_due_jobs_only_enabled -----------------------------------------

    #[test]
    fn test_due_jobs_only_enabled() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        // Job 1: enabled, next_run in the past
        let mut j1 = make_job(agent);
        j1.name = "enabled-due".into();
        let id1 = sched.add_job(j1, false).unwrap();

        // Job 2: disabled
        let mut j2 = make_job(agent);
        j2.name = "disabled-job".into();
        let id2 = sched.add_job(j2, false).unwrap();
        sched.set_enabled(id2, false).unwrap();

        // Force job 1's next_run to the past
        if let Some(mut meta) = sched.jobs.get_mut(&id1) {
            meta.job.next_run = Some(Utc::now() - Duration::seconds(10));
        }

        // Force job 2's next_run to the past too (but it's disabled)
        if let Some(mut meta) = sched.jobs.get_mut(&id2) {
            meta.job.next_run = Some(Utc::now() - Duration::seconds(10));
        }

        let due = sched.due_jobs();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "enabled-due");
    }

    #[test]
    fn test_due_jobs_future_not_included() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        let job = make_job(agent);
        sched.add_job(job, false).unwrap();

        // The job was just added with next_run = now + 3600s, so it should
        // not be due yet.
        let due = sched.due_jobs();
        assert!(due.is_empty());
    }

    // -- test_set_enabled ---------------------------------------------------

    #[test]
    fn test_set_enabled() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        // Disable
        sched.set_enabled(id, false).unwrap();
        let meta = sched.get_meta(id).unwrap();
        assert!(!meta.job.enabled);

        // Re-enable resets error count
        sched.record_failure(id, "ignored because disabled");
        // Actually the job is disabled so record_failure still updates it.
        // Let's first re-enable to test reset.
        sched.set_enabled(id, true).unwrap();
        let meta = sched.get_meta(id).unwrap();
        assert!(meta.job.enabled);
        assert_eq!(meta.consecutive_errors, 0);
        assert!(meta.job.next_run.is_some());

        // Non-existent ID should fail
        let fake_id = CronJobId::new();
        assert!(sched.set_enabled(fake_id, true).is_err());
    }

    // -- test_persist_and_load ----------------------------------------------

    #[test]
    fn test_persist_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();

        // Create scheduler, add jobs, persist
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let mut j1 = make_job(agent);
            j1.name = "persist-a".into();
            let mut j2 = make_job(agent);
            j2.name = "persist-b".into();

            sched.add_job(j1, false).unwrap();
            sched.add_job(j2, true).unwrap(); // one_shot

            sched.persist().unwrap();
        }

        // Create a new scheduler and load from disk
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let count = sched.load().unwrap();
            assert_eq!(count, 2);
            assert_eq!(sched.total_jobs(), 2);

            let jobs = sched.list_jobs(agent);
            assert_eq!(jobs.len(), 2);

            let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
            assert!(names.contains(&"persist-a"));
            assert!(names.contains(&"persist-b"));

            // Verify one_shot flag was preserved
            let b_id = jobs.iter().find(|j| j.name == "persist-b").unwrap().id;
            let meta = sched.get_meta(b_id).unwrap();
            assert!(meta.one_shot);
        }
    }

    #[test]
    fn test_load_no_file_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let sched = CronScheduler::new(tmp.path(), 100);
        assert_eq!(sched.load().unwrap(), 0);
    }

    // -- compute_next_run ---------------------------------------------------

    #[test]
    fn test_compute_next_run_at() {
        let target = Utc::now() + Duration::hours(2);
        let schedule = CronSchedule::At { at: target };
        let next = compute_next_run(&schedule);
        assert_eq!(next, target);
    }

    /// Regression: #2337 — `compute_next_run_after` for an `at` schedule
    /// in the past must NOT return the past time, otherwise the scheduler
    /// re-fires the job on every tick forever.
    #[test]
    fn test_compute_next_run_after_at_in_past_returns_far_future() {
        let now = Utc::now();
        let past = now - Duration::hours(1);
        let schedule = CronSchedule::At { at: past };
        let next = compute_next_run_after(&schedule, now);
        // Should be far future, not the original past time.
        assert!(next > now + Duration::days(1000));
    }

    #[test]
    fn test_compute_next_run_after_at_in_future_unchanged() {
        let now = Utc::now();
        let future = now + Duration::hours(1);
        let schedule = CronSchedule::At { at: future };
        let next = compute_next_run_after(&schedule, now);
        assert_eq!(next, future);
    }

    #[test]
    fn test_compute_next_run_every() {
        let before = Utc::now();
        let schedule = CronSchedule::Every { every_secs: 300 };
        let next = compute_next_run(&schedule);
        let after = Utc::now();

        // Should be roughly now + 300s
        assert!(next >= before + Duration::seconds(300));
        assert!(next <= after + Duration::seconds(300));
    }

    #[test]
    fn test_compute_next_run_cron_daily() {
        let now = Utc::now();
        let schedule = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let next = compute_next_run(&schedule);

        // Should be within the next 24 hours (next 09:00 UTC)
        assert!(next > now);
        assert!(next <= now + Duration::hours(24));
        assert_eq!(next.format("%M").to_string(), "00");
        assert_eq!(next.format("%H").to_string(), "09");
    }

    #[test]
    fn test_compute_next_run_cron_with_dow() {
        let now = Utc::now();
        let schedule = CronSchedule::Cron {
            expr: "30 14 * * 1-5".into(),
            tz: None,
        };
        let next = compute_next_run(&schedule);

        // Should be within the next 7 days and at 14:30
        assert!(next > now);
        assert!(next <= now + Duration::days(7));
        assert_eq!(next.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn test_compute_next_run_cron_invalid_expr() {
        let now = Utc::now();
        let schedule = CronSchedule::Cron {
            expr: "not a cron".into(),
            tz: None,
        };
        let next = compute_next_run(&schedule);
        // Invalid expression falls back to 1 hour from now
        assert!(next > now + Duration::minutes(59));
        assert!(next <= now + Duration::minutes(61));
    }

    // -- error message truncation in record_failure -------------------------

    #[test]
    fn test_compute_next_run_after_skips_current_second() {
        // A "every 4 hours" cron: next_run should be >= 4 hours from now,
        // not in the same minute (the bug from #55).
        let schedule = CronSchedule::Cron {
            expr: "0 */4 * * *".into(),
            tz: None,
        };
        // Use a fixed time to avoid flaky test (not close to minute boundary)
        let now = chrono::DateTime::parse_from_rfc3339("2024-06-15T12:15:30Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_run_after(&schedule, now);
        // Must be strictly after `now` and at least ~1 hour away
        // (the closest 4-hourly boundary is at least minutes away).
        assert!(next > now, "next_run should be strictly after now");
        let diff = next - now;
        assert!(
            diff.num_minutes() >= 1,
            "Expected next_run at least 1 min away, got {} seconds",
            diff.num_seconds()
        );
    }

    #[test]
    fn test_record_failure_truncates_long_error() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        let long_error = "x".repeat(1000);
        sched.record_failure(id, &long_error);

        let meta = sched.get_meta(id).unwrap();
        let status = meta.last_status.unwrap();
        // "error: " is 7 chars + 256 chars of truncated message = 263 max
        assert!(
            status.len() <= 263,
            "Status should be truncated, got {} chars",
            status.len()
        );
    }

    // -- timezone-aware cron (#473) -----------------------------------------

    #[test]
    fn test_cron_tz_shifts_next_run() {
        // "0 9 * * *" in America/New_York (UTC-5 or UTC-4 depending on DST).
        // The next fire time in UTC should differ from a plain UTC "0 9 * * *".
        let schedule_utc = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let schedule_ny = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("America/New_York".into()),
        };
        let now = Utc::now();
        let next_utc = compute_next_run_after(&schedule_utc, now);
        let next_ny = compute_next_run_after(&schedule_ny, now);

        // The New York schedule should fire at 09:00 Eastern, which is 13:00
        // or 14:00 UTC (depending on DST). In either case, it should NOT
        // equal the plain UTC 09:00 result.
        assert_ne!(
            next_utc, next_ny,
            "Timezone-aware schedule should produce a different UTC time"
        );

        // Verify the New York result, when converted to ET, shows hour 09.
        let ny_tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let next_ny_local = next_ny.with_timezone(&ny_tz);
        assert_eq!(
            next_ny_local.hour(),
            9,
            "Expected 09:00 in America/New_York, got {:02}:{:02}",
            next_ny_local.hour(),
            next_ny_local.minute()
        );
    }

    #[test]
    fn test_cron_tz_none_defaults_to_utc() {
        // tz: None should behave identically to tz: Some("UTC").
        let schedule_none = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: None,
        };
        let schedule_utc = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: Some("UTC".into()),
        };
        let now = Utc::now();
        let next_none = compute_next_run_after(&schedule_none, now);
        let next_utc = compute_next_run_after(&schedule_utc, now);
        assert_eq!(next_none, next_utc);
    }

    #[test]
    fn test_cron_tz_empty_string_defaults_to_utc() {
        let schedule_empty = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: Some(String::new()),
        };
        let schedule_none = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: None,
        };
        let now = Utc::now();
        assert_eq!(
            compute_next_run_after(&schedule_empty, now),
            compute_next_run_after(&schedule_none, now)
        );
    }

    #[test]
    fn test_cron_tz_invalid_falls_back_to_utc() {
        // An invalid timezone string should fall back to UTC, not panic.
        let schedule_bad = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("Not/A_Timezone".into()),
        };
        let schedule_utc = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let now = Utc::now();
        let next_bad = compute_next_run_after(&schedule_bad, now);
        let next_utc = compute_next_run_after(&schedule_utc, now);
        // Invalid tz falls back to UTC computation — same result.
        assert_eq!(next_bad, next_utc);
    }

    #[test]
    fn test_cron_tz_asia_shanghai() {
        // "0 8 * * *" in Asia/Shanghai (UTC+8) should fire at 00:00 UTC.
        let schedule = CronSchedule::Cron {
            expr: "0 8 * * *".into(),
            tz: Some("Asia/Shanghai".into()),
        };
        let now = Utc::now();
        let next = compute_next_run_after(&schedule, now);

        let shanghai_tz: chrono_tz::Tz = "Asia/Shanghai".parse().unwrap();
        let local = next.with_timezone(&shanghai_tz);
        assert_eq!(local.hour(), 8);
        assert_eq!(local.minute(), 0);

        // In UTC, 08:00 Shanghai = 00:00 UTC.
        assert_eq!(next.hour(), 0, "08:00 CST should be 00:00 UTC");
    }

    // -- reassign_agent_jobs (#461) -----------------------------------------

    #[test]
    fn test_reassign_agent_jobs_basic() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let mut j1 = make_job(old_agent);
        j1.name = "cron-a".into();
        let mut j2 = make_job(old_agent);
        j2.name = "cron-b".into();

        let id1 = sched.add_job(j1, false).unwrap();
        let id2 = sched.add_job(j2, false).unwrap();

        let count = sched.reassign_agent_jobs(old_agent, new_agent);
        assert_eq!(count, 2);

        // Both jobs should now belong to the new agent
        let job1 = sched.get_job(id1).unwrap();
        assert_eq!(job1.agent_id, new_agent);
        let job2 = sched.get_job(id2).unwrap();
        assert_eq!(job2.agent_id, new_agent);

        // Old agent should have zero jobs
        assert!(sched.list_jobs(old_agent).is_empty());
        // New agent should have both
        assert_eq!(sched.list_jobs(new_agent).len(), 2);
    }

    #[test]
    fn test_reassign_agent_jobs_does_not_touch_other_agents() {
        let (sched, _tmp) = make_scheduler(100);
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let agent_c = AgentId::new();

        let mut ja = make_job(agent_a);
        ja.name = "job-a".into();
        let mut jb = make_job(agent_b);
        jb.name = "job-b".into();

        let _id_a = sched.add_job(ja, false).unwrap();
        let id_b = sched.add_job(jb, false).unwrap();

        // Reassign agent_a -> agent_c
        let count = sched.reassign_agent_jobs(agent_a, agent_c);
        assert_eq!(count, 1);

        // agent_b's job should be untouched
        let job_b = sched.get_job(id_b).unwrap();
        assert_eq!(job_b.agent_id, agent_b);
    }

    #[test]
    fn test_reassign_agent_jobs_no_match_returns_zero() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let other = AgentId::new();

        let job = make_job(agent);
        sched.add_job(job, false).unwrap();

        // Reassign a non-existent agent
        let count = sched.reassign_agent_jobs(AgentId::new(), other);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_reassign_agent_jobs_resets_consecutive_errors() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let job = make_job(old_agent);
        let id = sched.add_job(job, false).unwrap();

        // Simulate some failures
        sched.record_failure(id, "agent not found");
        sched.record_failure(id, "agent not found");
        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.consecutive_errors, 2);

        // Reassign
        sched.reassign_agent_jobs(old_agent, new_agent);

        // Errors should be reset
        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.consecutive_errors, 0);
        assert_eq!(meta.job.agent_id, new_agent);
    }

    #[test]
    fn test_reassign_agent_jobs_reenables_disabled_stale_jobs() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let job = make_job(old_agent);
        let id = sched.add_job(job, false).unwrap();

        // Simulate enough failures to auto-disable (with "not found" message)
        for _ in 0..MAX_CONSECUTIVE_ERRORS {
            sched.record_failure(id, "No such agent");
        }
        let meta = sched.get_meta(id).unwrap();
        assert!(!meta.job.enabled, "Job should be auto-disabled");

        // Reassign should re-enable it
        sched.reassign_agent_jobs(old_agent, new_agent);

        let meta = sched.get_meta(id).unwrap();
        assert!(
            meta.job.enabled,
            "Job should be re-enabled after reassignment"
        );
        assert_eq!(meta.consecutive_errors, 0);
        assert_eq!(meta.job.agent_id, new_agent);
    }

    #[test]
    fn test_reassign_does_not_reenable_manually_disabled_jobs() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let job = make_job(old_agent);
        let id = sched.add_job(job, false).unwrap();

        // Manually disable the job via set_enabled — this is a deliberate user action.
        sched.set_enabled(id, false).unwrap();
        let meta = sched.get_meta(id).unwrap();
        assert!(!meta.job.enabled, "Job should be disabled");
        assert!(
            !meta.auto_disabled,
            "auto_disabled must be false for manual disables"
        );

        // Reassign should NOT re-enable a manually-disabled job.
        sched.reassign_agent_jobs(old_agent, new_agent);

        let meta = sched.get_meta(id).unwrap();
        assert!(
            !meta.job.enabled,
            "Manually-disabled job must stay disabled after reassignment"
        );
        assert_eq!(meta.job.agent_id, new_agent);
    }

    #[test]
    fn test_reassign_agent_jobs_persists_after_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        // Create scheduler, add job, reassign, persist
        let id = {
            let sched = CronScheduler::new(tmp.path(), 100);
            let job = make_job(old_agent);
            let id = sched.add_job(job, false).unwrap();

            sched.reassign_agent_jobs(old_agent, new_agent);
            sched.persist().unwrap();
            id
        };

        // Load from disk and verify the agent_id was persisted
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            sched.load().unwrap();

            let job = sched.get_job(id).unwrap();
            assert_eq!(job.agent_id, new_agent);
            assert!(sched.list_jobs(old_agent).is_empty());
        }
    }

    // -- remove_agent_jobs (#504) -------------------------------------------

    #[test]
    fn test_remove_agent_jobs_basic() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let other = AgentId::new();

        let mut j1 = make_job(agent);
        j1.name = "job-a".into();
        let mut j2 = make_job(agent);
        j2.name = "job-b".into();
        let mut j3 = make_job(other);
        j3.name = "job-other".into();

        sched.add_job(j1, false).unwrap();
        sched.add_job(j2, false).unwrap();
        let id3 = sched.add_job(j3, false).unwrap();

        assert_eq!(sched.total_jobs(), 3);

        let removed = sched.remove_agent_jobs(agent);
        assert_eq!(removed, 2);
        assert_eq!(sched.total_jobs(), 1);

        // The other agent's job should still exist
        assert!(sched.list_jobs(agent).is_empty());
        assert_eq!(sched.list_jobs(other).len(), 1);
        assert!(sched.get_job(id3).is_some());
    }

    #[test]
    fn test_remove_agent_jobs_no_match() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        let job = make_job(agent);
        sched.add_job(job, false).unwrap();

        // Remove for a non-existent agent
        let removed = sched.remove_agent_jobs(AgentId::new());
        assert_eq!(removed, 0);
        assert_eq!(sched.total_jobs(), 1);
    }

    #[test]
    fn test_remove_agent_jobs_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let other = AgentId::new();

        // Add jobs for two agents, remove one agent's jobs, persist
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let mut j1 = make_job(agent);
            j1.name = "doomed".into();
            let mut j2 = make_job(other);
            j2.name = "survivor".into();

            sched.add_job(j1, false).unwrap();
            sched.add_job(j2, false).unwrap();

            sched.remove_agent_jobs(agent);
            sched.persist().unwrap();
        }

        // Reload and verify
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            sched.load().unwrap();
            assert_eq!(sched.total_jobs(), 1);
            assert!(sched.list_jobs(agent).is_empty());
            assert_eq!(sched.list_jobs(other).len(), 1);
        }
    }

    // -- delivery_targets management ---------------------------------------

    #[test]
    fn set_delivery_targets_replaces_existing_list() {
        use librefang_types::scheduler::CronDeliveryTarget;

        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        // Initially empty.
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());

        // Set two targets. LocalFile paths must be workspace-relative —
        // an absolute `/tmp/x.log` would now fail SSRF/path validation
        // alongside the webhook host check (#4732).
        let targets = vec![
            CronDeliveryTarget::Channel {
                channel_type: "slack".into(),
                recipient: "C123".into(),
                thread_id: None,
                account_id: None,
            },
            CronDeliveryTarget::LocalFile {
                path: "out/x.log".into(),
                append: true,
            },
        ];
        sched.set_delivery_targets(id, targets.clone()).unwrap();
        assert_eq!(sched.get_job(id).unwrap().delivery_targets, targets);

        // Replace with a single target — full replacement, not append.
        let single = vec![CronDeliveryTarget::Webhook {
            url: "https://example.com/hook".into(),
            auth_header: None,
        }];
        sched.set_delivery_targets(id, single.clone()).unwrap();
        assert_eq!(sched.get_job(id).unwrap().delivery_targets, single);

        // Clear with an empty Vec.
        sched.set_delivery_targets(id, Vec::new()).unwrap();
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());
    }

    /// SSRF-prone webhook hosts must be rejected on the
    /// `set_delivery_targets` path, not just on `add_job` (#4732).
    #[test]
    fn set_delivery_targets_rejects_ssrf_webhook() {
        use librefang_types::scheduler::CronDeliveryTarget;

        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        // Hex-form loopback (`0x7f000001` == `127.0.0.1`) — the
        // pre-#4732 prefix-string check missed this entirely.
        let targets = vec![CronDeliveryTarget::Webhook {
            url: "http://0x7f000001/hook".into(),
            auth_header: None,
        }];
        let err = sched
            .set_delivery_targets(id, targets)
            .expect_err("SSRF webhook must be refused");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");

        // Original (empty) target list must remain — failed validation
        // must not partially mutate state.
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());
    }

    /// `update_job` previously skipped delivery / delivery_targets
    /// validation entirely — an attacker could route through PUT to
    /// install an SSRF webhook even when `add_job` would have rejected
    /// the same payload (#4732).
    #[test]
    fn update_job_rejects_ssrf_webhook_in_delivery() {
        use librefang_types::scheduler::CronDelivery;

        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        // make_job() sets `CronDelivery::None`; pin that as the invariant.
        assert!(matches!(
            sched.get_job(id).unwrap().delivery,
            CronDelivery::None
        ));

        let updates = serde_json::json!({
            "delivery": {"kind": "webhook", "url": "http://169.254.169.254/latest/meta-data/"}
        });
        let err = sched
            .update_job(id, &updates)
            .expect_err("link-local metadata IP must be refused");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");

        // State invariant: failed validation must leave delivery untouched.
        assert!(matches!(
            sched.get_job(id).unwrap().delivery,
            CronDelivery::None
        ));
    }

    #[test]
    fn update_job_rejects_ssrf_webhook_in_delivery_targets() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        // Seeded job has no targets; that's the state we expect to keep.
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());

        // Numeric/decimal-form loopback — also a #4732 bypass surface
        // before the WHATWG URL parser was wired in.
        let updates = serde_json::json!({
            "delivery_targets": [{"type": "webhook", "url": "http://2130706433/hook"}]
        });
        let err = sched
            .update_job(id, &updates)
            .expect_err("decimal-form loopback must be refused");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");

        // State invariant: targets must not have been partially written.
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());
    }

    /// Two-phase mutation guarantee (#4739 review): if any field in a
    /// multi-field update fails validation, no field may be applied to
    /// `meta.job`. The pre-#4739 in-place pattern would have committed
    /// `delivery` (a benign public webhook) before failing on
    /// `delivery_targets` (the SSRF target), leaving cron state half
    /// updated and divergent from the 400 the route returned.
    #[test]
    fn update_job_partial_mutation_is_atomic() {
        use librefang_types::scheduler::CronDelivery;

        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        // Valid `delivery` would succeed in isolation. The SSRF-laden
        // `delivery_targets` must roll the whole transaction back.
        let updates = serde_json::json!({
            "delivery": {"kind": "webhook", "url": "https://example.com/hook"},
            "delivery_targets": [
                {"type": "webhook", "url": "http://0x7f000001/hook"}
            ]
        });
        let err = sched
            .update_job(id, &updates)
            .expect_err("update with mixed valid + SSRF must fail");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");

        // `delivery` must NOT have been smuggled in despite passing its
        // own check — atomicity is the property we care about.
        let after = sched.get_job(id).unwrap();
        assert!(
            matches!(after.delivery, CronDelivery::None),
            "delivery must remain None (the seeded value), got {:?}",
            after.delivery
        );
        assert!(after.delivery_targets.is_empty());
    }

    /// candidate-validate-swap (#4739 review followup): non-SSRF shape
    /// rules now also gate the update path. Empty name was previously
    /// accepted on PUT — `add_job` rejected the same payload, so this
    /// closes a parallel bypass surface to the SSRF webhook one.
    #[test]
    fn update_job_rejects_empty_name() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        let original_name = sched.get_job(id).unwrap().name;

        let updates = serde_json::json!({ "name": "" });
        let err = sched
            .update_job(id, &updates)
            .expect_err("empty name must be refused");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");
        // State invariant: rejected payload must not partially mutate.
        assert_eq!(sched.get_job(id).unwrap().name, original_name);
    }

    /// `validate(0)` on the candidate also catches over-long names.
    #[test]
    fn update_job_rejects_oversized_name() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        // librefang_types::scheduler::MAX_NAME_LEN is 128.
        let long = "x".repeat(200);
        let updates = serde_json::json!({ "name": long });
        let err = sched
            .update_job(id, &updates)
            .expect_err("oversized name must be refused");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");
    }

    /// Same atomicity guarantee in the other direction: failure in the
    /// `delivery` phase must not leak the would-be `delivery_targets` mutation.
    #[test]
    fn update_job_partial_mutation_targets_not_smuggled_on_delivery_failure() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        // Seeded targets are empty; that's the invariant.
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());

        let updates = serde_json::json!({
            "delivery": {"kind": "webhook", "url": "http://10.0.0.1/hook"},
            "delivery_targets": [{"type": "webhook", "url": "https://example.com/hook"}]
        });
        let err = sched
            .update_job(id, &updates)
            .expect_err("RFC 1918 delivery must reject");
        assert!(matches!(err, LibreFangError::InvalidInput(_)), "{err:?}");

        // Atomicity: the would-be valid targets must not survive when
        // `delivery` (which the parser sees first) gets rejected.
        assert!(sched.get_job(id).unwrap().delivery_targets.is_empty());
    }

    #[test]
    fn set_delivery_targets_unknown_id_returns_error() {
        let (sched, _tmp) = make_scheduler(100);
        let unknown = CronJobId::new();
        let res = sched.set_delivery_targets(unknown, Vec::new());
        assert!(res.is_err());
    }

    #[test]
    fn update_job_patches_delivery_targets() {
        use librefang_types::scheduler::CronDeliveryTarget;

        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        let updates = serde_json::json!({
            "delivery_targets": [
                {"type": "webhook", "url": "https://example.com/hook"},
                {"type": "local_file", "path": "out/y.log"},
            ]
        });
        let updated = sched.update_job(id, &updates).unwrap();
        assert_eq!(updated.delivery_targets.len(), 2);
        assert!(matches!(
            &updated.delivery_targets[0],
            CronDeliveryTarget::Webhook { url, .. } if url == "https://example.com/hook"
        ));
        assert!(matches!(
            &updated.delivery_targets[1],
            CronDeliveryTarget::LocalFile { append, .. } if !*append
        ));
    }

    #[test]
    fn update_job_invalid_delivery_targets_rejected() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        let updates = serde_json::json!({
            "delivery_targets": [{"type": "bogus_target_type"}]
        });
        let res = sched.update_job(id, &updates);
        assert!(res.is_err());
    }

    /// Regression for #3648: two concurrent persists must never share a
    /// staging path, otherwise the second `O_CREAT|O_TRUNC` clobbers the
    /// first writer's tmp file mid-flight and rename produces a torn
    /// JSON.  The pid+seq+nanos suffix ensures uniqueness within a
    /// process and across daemons sharing a home_dir.
    #[test]
    fn persist_tmp_paths_are_unique_under_concurrency() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let sched = std::sync::Arc::new(CronScheduler::new(tmp.path(), 100));
        sched.add_job(make_job(agent), false).unwrap();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = sched.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..20 {
                    s.persist().unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // Final file must parse cleanly — no torn JSON.
        let path = tmp.path().join("data").join("cron_jobs.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        let _: Vec<JobMeta> =
            serde_json::from_str(&raw).expect("torn JSON after concurrent persist");

        // No leftover .tmp staging files in the parent dir.
        let parent = path.parent().unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no .tmp.* staging files should remain after concurrent persist; found: {leftovers:?}"
        );
    }

    /// Regression for #3515: when the persist destination is not writable
    /// (here: `data/` exists as a regular file instead of a directory so
    /// `create_dir_all` fails), `persist()` MUST surface the I/O error
    /// rather than swallow it. The route layer relies on this `Err` to
    /// translate into a 500 response, otherwise UI-driven cron updates
    /// silently revert on the next daemon restart.
    #[test]
    fn persist_returns_error_when_data_dir_unwritable() {
        let tmp = tempfile::tempdir().unwrap();
        // Pre-create `data` as a regular file so `create_dir_all` fails on
        // the persist path's parent. This is portable across platforms
        // (no need for chmod/permission tricks that don't work on
        // Windows or as root in CI).
        std::fs::write(tmp.path().join("data"), b"not a directory").unwrap();

        let sched = CronScheduler::new(tmp.path(), 100);
        let agent = AgentId::new();
        sched.add_job(make_job(agent), false).unwrap();

        let result = sched.persist();
        assert!(
            result.is_err(),
            "persist() must surface I/O failure (got Ok). Without this signal \
             the API layer cannot return 500 and silently reverts schedules \
             on restart (#3515)."
        );
    }

    /// Regression for #3515: an `update_job` that successfully patches the
    /// in-memory state must not mask a subsequent `persist()` failure.
    /// The two operations are independent — `update_job` returns Ok, then
    /// `persist()` returns Err on a non-writable path — and the route
    /// handler must observe both results separately.
    #[test]
    fn update_job_then_persist_failure_is_observable() {
        let tmp = tempfile::tempdir().unwrap();
        let sched = CronScheduler::new(tmp.path(), 100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        // First persist succeeds (data/ does not yet exist as a file).
        sched.persist().unwrap();

        // Now sabotage the parent dir so future persists fail. We replace
        // the existing data/ directory with a regular file of the same
        // name. `create_dir_all` on the parent will then fail because
        // a non-directory already occupies that path.
        let data_dir = tmp.path().join("data");
        std::fs::remove_dir_all(&data_dir).unwrap();
        std::fs::write(&data_dir, b"sabotage").unwrap();

        // In-memory update still succeeds (it doesn't touch disk).
        let updates = serde_json::json!({ "name": "renamed-after-sabotage" });
        let updated = sched
            .update_job(id, &updates)
            .expect("update_job must succeed in-memory");
        assert_eq!(updated.name, "renamed-after-sabotage");

        // But persist now fails — and the route handler must see this Err
        // so it can return 500 instead of pretending the change was saved.
        let persist_result = sched.persist();
        assert!(
            persist_result.is_err(),
            "persist() after in-memory update must fail loudly when disk \
             write is impossible (#3515)"
        );
    }

    #[test]
    fn delivery_targets_persist_across_reload() {
        use librefang_types::scheduler::CronDeliveryTarget;

        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let job_id = {
            let sched = CronScheduler::new(tmp.path(), 100);
            let id = sched.add_job(make_job(agent), false).unwrap();
            sched
                .set_delivery_targets(
                    id,
                    vec![CronDeliveryTarget::Email {
                        to: "alice@example.com".into(),
                        subject_template: Some("Cron: {job}".into()),
                    }],
                )
                .unwrap();
            sched.persist().unwrap();
            id
        };
        let sched = CronScheduler::new(tmp.path(), 100);
        sched.load().unwrap();
        let job = sched.get_job(job_id).unwrap();
        assert_eq!(job.delivery_targets.len(), 1);
        assert!(matches!(
            &job.delivery_targets[0],
            CronDeliveryTarget::Email { to, .. } if to == "alice@example.com"
        ));
    }
}
