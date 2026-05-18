use crate::common::repo_root;
use crate::local_check_mode;
use clap::Parser;
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
pub struct CiArgs {
    /// Skip web lint step
    #[arg(long)]
    pub no_web: bool,

    /// Skip test step
    #[arg(long)]
    pub no_test: bool,

    /// Use release profile for build
    #[arg(long)]
    pub release: bool,
}

fn run_step(name: &str, cmd: &mut Command) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== {} ===", name);
    let start = Instant::now();
    let status = cmd.status()?;
    let elapsed = start.elapsed();
    if !status.success() {
        return Err(format!(
            "{} failed (exit code: {:?}) [{:.1}s]",
            name,
            status.code(),
            elapsed.as_secs_f64()
        )
        .into());
    }
    println!("  Passed ({:.1}s)", elapsed.as_secs_f64());
    println!();
    Ok(())
}

/// Sidecar-first policy: every `impl ChannelAdapter` under
/// crates/librefang-channels/src/ must be grandfathered in
/// channels-allowlist.txt. Mirrors the pre-commit hook tree-wide so an
/// unset `core.hooksPath` can't bypass the gate.
fn check_channel_policy(root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let chan_src = root.join("crates/librefang-channels/src");
    let allowlist_path = chan_src.join("channels-allowlist.txt");
    let allow: std::collections::HashSet<String> = std::fs::read_to_string(&allowlist_path)?
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    let mut violations: Vec<String> = Vec::new();
    let mut check = |base: &str, file: &std::path::Path, rel: String| {
        if !allow.contains(base) {
            let content = std::fs::read_to_string(file).unwrap_or_default();
            if content.contains("impl ChannelAdapter") {
                violations.push(rel);
            }
        }
    };
    for entry in std::fs::read_dir(&chan_src)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Some(base) = path.file_stem().and_then(|s| s.to_str()) {
                let rel = format!("crates/librefang-channels/src/{base}.rs");
                check(base, &path, rel);
            }
        } else if path.is_dir() {
            let modrs = path.join("mod.rs");
            if modrs.exists() {
                if let Some(base) = path.file_name().and_then(|s| s.to_str()) {
                    let rel = format!("crates/librefang-channels/src/{base}/mod.rs");
                    check(base, &modrs, rel);
                }
            }
        }
    }

    if !violations.is_empty() {
        violations.sort();
        let mut msg = String::from(
            "in-process channel adapter(s) not on the sidecar-first \
             allowlist:\n",
        );
        for v in &violations {
            msg.push_str(&format!("  - {v}\n"));
        }
        msg.push_str(
            "New channels must be sidecar adapters. Grandfathering an \
             in-process adapter requires a maintainer decision: add its \
             basename to \
             crates/librefang-channels/src/channels-allowlist.txt.",
        );
        return Err(msg.into());
    }
    Ok(())
}

pub fn run(args: CiArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Apply LIBREFANG_LOCAL_CHECK_MODE before any cargo invocation (#3301).
    // Auto-throttles cargo concurrency on low-spec hosts; CI=true preserves
    // full parallelism. See `local_check_mode` for the behaviour matrix.
    local_check_mode::apply_for_subcommand("ci");

    let root = repo_root();
    let total_start = Instant::now();

    // Step 1: cargo build
    {
        let mut cmd = Command::new("cargo");
        cmd.args(["build", "--workspace", "--lib"])
            .current_dir(&root);
        if args.release {
            cmd.arg("--release");
        }
        run_step("cargo build", &mut cmd)?;
    }

    // Step 2: cargo test (unless --no-test)
    if !args.no_test {
        let mut cmd = Command::new("cargo");
        cmd.args(["test", "--workspace"]).current_dir(&root);
        if args.release {
            cmd.arg("--release");
        }
        run_step("cargo test", &mut cmd)?;
    }

    // Step 3: cargo clippy
    {
        let mut cmd = Command::new("cargo");
        cmd.args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(&root);
        run_step("cargo clippy", &mut cmd)?;
    }

    // Step 3b: sidecar-first channel policy (native check, no cargo).
    {
        println!("=== channel policy ===");
        let start = Instant::now();
        check_channel_policy(&root)?;
        println!("  Passed ({:.1}s)", start.elapsed().as_secs_f64());
        println!();
    }

    // Step 4: web lint (if web/package.json exists and not --no-web)
    if !args.no_web {
        let web_dir = root.join("web");
        let web_pkg = web_dir.join("package.json");
        if web_pkg.exists() {
            let mut cmd = Command::new("pnpm");
            cmd.args(["run", "lint"]).current_dir(&web_dir);
            run_step("web lint", &mut cmd)?;
        } else {
            println!("Skipping web lint (no web/package.json)");
        }
    }

    let total = total_start.elapsed();
    println!("All CI checks passed ({:.1}s total)", total.as_secs_f64());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_channel_policy;
    use std::fs;
    use std::path::PathBuf;

    struct TmpTree(PathBuf);
    impl Drop for TmpTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn make_tree(allowlist: &str) -> TmpTree {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lf-chanpol-{}-{nanos}", std::process::id()));
        let src = root.join("crates/librefang-channels/src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("channels-allowlist.txt"), allowlist).unwrap();
        TmpTree(root)
    }

    fn src(t: &TmpTree) -> PathBuf {
        t.0.join("crates/librefang-channels/src")
    }

    #[test]
    fn allowlisted_adapter_passes() {
        let t = make_tree("# header\n\nok\n");
        fs::write(
            src(&t).join("ok.rs"),
            "pub struct X;\nimpl ChannelAdapter for X {}\n",
        )
        .unwrap();
        // A non-adapter, non-allowlisted infra module is ignored.
        fs::write(src(&t).join("helpers.rs"), "pub fn util() {}\n").unwrap();
        assert!(check_channel_policy(&t.0).is_ok());
    }

    #[test]
    fn new_in_process_adapter_is_rejected() {
        let t = make_tree("ok\n");
        fs::write(src(&t).join("ok.rs"), "impl ChannelAdapter for A {}").unwrap();
        fs::write(src(&t).join("evil.rs"), "impl ChannelAdapter for E {}").unwrap();
        let err = check_channel_policy(&t.0).unwrap_err().to_string();
        assert!(err.contains("evil.rs"), "got: {err}");
        assert!(!err.contains("  - crates/librefang-channels/src/ok.rs"));
    }

    #[test]
    fn subdir_mod_adapter_is_rejected() {
        let t = make_tree("ok\n");
        let sub = src(&t).join("sneaky");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("mod.rs"), "impl ChannelAdapter for S {}").unwrap();
        let err = check_channel_policy(&t.0).unwrap_err().to_string();
        assert!(err.contains("sneaky/mod.rs"), "got: {err}");
    }
}
