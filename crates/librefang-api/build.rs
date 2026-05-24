use std::process::Command;

fn main() {
    // Ensure the dashboard embed directory exists so `include_dir!` never
    // fails on fresh clones/worktrees. The directory is gitignored because
    // it contains build artifacts produced by `npm run build` in the
    // dashboard subcrate (or downloaded from release assets at runtime).
    // When empty, `include_dir!` embeds nothing and the runtime directory
    // `~/.librefang/dashboard/` serves the actual assets.
    let dashboard_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("static")
        .join("react");
    if !dashboard_dir.exists() {
        std::fs::create_dir_all(&dashboard_dir)
            .expect("failed to create static/react placeholder directory");
    }

    // Re-run when the env inputs to git-sha / build-date capture change so
    // cargo invalidates this build script appropriately (refs #5667).
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=CI_COMMIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

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

    let Ok(git) = which::which("git") else {
        return "unknown".to_string();
    };
    Command::new(git)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn short_sha(sha: &str) -> String {
    // git's default short SHA length is 7. Match it for parity with the
    // `git rev-parse --short HEAD` fallback.
    sha.chars().take(7).collect()
}
