//! Multi-agent orchestration primitives — recovery snapshots, quality gates,
//! and coordination utilities for long-running workflow execution.
//!
//! This module extends the workflow engine with:
//! - **Checkpoint snapshots** — persist workflow state so runs can resume after
//!   a daemon restart or crash.
//! - **Quality gates** — validate agent output against user-defined criteria
//!   before the workflow proceeds.
//! - **Recovery** — retry failed steps with exponential backoff and jitter.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::workflow::{StepResult, WorkflowId, WorkflowRunId, WorkflowRunState};

// ---------------------------------------------------------------------------
// Checkpoint / Snapshot
// ---------------------------------------------------------------------------

/// A serializable snapshot of an in-progress workflow run.
///
/// Checkpoints are written to disk after every step so that the run can be
/// resumed from the last successful step if the daemon restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    /// The run this checkpoint belongs to.
    pub run_id: WorkflowRunId,
    /// The workflow definition id.
    pub workflow_id: WorkflowId,
    /// Workflow name (for display).
    pub workflow_name: String,
    /// Original input to the workflow.
    pub input: String,
    /// Index of the *next* step to execute (i.e. all steps before this have
    /// completed successfully).
    pub next_step_index: usize,
    /// The current input value that would be fed into `next_step_index`.
    pub current_input: String,
    /// Variable store accumulated so far.
    pub variables: HashMap<String, String>,
    /// Results from steps that have already completed.
    pub step_results: Vec<StepResult>,
    /// All collected outputs (used by fan-out / collect).
    pub all_outputs: Vec<String>,
    /// Run state at checkpoint time.
    pub state: WorkflowRunState,
    /// When the checkpoint was taken.
    pub created_at: DateTime<Utc>,
}

/// Manages checkpoint persistence on the local filesystem.
///
/// Checkpoints are stored as JSON files under a configurable directory,
/// keyed by `WorkflowRunId`.
#[derive(Debug, Clone)]
pub struct CheckpointStore {
    dir: PathBuf,
}

impl CheckpointStore {
    /// Create a new store that writes checkpoints to `dir`.
    ///
    /// The directory is created lazily on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Return the default checkpoint directory under the LibreFang data dir.
    pub fn default_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("librefang")
            .join("checkpoints")
    }

    fn path_for(&self, run_id: WorkflowRunId) -> PathBuf {
        self.dir.join(format!("{}.json", run_id))
    }

    /// Persist a checkpoint to disk.
    pub async fn save(&self, checkpoint: &WorkflowCheckpoint) -> Result<(), String> {
        fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| format!("Failed to create checkpoint dir: {e}"))?;

        let path = self.path_for(checkpoint.run_id);
        let json = serde_json::to_string_pretty(checkpoint)
            .map_err(|e| format!("Failed to serialize checkpoint: {e}"))?;

        // Write atomically via a temp file to avoid partial writes.
        let tmp = self.dir.join(format!(".tmp-{}", Uuid::new_v4()));
        fs::write(&tmp, json.as_bytes())
            .await
            .map_err(|e| format!("Failed to write checkpoint temp file: {e}"))?;
        fs::rename(&tmp, &path)
            .await
            .map_err(|e| format!("Failed to rename checkpoint file: {e}"))?;

        debug!(run_id = %checkpoint.run_id, step = checkpoint.next_step_index, "Checkpoint saved");
        Ok(())
    }

    /// Load a checkpoint for a specific run.
    pub async fn load(&self, run_id: WorkflowRunId) -> Result<Option<WorkflowCheckpoint>, String> {
        let path = self.path_for(run_id);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read checkpoint: {e}"))?;
        let cp: WorkflowCheckpoint =
            serde_json::from_str(&data).map_err(|e| format!("Failed to parse checkpoint: {e}"))?;
        Ok(Some(cp))
    }

    /// List all persisted checkpoints (for recovery on daemon startup).
    pub async fn list_all(&self) -> Result<Vec<WorkflowCheckpoint>, String> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&self.dir)
            .await
            .map_err(|e| format!("Failed to read checkpoint dir: {e}"))?;

        let mut checkpoints = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("Failed to iterate checkpoint dir: {e}"))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                match fs::read_to_string(&path).await {
                    Ok(data) => match serde_json::from_str::<WorkflowCheckpoint>(&data) {
                        Ok(cp) => checkpoints.push(cp),
                        Err(e) => {
                            warn!(path = %path.display(), "Skipping corrupt checkpoint: {e}");
                        }
                    },
                    Err(e) => {
                        warn!(path = %path.display(), "Cannot read checkpoint file: {e}");
                    }
                }
            }
        }
        Ok(checkpoints)
    }

    /// Remove a checkpoint once the run has finished (completed or permanently failed).
    pub async fn remove(&self, run_id: WorkflowRunId) -> Result<(), String> {
        let path = self.path_for(run_id);
        if path.exists() {
            fs::remove_file(&path)
                .await
                .map_err(|e| format!("Failed to remove checkpoint: {e}"))?;
            debug!(run_id = %run_id, "Checkpoint removed");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Quality Gate
// ---------------------------------------------------------------------------

/// A quality gate evaluates the output of a workflow step and decides whether
/// the workflow should proceed, retry the step, or abort.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityGate {
    /// Human-readable name for logging.
    pub name: String,
    /// The check to perform on the step output.
    pub check: QualityCheck,
    /// What to do when the check fails.
    pub on_failure: QualityAction,
}

/// A single quality check condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityCheck {
    /// Output must contain this substring (case-insensitive).
    Contains(String),
    /// Output must NOT contain this substring (case-insensitive).
    NotContains(String),
    /// Output length must be at least this many characters.
    MinLength(usize),
    /// Output length must be at most this many characters.
    MaxLength(usize),
    /// Output must match this regex pattern.
    MatchesRegex(String),
    /// All inner checks must pass.
    All(Vec<QualityCheck>),
    /// At least one inner check must pass.
    Any(Vec<QualityCheck>),
}

/// What happens when a quality gate fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityAction {
    /// Abort the workflow with an error.
    Abort,
    /// Retry the step (up to the step's own retry limit).
    Retry,
    /// Log a warning but proceed.
    Warn,
}

impl QualityGate {
    /// Evaluate the gate against step output. Returns `Ok(())` if the gate
    /// passes, or `Err(reason)` if it fails.
    pub fn evaluate(&self, output: &str) -> Result<(), String> {
        if self.check.passes(output) {
            Ok(())
        } else {
            Err(format!("Quality gate '{}' failed", self.name))
        }
    }
}

impl QualityCheck {
    /// Returns `true` when the check passes for the given output.
    pub fn passes(&self, output: &str) -> bool {
        match self {
            QualityCheck::Contains(s) => output.to_lowercase().contains(&s.to_lowercase()),
            QualityCheck::NotContains(s) => !output.to_lowercase().contains(&s.to_lowercase()),
            QualityCheck::MinLength(n) => output.len() >= *n,
            QualityCheck::MaxLength(n) => output.len() <= *n,
            QualityCheck::MatchesRegex(pattern) => regex_lite::Regex::new(pattern)
                .map(|re| re.is_match(output))
                .unwrap_or(false),
            QualityCheck::All(checks) => checks.iter().all(|c| c.passes(output)),
            QualityCheck::Any(checks) => checks.iter().any(|c| c.passes(output)),
        }
    }
}

// ---------------------------------------------------------------------------
// Recovery — exponential backoff with jitter
// ---------------------------------------------------------------------------

/// Configuration for exponential-backoff retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retries).
    pub max_retries: u32,
    /// Initial backoff duration.
    #[serde(with = "humantime_millis")]
    pub initial_backoff: Duration,
    /// Multiplier applied after each attempt (e.g. 2.0 for doubling).
    pub backoff_multiplier: f64,
    /// Maximum backoff duration cap.
    #[serde(with = "humantime_millis")]
    pub max_backoff: Duration,
    /// Whether to add random jitter (0..backoff) to each wait.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(30),
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Compute the wait duration for a given attempt (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base = self
            .initial_backoff
            .mul_f64(self.backoff_multiplier.powi(attempt as i32));
        let capped = base.min(self.max_backoff);
        if self.jitter {
            let jitter_ms = rand::random::<u64>() % (capped.as_millis() as u64).max(1);
            capped + Duration::from_millis(jitter_ms)
        } else {
            capped
        }
    }

    /// Execute an async closure with retries according to this policy.
    ///
    /// Returns the first `Ok` value, or the last `Err` if all attempts fail.
    pub async fn execute<F, Fut, T, E>(&self, mut f: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut last_err: Option<E> = None;
        for attempt in 0..=self.max_retries {
            match f().await {
                Ok(val) => return Ok(val),
                Err(e) => {
                    if attempt < self.max_retries {
                        let delay = self.delay_for_attempt(attempt);
                        warn!(
                            attempt = attempt + 1,
                            max = self.max_retries,
                            delay_ms = delay.as_millis() as u64,
                            error = %e,
                            "Retry policy: attempt failed, backing off"
                        );
                        tokio::time::sleep(delay).await;
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap())
    }
}

/// Serde helper to encode `Duration` as milliseconds.
mod humantime_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(dur: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(dur.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- QualityCheck -------------------------------------------------------

    #[test]
    fn test_quality_check_contains() {
        let check = QualityCheck::Contains("success".into());
        assert!(check.passes("Operation SUCCESS completed"));
        assert!(!check.passes("something failed"));
    }

    #[test]
    fn test_quality_check_not_contains() {
        let check = QualityCheck::NotContains("error".into());
        assert!(check.passes("all good"));
        assert!(!check.passes("an ERROR occurred"));
    }

    #[test]
    fn test_quality_check_min_length() {
        let check = QualityCheck::MinLength(10);
        assert!(check.passes("hello world!"));
        assert!(!check.passes("short"));
    }

    #[test]
    fn test_quality_check_max_length() {
        let check = QualityCheck::MaxLength(5);
        assert!(check.passes("hi"));
        assert!(!check.passes("too long string"));
    }

    #[test]
    fn test_quality_check_regex() {
        let check = QualityCheck::MatchesRegex(r"\d{3}-\d{4}".into());
        assert!(check.passes("Call 555-1234 now"));
        assert!(!check.passes("no numbers here"));
    }

    #[test]
    fn test_quality_check_all() {
        let check = QualityCheck::All(vec![
            QualityCheck::Contains("ok".into()),
            QualityCheck::MinLength(5),
        ]);
        assert!(check.passes("everything is ok"));
        assert!(!check.passes("ok")); // too short
        assert!(!check.passes("everything is fine")); // no "ok"
    }

    #[test]
    fn test_quality_check_any() {
        let check = QualityCheck::Any(vec![
            QualityCheck::Contains("yes".into()),
            QualityCheck::Contains("ok".into()),
        ]);
        assert!(check.passes("yes please"));
        assert!(check.passes("ok then"));
        assert!(!check.passes("nope"));
    }

    #[test]
    fn test_quality_gate_evaluate() {
        let gate = QualityGate {
            name: "has-result".into(),
            check: QualityCheck::Contains("result".into()),
            on_failure: QualityAction::Abort,
        };
        assert!(gate.evaluate("Here is the result").is_ok());
        assert!(gate.evaluate("no match").is_err());
    }

    // -- QualityCheck serde round-trip --------------------------------------

    #[test]
    fn test_quality_check_serde_round_trip() {
        let check = QualityCheck::All(vec![
            QualityCheck::Contains("ok".into()),
            QualityCheck::NotContains("error".into()),
            QualityCheck::MinLength(10),
        ]);
        let json = serde_json::to_string(&check).unwrap();
        let parsed: QualityCheck = serde_json::from_str(&json).unwrap();
        assert!(parsed.passes("everything is ok here"));
        assert!(!parsed.passes("ok")); // too short
    }

    // -- RetryPolicy --------------------------------------------------------

    #[test]
    fn test_retry_policy_delay_increases() {
        let policy = RetryPolicy {
            max_retries: 5,
            initial_backoff: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(10),
            jitter: false,
        };
        let d0 = policy.delay_for_attempt(0);
        let d1 = policy.delay_for_attempt(1);
        let d2 = policy.delay_for_attempt(2);
        assert_eq!(d0, Duration::from_millis(100));
        assert_eq!(d1, Duration::from_millis(200));
        assert_eq!(d2, Duration::from_millis(400));
    }

    #[test]
    fn test_retry_policy_caps_at_max() {
        let policy = RetryPolicy {
            max_retries: 10,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 10.0,
            max_backoff: Duration::from_secs(5),
            jitter: false,
        };
        // 1 * 10^3 = 1000s, but capped at 5s
        let d = policy.delay_for_attempt(3);
        assert_eq!(d, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_retry_policy_succeeds_on_third_attempt() {
        let policy = RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_millis(1),
            backoff_multiplier: 1.0,
            max_backoff: Duration::from_millis(1),
            jitter: false,
        };

        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        let result: Result<&str, String> = policy
            .execute(|| {
                let c = c.clone();
                async move {
                    let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n < 2 {
                        Err(format!("fail #{n}"))
                    } else {
                        Ok("done")
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), "done");
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_policy_exhausted() {
        let policy = RetryPolicy {
            max_retries: 1,
            initial_backoff: Duration::from_millis(1),
            backoff_multiplier: 1.0,
            max_backoff: Duration::from_millis(1),
            jitter: false,
        };

        let result: Result<(), String> = policy
            .execute(|| async { Err::<(), String>("always fails".into()) })
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "always fails");
    }

    // -- RetryPolicy serde round-trip ---------------------------------------

    #[test]
    fn test_retry_policy_serde_round_trip() {
        let policy = RetryPolicy::default();
        let json = serde_json::to_string(&policy).unwrap();
        let parsed: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_retries, 3);
        assert_eq!(parsed.initial_backoff, Duration::from_secs(1));
    }

    // -- CheckpointStore ----------------------------------------------------

    #[tokio::test]
    async fn test_checkpoint_save_load_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CheckpointStore::new(tmp.path());

        let run_id = WorkflowRunId::new();
        let cp = WorkflowCheckpoint {
            run_id,
            workflow_id: WorkflowId::new(),
            workflow_name: "test-wf".into(),
            input: "hello".into(),
            next_step_index: 2,
            current_input: "step2-input".into(),
            variables: HashMap::new(),
            step_results: vec![],
            all_outputs: vec!["out1".into()],
            state: WorkflowRunState::Running,
            created_at: Utc::now(),
        };

        // Save
        store.save(&cp).await.unwrap();

        // Load
        let loaded = store.load(run_id).await.unwrap().unwrap();
        assert_eq!(loaded.run_id, run_id);
        assert_eq!(loaded.next_step_index, 2);
        assert_eq!(loaded.current_input, "step2-input");

        // List
        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 1);

        // Remove
        store.remove(run_id).await.unwrap();
        assert!(store.load(run_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_load_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CheckpointStore::new(tmp.path());
        let result = store.load(WorkflowRunId::new()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_list_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CheckpointStore::new(tmp.path().join("nonexistent"));
        let all = store.list_all().await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_checkpoint_multiple_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CheckpointStore::new(tmp.path());

        for i in 0..3 {
            let cp = WorkflowCheckpoint {
                run_id: WorkflowRunId::new(),
                workflow_id: WorkflowId::new(),
                workflow_name: format!("wf-{i}"),
                input: "in".into(),
                next_step_index: i,
                current_input: format!("input-{i}"),
                variables: HashMap::new(),
                step_results: vec![],
                all_outputs: vec![],
                state: WorkflowRunState::Running,
                created_at: Utc::now(),
            };
            store.save(&cp).await.unwrap();
        }

        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 3);
    }
}
