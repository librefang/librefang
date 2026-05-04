//! Local check mode: throttle escape hatch for compile-heavy xtask subcommands.
//!
//! Background (refs #3301): `cargo xtask ci` and friends run at full
//! concurrency regardless of host capability. That is fine on a beefy CI
//! runner, but reliably OOMs the linker or proc-macro pipeline on a
//! 16 GB / 4-CPU laptop. This module probes the host once at the start of a
//! compile-heavy subcommand and adjusts a small set of cargo / rustc env
//! vars to match.
//!
//! Behaviour matrix:
//!
//! | Mode      | When                                          | Effect                                                |
//! |-----------|-----------------------------------------------|-------------------------------------------------------|
//! | `Full`    | `CI=true`, or auto-detect on capable hosts    | No env tweaks (matches historical behaviour)         |
//! | `Throttled` | Auto-detect on `mem < 16 GB` or `cpus < 4` | Single-thread cargo, `codegen-units=1`, larger stack |
//! | `Off`     | User explicit opt-out                         | No env tweaks (user knows what they are doing)       |
//!
//! Override: `LIBREFANG_LOCAL_CHECK_MODE` accepts `throttled`, `full`, `off`.
//! Anything else (including unset) falls through to auto-detection.
//!
//! The function preserves any caller-set env values — `set_env_if_unset` only
//! writes when the variable is absent, and `append_rustflags` concatenates
//! rather than replacing.

use std::env;
use std::fmt;

/// Threshold below which we auto-throttle. 16 GB / 4 CPUs is the OOM line on
/// the issue reporter's laptop and matches the openclaw heuristic.
const LOW_SPEC_MEM_GB: u64 = 16;
const LOW_SPEC_CPUS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCheckMode {
    /// User explicitly opts out of any env tweaking.
    Off,
    /// Reduce concurrency for low-spec hosts.
    Throttled,
    /// Full concurrency (CI default, capable laptops).
    Full,
}

impl fmt::Display for LocalCheckMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocalCheckMode::Off => f.write_str("off"),
            LocalCheckMode::Throttled => f.write_str("throttled"),
            LocalCheckMode::Full => f.write_str("full"),
        }
    }
}

/// Snapshot of the host probe used to decide the mode.
#[derive(Debug, Clone, Copy)]
pub struct HostProbe {
    pub cpus: usize,
    pub mem_gb: u64,
}

impl HostProbe {
    pub fn detect() -> Self {
        Self {
            cpus: detect_cpus(),
            mem_gb: detect_mem_gb(),
        }
    }
}

fn detect_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn detect_mem_gb() -> u64 {
    // Only refresh RAM — skip processes / disks / networks for a fast probe.
    let sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::new().with_memory(sysinfo::MemoryRefreshKind::new().with_ram()),
    );
    // sysinfo reports bytes; convert to GB rounding down so a 15.9 GB
    // machine is correctly treated as "below 16".
    sys.total_memory() / (1024 * 1024 * 1024)
}

/// Resolve the mode from the env override, falling back to auto-detection.
pub fn detect() -> (LocalCheckMode, HostProbe) {
    let probe = HostProbe::detect();
    let mode = match env::var("LIBREFANG_LOCAL_CHECK_MODE")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("off") => LocalCheckMode::Off,
        Some("throttled") => LocalCheckMode::Throttled,
        Some("full") => LocalCheckMode::Full,
        _ => auto_detect(&probe),
    };
    (mode, probe)
}

fn auto_detect(probe: &HostProbe) -> LocalCheckMode {
    // CI always runs at full concurrency. The widely-used `CI=true`
    // convention is honoured by GitHub Actions, GitLab CI, CircleCI, etc.
    if env::var_os("CI").is_some() {
        return LocalCheckMode::Full;
    }
    if probe.mem_gb < LOW_SPEC_MEM_GB || probe.cpus < LOW_SPEC_CPUS {
        LocalCheckMode::Throttled
    } else {
        LocalCheckMode::Full
    }
}

/// Apply mode-specific env tweaks. Existing env values win — we only fill in
/// gaps and append (never replace) to RUSTFLAGS.
pub fn apply(mode: LocalCheckMode) {
    match mode {
        LocalCheckMode::Full | LocalCheckMode::Off => {}
        LocalCheckMode::Throttled => {
            set_env_if_unset("CARGO_BUILD_JOBS", "1");
            append_rustflags("-C codegen-units=1");
            // 8 MiB. Stops proc-macro and test-harness recursion from
            // tipping the linker over on low-mem hosts.
            set_env_if_unset("RUST_MIN_STACK", "8388608");
        }
    }
}

/// Convenience: detect + apply + print a one-line banner. Call this as the
/// first line of any compile-heavy xtask subcommand.
pub fn apply_for_subcommand(name: &str) {
    let (mode, probe) = detect();
    apply(mode);
    println!(
        "xtask {name}: local-check-mode = {mode} (cpus={}, mem={} GB)",
        probe.cpus, probe.mem_gb
    );
}

fn set_env_if_unset(key: &str, val: &str) {
    if env::var_os(key).is_none() {
        // xtask is single-threaded at this point — called before any cargo
        // subprocess is spawned and before any thread pool starts. Edition
        // 2021 still treats `set_var` as safe; the wider migration to the
        // 2024-edition `unsafe` form lives outside this PR.
        env::set_var(key, val);
    }
}

fn append_rustflags(extra: &str) {
    let cur = env::var("RUSTFLAGS").unwrap_or_default();
    let new = if cur.is_empty() {
        extra.to_string()
    } else {
        // `extra` may itself be multi-token (e.g. "-C codegen-units=1"),
        // so token-by-token equality is wrong. Treat it as a contiguous
        // sub-sequence of whitespace-separated tokens.
        let cur_toks: Vec<&str> = cur.split_whitespace().collect();
        let extra_toks: Vec<&str> = extra.split_whitespace().collect();
        let already_present = !extra_toks.is_empty()
            && extra_toks.len() <= cur_toks.len()
            && cur_toks
                .windows(extra_toks.len())
                .any(|w| w == extra_toks.as_slice());
        if already_present {
            cur
        } else {
            format!("{cur} {extra}")
        }
    };
    env::set_var("RUSTFLAGS", new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // env::set_var is process-global; serialize tests that touch env.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            Self {
                keys: keys.iter().map(|k| (*k, env::var(k).ok())).collect(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.keys {
                match v {
                    Some(val) => env::set_var(k, val),
                    None => env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn detect_explicit_throttled() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["LIBREFANG_LOCAL_CHECK_MODE", "CI"]);
        env::set_var("LIBREFANG_LOCAL_CHECK_MODE", "throttled");
        env::remove_var("CI");
        let (mode, _) = detect();
        assert_eq!(mode, LocalCheckMode::Throttled);
    }

    #[test]
    fn detect_explicit_full_overrides_low_spec() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["LIBREFANG_LOCAL_CHECK_MODE", "CI"]);
        env::set_var("LIBREFANG_LOCAL_CHECK_MODE", "full");
        env::remove_var("CI");
        let (mode, _) = detect();
        assert_eq!(mode, LocalCheckMode::Full);
    }

    #[test]
    fn detect_explicit_off() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["LIBREFANG_LOCAL_CHECK_MODE", "CI"]);
        env::set_var("LIBREFANG_LOCAL_CHECK_MODE", "off");
        env::remove_var("CI");
        let (mode, _) = detect();
        assert_eq!(mode, LocalCheckMode::Off);
    }

    #[test]
    fn detect_unknown_value_falls_through_to_auto() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["LIBREFANG_LOCAL_CHECK_MODE", "CI"]);
        env::set_var("LIBREFANG_LOCAL_CHECK_MODE", "weird");
        env::set_var("CI", "true");
        // CI=true forces Full regardless of host specs.
        let (mode, _) = detect();
        assert_eq!(mode, LocalCheckMode::Full);
    }

    #[test]
    fn auto_detect_low_spec_returns_throttled() {
        let probe = HostProbe { cpus: 2, mem_gb: 8 };
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CI"]);
        env::remove_var("CI");
        assert_eq!(auto_detect(&probe), LocalCheckMode::Throttled);
    }

    #[test]
    fn auto_detect_high_spec_returns_full() {
        let probe = HostProbe {
            cpus: 16,
            mem_gb: 64,
        };
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CI"]);
        env::remove_var("CI");
        assert_eq!(auto_detect(&probe), LocalCheckMode::Full);
    }

    #[test]
    fn auto_detect_ci_forces_full_on_low_spec() {
        let probe = HostProbe { cpus: 1, mem_gb: 2 };
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CI"]);
        env::set_var("CI", "true");
        assert_eq!(auto_detect(&probe), LocalCheckMode::Full);
    }

    #[test]
    fn apply_throttled_sets_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CARGO_BUILD_JOBS", "RUSTFLAGS", "RUST_MIN_STACK"]);
        env::remove_var("CARGO_BUILD_JOBS");
        env::remove_var("RUSTFLAGS");
        env::remove_var("RUST_MIN_STACK");
        apply(LocalCheckMode::Throttled);
        assert_eq!(env::var("CARGO_BUILD_JOBS").unwrap(), "1");
        assert!(env::var("RUSTFLAGS")
            .unwrap()
            .contains("-C codegen-units=1"));
        assert_eq!(env::var("RUST_MIN_STACK").unwrap(), "8388608");
    }

    #[test]
    fn apply_throttled_preserves_existing_rustflags() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CARGO_BUILD_JOBS", "RUSTFLAGS", "RUST_MIN_STACK"]);
        env::set_var("RUSTFLAGS", "-C target-cpu=native");
        env::remove_var("CARGO_BUILD_JOBS");
        env::remove_var("RUST_MIN_STACK");
        apply(LocalCheckMode::Throttled);
        let flags = env::var("RUSTFLAGS").unwrap();
        assert!(flags.contains("-C target-cpu=native"));
        assert!(flags.contains("-C codegen-units=1"));
    }

    #[test]
    fn apply_throttled_does_not_override_user_jobs() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CARGO_BUILD_JOBS", "RUSTFLAGS", "RUST_MIN_STACK"]);
        env::set_var("CARGO_BUILD_JOBS", "4");
        env::remove_var("RUSTFLAGS");
        env::remove_var("RUST_MIN_STACK");
        apply(LocalCheckMode::Throttled);
        assert_eq!(env::var("CARGO_BUILD_JOBS").unwrap(), "4");
    }

    #[test]
    fn apply_full_is_a_noop() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CARGO_BUILD_JOBS", "RUSTFLAGS", "RUST_MIN_STACK"]);
        env::remove_var("CARGO_BUILD_JOBS");
        env::remove_var("RUSTFLAGS");
        env::remove_var("RUST_MIN_STACK");
        apply(LocalCheckMode::Full);
        assert!(env::var_os("CARGO_BUILD_JOBS").is_none());
        assert!(env::var_os("RUSTFLAGS").is_none());
        assert!(env::var_os("RUST_MIN_STACK").is_none());
    }

    #[test]
    fn apply_off_is_a_noop() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["CARGO_BUILD_JOBS", "RUSTFLAGS", "RUST_MIN_STACK"]);
        env::remove_var("CARGO_BUILD_JOBS");
        env::remove_var("RUSTFLAGS");
        env::remove_var("RUST_MIN_STACK");
        apply(LocalCheckMode::Off);
        assert!(env::var_os("CARGO_BUILD_JOBS").is_none());
        assert!(env::var_os("RUSTFLAGS").is_none());
        assert!(env::var_os("RUST_MIN_STACK").is_none());
    }

    #[test]
    fn append_rustflags_dedups() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(&["RUSTFLAGS"]);
        env::set_var("RUSTFLAGS", "-C codegen-units=1");
        append_rustflags("-C codegen-units=1");
        // No duplication.
        assert_eq!(env::var("RUSTFLAGS").unwrap(), "-C codegen-units=1");
    }
}
