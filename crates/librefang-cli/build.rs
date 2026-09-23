use std::process::Command;

fn main() {
    // Re-run when the env inputs to git-sha / build-date capture change so
    // cargo invalidates this build script appropriately (refs #5667).
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=CI_COMMIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // Declared only when the checkout is what decides the commit id: with a CI-supplied SHA present `resolve_git_sha` returns before it looks at the repository, so these inputs could not change the output and declaring them would spend three `git` invocations plus their PATH lookups on every build for nothing.
    if !ci_supplied_sha() {
        rerun_when_commit_changes();
    }

    // Capture git commit hash at build time.
    //
    // Prefer CI-provided env vars (`GITHUB_SHA`, `CI_COMMIT_SHA`) — they're
    // authoritative on hosted runners and avoid spawning `git` entirely.
    // Outside CI, resolve the `git` binary via `which` first so we don't
    // depend on shell PATH lookup semantics, then call `git rev-parse`.
    let git_sha = resolve_git_sha();
    println!("cargo:rustc-env=GIT_SHA={git_sha}");

    // Capture build date (UTC, date only) via `chrono::Utc::now()` rather
    // than shelling out to `date -u +%Y-%m-%d`. Removes a platform-specific
    // process spawn (BSD `date` and GNU `date` accept different flags) and
    // keeps the build script reproducible across hosts.
    let build_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    // Capture rustc version.
    let rustc_ver = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=RUSTC_VERSION={rustc_ver}");

    reserve_windows_main_thread_stack();
}

/// Raise the `librefang` binary's main-thread stack reserve on Windows.
///
/// Windows reserves 1 MiB for a process's main thread unless the image header says otherwise, against 8 MiB on Linux and macOS.
/// `main` dispatches a `Commands` enum with hundreds of variants into handlers that drive the agent loop's deeply-nested async state machines, and in an unoptimised build the per-arm stack slots are not merged: the frame exceeds 1 MiB before the first command does any work, so *every* invocation of a debug `librefang.exe` died at startup with `STATUS_STACK_OVERFLOW` (`0xC00000FD`, surfacing as exit code `-1073741571`).
///
/// The reserve is virtual address space, committed page by page as the stack actually grows, so this costs nothing at runtime on a 64-bit target.
/// 16 MiB matches the `RUST_MIN_STACK` floor `.cargo/config.toml` already sets for the same reason on cargo-spawned threads — that variable only reaches spawned threads, never the main one, which is why it did not cover this.
///
/// Lives in the build script rather than `.cargo/config.toml` so it travels with the crate: a `cargo install librefang-cli` outside this repo gets it too.
fn reserve_windows_main_thread_stack() {
    const STACK_BYTES: usize = 16 * 1024 * 1024;

    let Ok(os) = std::env::var("CARGO_CFG_TARGET_OS") else {
        return;
    };
    if os != "windows" {
        return;
    }
    // The two Windows toolchains spell it differently, and handing either flag to the other's linker is a hard error rather than a warning.
    let arg = match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => format!("/STACK:{STACK_BYTES}"),
        _ => format!("-Wl,--stack,{STACK_BYTES}"),
    };
    println!("cargo:rustc-link-arg-bin=librefang={arg}");
}

/// Resolve the short git SHA for the build.
///
/// Order of preference:
/// 1. `GITHUB_SHA` (GitHub Actions)
/// 2. `CI_COMMIT_SHA` (GitLab CI, generic)
/// 3. `git rev-parse --short HEAD`, with `git` located via `which`.
/// 4. `"unknown"` if all of the above fail.
fn resolve_git_sha() -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        let sha = sha.trim();
        if !sha.is_empty() {
            // GitHub provides a full 40-char SHA; truncate to the same
            // short form `git rev-parse --short HEAD` would produce.
            return short_sha(sha);
        }
    }
    if let Ok(sha) = std::env::var("CI_COMMIT_SHA") {
        let sha = sha.trim();
        if !sha.is_empty() {
            return short_sha(sha);
        }
    }

    git_stdout(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
}

/// Declare the files git rewrites when the commit under `HEAD` moves, so cargo re-runs this script and re-captures the commit id (refs #8414).
///
/// The `rerun-if-env-changed` directives above only cover the CI-supplied SHAs, and a commit moves none of them.
/// Without an input that follows the repository, the script kept reporting the commit it captured on its first run, so a binary built at a later commit advertised the earlier one.
///
/// The paths are asked of git rather than spelled `.git/…`.
/// In a linked worktree `.git` is a file, and the branch that worktree has checked out lives in the repository's common directory, which is what `rev-parse --git-path` relocates to.
///
/// `HEAD` alone is not enough: it holds `ref: refs/heads/<branch>`, which a commit leaves untouched.
/// It is listed for the two cases that do move it — checking out a different branch, and a detached checkout, where the commit id is written into `HEAD` itself.
fn rerun_when_commit_changes() {
    // Nothing to watch outside a checkout; a registry tarball has no repository and falls back to `unknown` regardless.
    let Some(git_dir) = git_stdout(&["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    println!("cargo:rerun-if-changed={git_dir}/HEAD");

    if let Some(branch) = git_stdout(&["symbolic-ref", "-q", "HEAD"]) {
        // `--git-path` may answer relative to the directory git ran in, and that directory is this package's root — the same base cargo resolves a relative `rerun-if-changed` against, so the answer is passed through unchanged.
        if let Some(reference) = git_stdout(&["rev-parse", "--git-path", &branch]) {
            println!("cargo:rerun-if-changed={reference}");
        }
    }
}

/// Whether a CI-supplied commit id is what [`resolve_git_sha`] is going to return.
///
/// The two variables are read in the same order and with the same emptiness rule there, so `false` here means the checkout is genuinely what decides the reported commit.
fn ci_supplied_sha() -> bool {
    ["GITHUB_SHA", "CI_COMMIT_SHA"]
        .iter()
        .any(|key| std::env::var(key).is_ok_and(|sha| !sha.trim().is_empty()))
}

/// The `git` binary, resolved once per run.
///
/// `which` walks `PATH` on every call and this script calls [`git_stdout`] up to three times, so resolving it once keeps the walk from repeating for an answer that cannot change while the script runs (refs #5667).
fn git_binary() -> Option<&'static std::path::PathBuf> {
    static GIT: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    GIT.get_or_init(|| which::which("git").ok()).as_ref()
}

/// Run `git <args>` and return its trimmed stdout, or `None` when git is missing, the command fails, or it prints nothing.
///
/// The binary is resolved through `which` rather than relying on shell PATH lookup semantics (refs #5667).
fn git_stdout(args: &[&str]) -> Option<String> {
    let git = git_binary()?;
    let output = Command::new(git).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn short_sha(sha: &str) -> String {
    // git's default short SHA length is 7. Match it for parity with the
    // `git rev-parse --short HEAD` fallback.
    sha.chars().take(7).collect()
}
