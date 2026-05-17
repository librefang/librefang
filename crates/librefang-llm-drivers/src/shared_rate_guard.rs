//! Cross-process file-backed rate-limit guard.
//!
//! Complements the in-process [`crate::rate_limit_tracker::RateLimitBucket`]
//! by persisting provider rate-limit lockouts to disk so that:
//!
//! 1. **Daemon restart** does not forget a 429: a provider that said
//!    "no more requests for 1 hour" still won't be hit on next boot.
//! 2. **Multiple processes** sharing one API key (daemon + `librefang agent
//!    chat` + cron) observe the same lockout instead of each independently
//!    re-discovering it.
//! 3. **Concurrent fires** within one process (cron + interactive turn) can
//!    each `check()` cheaply before hammering the same dry RPH bucket.
//!
//! ## File layout
//!
//! ```text
//! ~/.librefang/rate_limits/<provider>__<key_id_hash>.json
//! ```
//!
//! - `provider` is a short, stable string (e.g. `"openai"`, `"anthropic"`).
//! - `key_id_hash` is the first 16 hex characters of `sha256(api_key)`.
//!   Independent keys for the same provider get independent files. Keys are
//!   never written to disk in any form longer than 16 hex characters.
//!
//! ## Schema
//!
//! ```json
//! { "provider": "openai", "until_unix": 1733254800, "reason": "RPH cap reached" }
//! ```
//!
//! `until_unix` is a UNIX timestamp (seconds since epoch). We deliberately
//! avoid `std::time::Instant` in the file format because `Instant` is
//! process-local and meaningless across restarts.
//!
//! ## Atomicity
//!
//! Writes go through a temp file `<final>.tmp.<pid>.<rand>` followed by
//! `fsync` and `rename`, so a reader never sees a partially-written file.
//! Reads are best-effort; corrupt or unparsable files are logged and treated
//! as "no lockout" rather than failing the request.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

/// Default cooldown applied when a 429 is received but no usable
/// `retry-after` / `x-ratelimit-reset-*` header is present.
///
/// Matches the hermes-agent reference (5 minutes) and the per-driver fallback
/// already used in [`crate::rate_limit_tracker`].
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(300);

/// On-disk record of a single provider+key lockout.
///
/// Field names are part of the file format — do not rename without a
/// migration. `until_unix` is seconds since the Unix epoch; we use
/// `SystemTime` at the API boundary and `u64` on disk because `Instant` is
/// process-local and would be meaningless after a daemon restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredLockout {
    /// Provider name as passed to [`record`] (`"openai"`, `"anthropic"`, …).
    provider: String,
    /// UNIX timestamp (seconds) at which the lockout expires.
    until_unix: u64,
    /// Optional human-readable reason; not parsed, only logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Compute the 16-hex-character key identifier used in the on-disk filename.
///
/// `sha256(api_key)`, truncated to 8 bytes / 16 hex chars. 16 chars give
/// 64 bits of separation between independent keys — far more than needed
/// for filename collision avoidance, while still being short enough that
/// the leaked filename does not let an attacker brute-force the key.
pub fn key_id_hash(api_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for b in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Resolve the directory in which lockout files live.
///
/// Honors `LIBREFANG_HOME` first (matches `librefang_kernel::config`), then
/// falls back to `~/.librefang`, then to a temp dir if the home directory
/// cannot be determined (e.g. in tests on CI runners with no `$HOME`).
fn rate_limit_dir() -> PathBuf {
    let home = if let Ok(custom) = std::env::var("LIBREFANG_HOME") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".librefang")
    };
    home.join("rate_limits")
}

/// Compute the absolute path of the lockout file for a given (provider, key).
fn lockout_path(provider: &str, key_id: &str) -> PathBuf {
    // Sanitise provider — only ASCII alphanumerics, `_` and `-`.  Anything
    // else is replaced with `_` so a hostile/malformed provider name cannot
    // do path traversal (e.g. `../foo`) or collide with hash characters.
    let safe_provider: String = provider
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    rate_limit_dir().join(format!("{safe_provider}__{key_id}.json"))
}

/// Atomically persist a lockout for `(provider, key_id)` lasting until
/// `until` (a wall-clock `SystemTime`).
///
/// Implementation: write to `<final>.tmp.<pid>.<rand>`, `fsync`, then
/// `rename` to the final name. If the rate-limit dir does not yet exist it
/// is created with default permissions.
///
/// Errors are logged at `warn!` and swallowed — the durable layer is
/// best-effort; failing to persist a lockout must not break the request
/// path. The in-process `RateLimitBucket` continues to provide protection.
pub fn record(provider: &str, key_id: &str, until: SystemTime, reason: Option<String>) {
    if let Err(e) = record_inner(provider, key_id, until, reason) {
        warn!(
            target: "librefang::shared_rate_guard",
            provider, key_id, error = %e,
            "failed to persist rate-limit lockout (continuing — in-process tracker still active)"
        );
    }
}

fn record_inner(
    provider: &str,
    key_id: &str,
    until: SystemTime,
    reason: Option<String>,
) -> std::io::Result<()> {
    let until_unix = until
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let final_path = lockout_path(provider, key_id);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Max-merge with any existing lockout. A sibling process or earlier
    // 429 may have recorded a *longer* cooldown (e.g. the 1h RPH header
    // called out in #3315). The current response might only carry a
    // short `retry-after` — overwriting unconditionally would forget the
    // stronger lockout. Read-modify-write here is not atomic across
    // processes, but the "take the longer of the two" invariant
    // converges across repeated writes and recovers the worst-case
    // bound (a 1h reset surviving until it really expires).
    if let Ok(bytes) = fs::read(&final_path) {
        if let Ok(existing) = serde_json::from_slice::<StoredLockout>(&bytes) {
            if existing.until_unix > until_unix {
                debug!(
                    target: "librefang::shared_rate_guard",
                    provider, key_id,
                    existing_until = existing.until_unix,
                    new_until = until_unix,
                    "skipping record — existing lockout is longer"
                );
                return Ok(());
            }
        }
    }

    let stored = StoredLockout {
        provider: provider.to_string(),
        until_unix,
        reason,
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(&final_path, &bytes)?;
    debug!(
        target: "librefang::shared_rate_guard",
        provider, key_id, until_unix,
        path = %final_path.display(),
        "recorded rate-limit lockout"
    );
    Ok(())
}

/// Write `bytes` to `final_path` atomically: temp file in the same dir,
/// `fsync`, then `rename`.
fn write_atomic(final_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = final_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rate-limit lockout path has no parent directory",
        )
    })?;
    let pid = std::process::id();
    let nonce: u64 = rand::random();
    let file_name = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("lockout.json");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{pid}.{nonce:016x}"));

    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    // `rename` is atomic on POSIX; on Windows it replaces an existing file
    // when the source and destination are on the same volume — which is
    // always true here because we placed the temp file in the same dir.
    if let Err(e) = fs::rename(&tmp_path, final_path) {
        // Best-effort cleanup; primary error is the rename failure.
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

/// Convenience helper: parse a `RateLimitSnapshot` (or 429 retry-after
/// header) and persist a lockout extending until the soonest reset.
///
/// Precedence (matches `rate_limit_tracker`):
///   1. requests-per-hour reset (preferred — covers Nous Portal / OpenAI
///      tier-1 RPH bans)
///   2. requests-per-minute reset
///   3. caller-supplied `retry_after` (e.g. from the `Retry-After` header)
///   4. [`DEFAULT_COOLDOWN`] (5 minutes)
pub fn record_from_snapshot(
    provider: &str,
    key_id: &str,
    snapshot: Option<&crate::rate_limit_tracker::RateLimitSnapshot>,
    retry_after: Option<Duration>,
    reason: Option<String>,
) {
    let cooldown = pick_cooldown(snapshot, retry_after);
    let until = SystemTime::now() + cooldown;
    record(provider, key_id, until, reason);
}

fn pick_cooldown(
    snapshot: Option<&crate::rate_limit_tracker::RateLimitSnapshot>,
    retry_after: Option<Duration>,
) -> Duration {
    if let Some(snap) = snapshot {
        // RPH is preferred — the issue specifically calls out RPH-strict
        // providers (Nous Portal, OpenAI tier-1).  A 1-hour reset is the
        // dangerous case we must not forget across restarts.
        // `reset_after_secs` is parsed from an upstream `X-RateLimit-Reset`
        // header (f64). `rph > 0.0` excludes negatives and NaN but admits
        // `f64::INFINITY` (a malformed/overflowing header like `1e400` or
        // `inf` parses to +inf), and `Duration::from_secs_f64(INFINITY)`
        // panics in this driver hot path. Require a finite, positive value.
        let rph = snap.requests_per_hour.reset_after_secs;
        if snap.requests_per_hour.has_data() && rph.is_finite() && rph > 0.0 {
            return Duration::from_secs_f64(rph);
        }
        let rpm = snap.requests_per_minute.reset_after_secs;
        if snap.requests_per_minute.has_data() && rpm.is_finite() && rpm > 0.0 {
            return Duration::from_secs_f64(rpm);
        }
    }
    if let Some(d) = retry_after {
        if !d.is_zero() {
            return d;
        }
    }
    DEFAULT_COOLDOWN
}

/// Check whether `(provider, key_id)` is currently locked out.
///
/// - Returns `None` if the file does not exist, is corrupt, or the recorded
///   `until` has already passed.
/// - Returns `Some(remaining)` if a lockout is still in effect.
///
/// Expired files are deleted on a best-effort basis to keep the dir tidy.
pub fn check(provider: &str, key_id: &str) -> Option<Duration> {
    let path = lockout_path(provider, key_id);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(
                target: "librefang::shared_rate_guard",
                provider, key_id, error = %e,
                "failed to read rate-limit lockout file"
            );
            return None;
        }
    };
    let stored: StoredLockout = match serde_json::from_slice(&bytes) {
        Ok(s) => s,
        Err(e) => {
            warn!(
                target: "librefang::shared_rate_guard",
                provider, key_id, error = %e,
                "rate-limit lockout file is corrupt; ignoring"
            );
            return None;
        }
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if stored.until_unix <= now {
        // Expired — sweep best-effort.
        let _ = fs::remove_file(&path);
        return None;
    }
    Some(Duration::from_secs(stored.until_unix - now))
}

/// Pre-request guard for drivers.
///
/// Returns `Ok(())` if no lockout is in effect, or
/// `Err(LlmError::RateLimited)` sized to the remaining cooldown so the
/// caller can short-circuit before any HTTP work.
pub fn pre_request_check(
    provider: &str,
    key_id: &str,
    label: &str,
) -> Result<(), librefang_llm_driver::LlmError> {
    if let Some(remaining) = check(provider, key_id) {
        warn!(
            target: "librefang::shared_rate_guard",
            provider,
            remaining_secs = remaining.as_secs(),
            "skipping {label} request — provider is rate-limited per persistent guard"
        );
        return Err(librefang_llm_driver::LlmError::RateLimited {
            retry_after_ms: remaining.as_millis().min(u64::MAX as u128) as u64,
            message: Some("rate limit recorded from previous response".into()),
        });
    }
    Ok(())
}

/// Persist a 429 lockout from a `reqwest` response's headers and return
/// the parsed `Retry-After` so the caller can reuse it for backoff.
///
/// Header parsing precedence (RPH > RPM > Retry-After > 5min default) is
/// delegated to [`record_from_snapshot`].
pub fn record_429_from_headers(
    provider: &str,
    key_id: &str,
    headers: &reqwest::header::HeaderMap,
    reason: &str,
) -> Duration {
    // 0 fallback preserves the existing precedence semantics
    // (RPH > RPM > Retry-After > 5min default) — a missing or
    // unparseable header means "no Retry-After signal", not "5 s".
    let retry_after = crate::retry_after::parse_retry_after(headers, 0);
    let snap = crate::rate_limit_tracker::RateLimitSnapshot::from_headers(headers);
    record_from_snapshot(
        provider,
        key_id,
        snap.as_ref(),
        Some(retry_after),
        Some(reason.to_string()),
    );
    // Cardinality: provider is a fixed enum (anthropic/openai/gemini/…),
    // status is the bare HTTP status code as string. We never include
    // key_id or reason text. (#3495)
    metrics::counter!(
        "librefang_llm_provider_errors_total",
        "provider" => provider.to_string(),
        "status" => "429",
    )
    .increment(1);
    retry_after
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests mutate `LIBREFANG_HOME`, which is process-global.  Serialise
    /// them so they don't trample one another when run in parallel.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    fn fresh_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn key_id_hash_is_stable_and_short() {
        let h1 = key_id_hash("sk-test-1234");
        let h2 = key_id_hash("sk-test-1234");
        assert_eq!(h1, h2, "hash must be deterministic");
        assert_eq!(h1.len(), 16, "hash must be 16 hex chars");
        assert!(
            h1.chars().all(|c| c.is_ascii_hexdigit()),
            "hash must be hex"
        );
        let h3 = key_id_hash("sk-test-5678");
        assert_ne!(h1, h3, "different keys must hash differently");
    }

    #[test]
    fn record_then_check_round_trip() {
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        let key_id = key_id_hash("sk-roundtrip");
        assert!(
            check("openai", &key_id).is_none(),
            "no file → not locked out"
        );

        let until = SystemTime::now() + Duration::from_secs(60);
        record("openai", &key_id, until, Some("test".into()));

        let remaining = check("openai", &key_id).expect("must be locked out");
        assert!(
            remaining.as_secs() > 50 && remaining.as_secs() <= 60,
            "remaining ~60s, got {}s",
            remaining.as_secs()
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    #[test]
    fn expired_lockout_is_cleared() {
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        let key_id = key_id_hash("sk-expired");
        // until = 1s in the past
        let until = SystemTime::now() - Duration::from_secs(1);
        record("openai", &key_id, until, None);

        assert!(
            check("openai", &key_id).is_none(),
            "expired file must report no lockout"
        );
        assert!(
            !lockout_path("openai", &key_id).exists(),
            "expired file should have been removed"
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    #[test]
    fn corrupt_file_is_treated_as_unlocked() {
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        let key_id = key_id_hash("sk-corrupt");
        let path = lockout_path("openai", &key_id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"this is not json").unwrap();

        assert!(
            check("openai", &key_id).is_none(),
            "corrupt file must not block requests"
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    #[test]
    fn second_process_observes_same_lockout() {
        // Simulates the cross-process scenario: process A records, process B
        // (i.e. a fresh check() call without ever calling record() in this
        // test scope) sees the lockout.  Same code path, but exercised
        // separately to lock down the contract called out in the issue's
        // acceptance criteria.
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        let key_id = key_id_hash("sk-shared");
        record(
            "openai",
            &key_id,
            SystemTime::now() + Duration::from_secs(120),
            None,
        );

        // Pretend we are a brand-new process: nothing in memory.
        let observed = check("openai", &key_id);
        assert!(
            matches!(observed, Some(d) if d.as_secs() > 100),
            "second process must see >100s remaining, got {observed:?}"
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    #[test]
    fn pick_cooldown_prefers_rph_over_rpm() {
        use crate::rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot};
        use std::time::Instant;
        let snap = RateLimitSnapshot {
            requests_per_minute: RateLimitBucket {
                limit: 100,
                remaining: 0,
                reset_after_secs: 30.0,
                captured_at: Instant::now(),
            },
            requests_per_hour: RateLimitBucket {
                limit: 1000,
                remaining: 0,
                reset_after_secs: 3540.0,
                captured_at: Instant::now(),
            },
            ..Default::default()
        };
        let cooldown = pick_cooldown(Some(&snap), Some(Duration::from_secs(5)));
        assert_eq!(
            cooldown.as_secs(),
            3540,
            "must prefer requests-per-hour reset"
        );
    }

    #[test]
    fn pick_cooldown_falls_back_to_retry_after() {
        let cooldown = pick_cooldown(None, Some(Duration::from_secs(42)));
        assert_eq!(cooldown.as_secs(), 42);
    }

    #[test]
    fn pick_cooldown_falls_back_to_default() {
        let cooldown = pick_cooldown(None, None);
        assert_eq!(cooldown, DEFAULT_COOLDOWN);
    }

    #[test]
    fn provider_name_is_sanitised() {
        // A hostile provider name must not be able to write outside the
        // rate_limits dir.
        let path = lockout_path("../etc/passwd", "abc");
        let s = path.to_string_lossy();
        assert!(
            !s.contains("../etc/passwd"),
            "path traversal not sanitised: {s}"
        );
        assert!(s.contains("___etc_passwd__abc"), "got {s}");
    }

    #[test]
    fn record_keeps_longer_lockout_against_shorter_followup() {
        // Simulates two processes hitting 429 in sequence:
        //   A: x-ratelimit-reset-requests-1h: 3540  → records 1h cooldown
        //   B: only retry-after: 30                 → would shorten to 30s
        // The 1h lockout MUST survive — that's the strongest guarantee
        // #3315 asks for and the regression we're locking down here.
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        let key_id = key_id_hash("sk-max-merge");

        let long_until = SystemTime::now() + Duration::from_secs(3540);
        record("openai", &key_id, long_until, Some("1h RPH".into()));

        let short_until = SystemTime::now() + Duration::from_secs(30);
        record(
            "openai",
            &key_id,
            short_until,
            Some("retry-after 30s".into()),
        );

        let remaining = check("openai", &key_id).expect("locked out");
        assert!(
            remaining.as_secs() > 3000,
            "longer lockout must survive a shorter follow-up record(); \
             got {}s remaining (expected ~3540)",
            remaining.as_secs()
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    #[test]
    fn record_extends_existing_shorter_lockout() {
        // Inverse of `record_keeps_longer_lockout_against_shorter_followup`:
        // if the new lockout is *longer*, it MUST replace the old one.
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        let key_id = key_id_hash("sk-extend");

        let short_until = SystemTime::now() + Duration::from_secs(30);
        record("openai", &key_id, short_until, None);

        let long_until = SystemTime::now() + Duration::from_secs(3540);
        record("openai", &key_id, long_until, None);

        let remaining = check("openai", &key_id).expect("locked out");
        assert!(
            remaining.as_secs() > 3000,
            "newer longer lockout must replace shorter; got {}s",
            remaining.as_secs()
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    #[test]
    fn record_from_snapshot_writes_file() {
        let _g = ENV_GUARD.lock().unwrap();
        let home = fresh_home();
        // SAFETY: guarded by ENV_GUARD mutex; no concurrent thread reads this var.
        unsafe { std::env::set_var("LIBREFANG_HOME", home.path()) };

        use crate::rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot};
        use std::time::Instant;
        let snap = RateLimitSnapshot {
            requests_per_hour: RateLimitBucket {
                limit: 1000,
                remaining: 0,
                reset_after_secs: 3540.0,
                captured_at: Instant::now(),
            },
            ..Default::default()
        };
        let key_id = key_id_hash("sk-from-snapshot");
        record_from_snapshot("openai", &key_id, Some(&snap), None, Some("429".into()));

        let remaining = check("openai", &key_id).expect("locked out");
        assert!(
            remaining.as_secs() > 3500,
            "must record full RPH cooldown, got {}s",
            remaining.as_secs()
        );

        // SAFETY: guarded by ENV_GUARD mutex.
        unsafe { std::env::remove_var("LIBREFANG_HOME") };
    }

    // -- #5136: non-finite reset header must not panic the cooldown path ----

    #[test]
    fn pick_cooldown_rejects_infinite_rph_reset() {
        // A malformed / overflowing `X-RateLimit-Reset` header parses to
        // f64::INFINITY. The old `rph > 0.0` gate admitted it, and
        // `Duration::from_secs_f64(INFINITY)` panics in this driver hot
        // path. The guard must fall back instead of panicking.
        use crate::rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot};
        use std::time::Instant;
        let snap = RateLimitSnapshot {
            requests_per_hour: RateLimitBucket {
                limit: 1000,
                remaining: 0,
                reset_after_secs: f64::INFINITY,
                captured_at: Instant::now(),
            },
            ..Default::default()
        };
        // Must not panic; with no usable RPH/RPM and no retry_after, falls
        // back to DEFAULT_COOLDOWN.
        let cd = pick_cooldown(Some(&snap), None);
        assert_eq!(cd, DEFAULT_COOLDOWN);

        // A finite caller-supplied retry_after still wins over the bogus RPH.
        let cd2 = pick_cooldown(Some(&snap), Some(Duration::from_secs(42)));
        assert_eq!(cd2, Duration::from_secs(42));
    }

    #[test]
    fn pick_cooldown_rejects_infinite_rpm_reset() {
        use crate::rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot};
        use std::time::Instant;
        let snap = RateLimitSnapshot {
            requests_per_minute: RateLimitBucket {
                limit: 60,
                remaining: 0,
                reset_after_secs: f64::INFINITY,
                captured_at: Instant::now(),
            },
            ..Default::default()
        };
        let cd = pick_cooldown(Some(&snap), None);
        assert_eq!(cd, DEFAULT_COOLDOWN);
    }
}
