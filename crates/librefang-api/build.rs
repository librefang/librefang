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

    // Capture git commit hash at build time.
    let git_sha = Command::new("git")
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
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_SHA={git_sha}");

    // Capture build date (UTC, date only).
    let build_date = Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
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
