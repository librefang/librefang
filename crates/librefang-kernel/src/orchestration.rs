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
        match fs::read_to_string(&path).await {
            Ok(data) => {
                let cp: WorkflowCheckpoint = serde_json::from_str(&data)
                    .map_err(|e| format!("Failed to parse checkpoint: {e}"))?;
                Ok(Some(cp))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Failed to read checkpoint: {e}")),
        }
    }

    /// List all persisted checkpoints (for recovery on daemon startup).
    pub async fn list_all(&self) -> Result<Vec<WorkflowCheckpoint>, String> {
        let mut entries = match fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("Failed to read checkpoint dir: {e}")),
        };

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
        match fs::remove_file(&path).await {
            Ok(()) => {
                debug!(run_id = %run_id, "Checkpoint removed");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Failed to remove checkpoint: {e}")),
        }
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

/// Cache compiled `MatchesRegex` patterns so quality gates evaluated on every
/// workflow step don't pay the per-call `Regex::new` cost (#3491).
static QUALITY_REGEX_CACHE: std::sync::OnceLock<
    std::sync::Mutex<HashMap<String, regex_lite::Regex>>,
> = std::sync::OnceLock::new();

fn matches_quality_regex(pattern: &str, output: &str) -> bool {
    let cache = QUALITY_REGEX_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(pattern.to_string()).or_insert_with(|| {
        regex_lite::Regex::new(pattern).unwrap_or_else(|_| {
            // Never-match sentinel: an invalid pattern fails the gate forever.
            // `regex_lite` rejects look-around (`(?!x)x`), so use the negated
            // class containing both \s and \S — that's the universe of chars,
            // negated to the empty set — supported by regex_lite syntax.
            regex_lite::Regex::new(r"[^\s\S]").expect("static never-match regex compiles")
        })
    });
    entry.is_match(output)
}

impl QualityCheck {
    /// Returns `true` when the check passes for the given output.
    pub fn passes(&self, output: &str) -> bool {
        match self {
            QualityCheck::Contains(s) => output.to_lowercase().contains(&s.to_lowercase()),
            QualityCheck::NotContains(s) => !output.to_lowercase().contains(&s.to_lowercase()),
            QualityCheck::MinLength(n) => output.len() >= *n,
            QualityCheck::MaxLength(n) => output.len() <= *n,
            QualityCheck::MatchesRegex(pattern) => matches_quality_regex(pattern, output),
            QualityCheck::All(checks) => checks.iter().all(|c| c.passes(output)),
            QualityCheck::Any(checks) => checks.iter().any(|c| c.passes(output)),
        }
    }
}

// ---------------------------------------------------------------------------
// Recovery — exponential backoff with jitter
// ---------------------------------------------------------------------------

/// Hard ceiling (seconds) applied when a `RetryPolicy` has `max_backoff ==
/// ZERO` (interpreted as "no operator cap"). Bounds the scaled wait so an
/// unbounded multiplier can never overflow `Duration::mul_f64`. One hour is
/// far longer than any sane retry wait while staying well inside Duration's
/// range. #5136.
const MAX_BACKOFF_FALLBACK_SECS: f64 = 3600.0;

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
    /// Sanitize `backoff_multiplier` for use in [`Self::delay_for_attempt`].
    ///
    /// `f64::powi` of a negative base with an odd exponent yields a negative
    /// number, and `Duration::mul_f64(<0.0)` panics. NaN propagates through
    /// `powi` and also panics in `mul_f64`. A multiplier below `1.0` would
    /// shrink the backoff every attempt (anti-backoff). Clamp to the sane
    /// `[1.0, MAX]` range and treat any non-finite value as the default `2.0`
    /// so a hand-edited or deserialized config can never panic the retry hot
    /// path. Returns the effective multiplier.
    fn effective_multiplier(&self) -> f64 {
        let m = self.backoff_multiplier;
        if !m.is_finite() {
            // NaN / ±inf — fall back to the documented default.
            return 2.0;
        }
        // Clamp into [1.0, MAX]: < 1.0 (incl. negatives) would invert the
        // backoff curve; MAX keeps `powi` from overflowing to +inf early.
        m.clamp(1.0, f64::MAX)
    }

    /// Reject a structurally invalid policy at construction / config-load
    /// time, before any retry actually runs.
    ///
    /// [`Self::delay_for_attempt`] is internally panic-proof (it clamps the
    /// multiplier and floors the jitter window), but a NaN or negative
    /// multiplier in a deserialized config is almost always an operator
    /// mistake we want surfaced loudly rather than silently coerced to the
    /// default. Callers that load a `RetryPolicy` from user-supplied TOML/JSON
    /// should call this and propagate the error.
    pub fn validate(&self) -> Result<(), String> {
        if self.backoff_multiplier.is_nan() {
            return Err("retry backoff_multiplier is NaN".to_string());
        }
        if !self.backoff_multiplier.is_finite() {
            return Err(format!(
                "retry backoff_multiplier must be finite, got {}",
                self.backoff_multiplier
            ));
        }
        if self.backoff_multiplier < 1.0 {
            return Err(format!(
                "retry backoff_multiplier must be >= 1.0, got {}",
                self.backoff_multiplier
            ));
        }
        Ok(())
    }

    /// Compute the wait duration for a given attempt (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        // Cap exponent to 63 — beyond that any multiplier >= 2 overflows f64
        // or produces values so large that Duration::mul_f64 panics.
        let exp = attempt.min(63) as i32;
        // `effective_multiplier()` guarantees a finite value in [1.0, MAX],
        // so `mul_f64` below can never see a negative or NaN scale factor.
        let multiplier = self.effective_multiplier().powi(exp);
        // A zero `max_backoff` means "no cap configured" — NOT "cap every
        // wait to zero". The original code divided `f64::MAX / 0.0` (→ +inf)
        // and then `.min(ZERO)`, collapsing every retrier's wait to 0 so they
        // all fired in lockstep. Treat ZERO (and any non-positive cap) as
        // uncapped: scale freely and skip the `.min(max_backoff)` clamp.
        let max_secs = self.max_backoff.as_secs_f64();
        let capped = if max_secs > 0.0 {
            let overflow_guard = f64::MAX / max_secs;
            if multiplier.is_finite() && multiplier < overflow_guard {
                self.initial_backoff
                    .mul_f64(multiplier)
                    .min(self.max_backoff)
            } else {
                self.max_backoff
            }
        } else if multiplier.is_finite() {
            // Uncapped: clamp the scaled value to MAX_BACKOFF_FALLBACK so an
            // unbounded multiplier can't overflow Duration::mul_f64.
            let scaled = self.initial_backoff.as_secs_f64() * multiplier;
            if scaled.is_finite() && scaled < MAX_BACKOFF_FALLBACK_SECS {
                self.initial_backoff.mul_f64(multiplier)
            } else {
                Duration::from_secs_f64(MAX_BACKOFF_FALLBACK_SECS)
            }
        } else {
            Duration::from_secs_f64(MAX_BACKOFF_FALLBACK_SECS)
        };
        if self.jitter {
            // Full jitter: random duration in [0, capped) instead of
            // [capped, 2*capped). Floor the jitter window at 1ms: when
            // `capped < 1ms` the millisecond truncation made `capped_ms == 0`,
            // so `rand % 1` was always 0 and every retrier fired in lockstep
            // (thundering herd). A 1ms floor restores per-retrier dispersion.
            let capped_ms = (capped.as_millis() as u64).max(1);
            Duration::from_millis(rand::random::<u64>() % capped_ms)
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

    #[test]
    fn test_delay_for_attempt_large_attempt_no_panic() {
        let policy = RetryPolicy {
            max_retries: 1000,
            initial_backoff: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(30),
            jitter: false,
        };
        // Must not panic even with very large attempt values.
        let d = policy.delay_for_attempt(1000);
        assert_eq!(d, Duration::from_secs(30));
        let d = policy.delay_for_attempt(u32::MAX);
        assert_eq!(d, Duration::from_secs(30));
    }

    #[test]
    fn test_delay_for_attempt_with_jitter() {
        let policy = RetryPolicy {
            max_retries: 5,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(10),
            jitter: true,
        };
        // With full jitter, delay should be in [0, capped).
        for attempt in 0..5 {
            let d = policy.delay_for_attempt(attempt);
            assert!(
                d < Duration::from_secs(10),
                "jitter delay {d:?} should be < max_backoff"
            );
        }
    }

    #[test]
    fn test_quality_check_invalid_regex() {
        let check = QualityCheck::MatchesRegex("[invalid".into());
        // Invalid regex should return false, not panic.
        assert!(!check.passes("anything"));
    }

    #[test]
    fn test_quality_check_empty_all_and_any() {
        // All([]) is vacuously true.
        assert!(QualityCheck::All(vec![]).passes("anything"));
        // Any([]) has no passing check, so false.
        assert!(!QualityCheck::Any(vec![]).passes("anything"));
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

    // -- #5136: retry-backoff arithmetic robustness -------------------------

    #[test]
    fn test_negative_multiplier_does_not_panic() {
        // Odd exponent with a negative multiplier previously produced
        // `Duration::mul_f64(<0.0)` which panics. Drive several attempts
        // (mix of odd/even exponents) and assert no panic + bounded output.
        let policy = RetryPolicy {
            max_retries: 5,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: -2.0,
            max_backoff: Duration::from_secs(30),
            jitter: false,
        };
        for attempt in 0..6 {
            let d = policy.delay_for_attempt(attempt);
            assert!(
                d <= Duration::from_secs(30),
                "attempt {attempt}: {d:?} must stay within max_backoff"
            );
        }
    }

    #[test]
    fn test_nan_multiplier_does_not_panic() {
        let policy = RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: f64::NAN,
            max_backoff: Duration::from_secs(30),
            jitter: false,
        };
        // NaN previously propagated into mul_f64 and panicked. The effective
        // multiplier falls back to the default 2.0, so attempt 0 == base.
        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(2));
    }

    #[test]
    fn test_zero_max_backoff_does_not_collapse_to_zero() {
        // max_backoff == ZERO made the old `f64::MAX / 0.0` divisor +inf,
        // so the finite branch was always taken with `.min(ZERO)` → every
        // wait collapsed to zero (all retriers in lockstep). With the fix,
        // a zero cap means "no cap": the wait grows with the multiplier.
        let policy = RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::ZERO,
            jitter: false,
        };
        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(4));
    }

    #[test]
    fn test_sub_millisecond_cap_keeps_jitter_window() {
        // capped < 1ms previously truncated `capped_ms` to 0, so
        // `rand % 1` was always 0 → zero jitter, thundering herd. The 1ms
        // floor restores a non-degenerate window: not every sample is
        // identical across many draws.
        let policy = RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_micros(100),
            backoff_multiplier: 1.0,
            max_backoff: Duration::from_micros(100),
            jitter: true,
        };
        // All samples must be in [0, 1ms) and not panic.
        for _ in 0..50 {
            let d = policy.delay_for_attempt(0);
            assert!(d < Duration::from_millis(1), "jitter {d:?} out of window");
        }
    }

    #[test]
    fn test_validate_rejects_nan_and_negative() {
        let nan = RetryPolicy {
            backoff_multiplier: f64::NAN,
            ..RetryPolicy::default()
        };
        assert!(nan.validate().is_err());

        let neg = RetryPolicy {
            backoff_multiplier: -1.0,
            ..RetryPolicy::default()
        };
        assert!(neg.validate().is_err());

        let inf = RetryPolicy {
            backoff_multiplier: f64::INFINITY,
            ..RetryPolicy::default()
        };
        assert!(inf.validate().is_err());

        assert!(RetryPolicy::default().validate().is_ok());
    }
}
