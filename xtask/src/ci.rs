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

/// `cargo xtask channel-policy` — standalone, build-free entrypoint
/// for the sidecar-first gate, so CI can run it on EVERY PR (the
/// `quality` job's full `cargo xtask ci` only runs on a full_run).
#[derive(Parser, Debug)]
pub struct ChannelPolicyArgs {}

pub fn run_channel_policy(_args: ChannelPolicyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    println!("=== channel policy ===");
    let start = Instant::now();
    check_channel_policy(&root)?;
    println!("  Passed ({:.1}s)", start.elapsed().as_secs_f64());
    Ok(())
}

/// Sidecar-first policy: every channel adapter under
/// crates/librefang-channels/src/ must be grandfathered in
/// channels-allowlist.txt. Mirrors the pre-commit hook tree-wide.
///
/// Needle is `ChannelAdapter for` (not `impl ChannelAdapter`) so
/// `impl<T> ChannelAdapter for …` and odd whitespace are still
/// caught. Known, accepted limitation: a macro-generated impl, or a
/// new adapter `impl` added *inside* an already-allowlisted file, is
/// not detected — this is a policy ratchet, not a security boundary.
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
            if content.contains("ChannelAdapter for") {
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
            // Scan every .rs one level under src/<name>/, keyed by the
            // directory name — not just mod.rs, so an adapter split
            // into src/<name>/adapter.rs is still caught.
            if let Some(base) = path.file_name().and_then(|s| s.to_str()).map(String::from) {
                if let Ok(sub) = std::fs::read_dir(&path) {
                    for e in sub {
                        let p = e?.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                            let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("mod.rs");
                            let rel = format!("crates/librefang-channels/src/{base}/{fname}");
                            check(&base, &p, rel);
                        }
                    }
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
             in-process adapter requires explicit maintainer approval: \
             add its basename to \
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
        // A process-wide atomic counter guarantees uniqueness under
        // parallel `cargo test` — wall-clock nanos alone can collide
        // across threads and let one test's files contaminate another.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("lf-chanpol-{}-{seq}", std::process::id()));
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

    #[test]
    fn subdir_non_mod_adapter_is_rejected() {
        // src/<name>/adapter.rs (not mod.rs) must also be caught.
        let t = make_tree("ok\n");
        let sub = src(&t).join("sneaky");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("mod.rs"), "mod adapter;\n").unwrap();
        fs::write(sub.join("adapter.rs"), "impl ChannelAdapter for S {}").unwrap();
        let err = check_channel_policy(&t.0).unwrap_err().to_string();
        assert!(err.contains("sneaky/adapter.rs"), "got: {err}");
    }

    #[test]
    fn generic_impl_is_caught() {
        // `impl<T> ChannelAdapter for X` has no literal
        // `impl ChannelAdapter`; the `ChannelAdapter for` needle still
        // catches it.
        let t = make_tree("ok\n");
        fs::write(
            src(&t).join("gen.rs"),
            "impl<T: Send> ChannelAdapter for Wrap<T> {}",
        )
        .unwrap();
        let err = check_channel_policy(&t.0).unwrap_err().to_string();
        assert!(err.contains("gen.rs"), "got: {err}");
    }

    #[test]
    fn real_repo_tree_is_green() {
        // Guards against allowlist drift: the committed allowlist must
        // cover every in-process adapter in the actual tree. Fails if
        // someone lands a new adapter (or deletes one) without updating
        // channels-allowlist.txt. CARGO_MANIFEST_DIR == <repo>/xtask.
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        if let Err(e) = check_channel_policy(&repo_root) {
            panic!(
                "sidecar-first policy violation in the committed tree \
                 (NOT a flaky test): {e}\n\n\
                 A new in-process channel adapter was added without \
                 going through a sidecar, or an allowlisted module was \
                 deleted without removing its line. Fix the adapter \
                 (make it a sidecar) or update \
                 crates/librefang-channels/src/channels-allowlist.txt."
            );
        }
    }
}
