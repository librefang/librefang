//! Local mirror of the `changes` job in `.github/workflows/ci.yml` (#3296).
//!
//! Lets a developer ask "given my current branch, what would CI run?"
//! without having to push and wait. Same regex and routing rules as
//! `ci.yml`, kept structurally close so reviewers can diff the two when
//! either side changes.
//!
//! Usage:
//!   cargo xtask check-changed                       # plan against origin/main
//!   cargo xtask check-changed --from main           # plan against local main
//!   cargo xtask check-changed --json                # machine-readable
//!   cargo xtask check-changed --run check           # actually run cargo check
//!   cargo xtask check-changed --run check,clippy    # check then clippy
//!
//! `--run` only invokes the requested kinds against the affected crate set
//! (or the workspace if `full_run`/`full_test` is true). When no `--run`
//! is given the command exits 0 after printing the plan — useful as a
//! pre-commit dry-run.

use clap::Parser;
use std::collections::BTreeSet;
use std::process::Command;

use crate::common::repo_root;

#[derive(Parser, Debug)]
pub struct CheckChangedArgs {
    /// Compare HEAD against this revision (defaults to `origin/main`).
    /// Use `HEAD~1` for "just the last commit".
    #[arg(long, default_value = "origin/main")]
    pub from: String,

    /// Emit machine-readable JSON instead of the human summary.
    #[arg(long)]
    pub json: bool,

    /// Comma-separated cargo lanes to run against the affected crate set:
    /// `check`, `clippy`, `test`. Workspace-wide when `full_run` /
    /// `full_test` is true; selective otherwise. Defaults to none — just
    /// prints the plan.
    #[arg(long, value_delimiter = ',')]
    pub run: Vec<String>,
}

#[derive(Debug, Clone)]
struct Lanes {
    rust: bool,
    docs: bool,
    ci: bool,
    install: bool,
    workspace_cargo: bool,
    xtask_src: bool,
}

#[derive(Debug, Clone)]
struct Decision {
    value: bool,
    reason: &'static str,
}

#[derive(Debug, Clone)]
struct Plan {
    lanes: Lanes,
    full_run: Decision,
    full_test: Decision,
    crates: BTreeSet<String>,
    files: Vec<String>,
}

fn changed_files(from: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // `<from>...HEAD` is the merge-base diff (only commits unique to HEAD),
    // matching what GitHub reports for a PR.
    let range = format!("{from}...HEAD");
    let output = Command::new("git")
        .args(["diff", "--name-only", &range])
        .current_dir(repo_root())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

fn detect_lanes(changed: &[String]) -> Lanes {
    // Same regexes as ci.yml's `Compute diff and route` step. Keep these
    // identical to the shell version — drift would silently make the local
    // command lie about CI behaviour.
    let rust = regex::Regex::new(r"^(crates/|Cargo\.(toml|lock)$|xtask/|openapi\.json$|sdk/)")
        .expect("static regex");
    let docs = regex::Regex::new(r"^(docs/|.*\.md$)").expect("static regex");
    let ci = regex::Regex::new(r"^\.github/workflows/").expect("static regex");
    let install = regex::Regex::new(
        r"^web/public/install\.(sh|ps1)$|^scripts/tests/install_sh_test\.sh$",
    )
    .expect("static regex");
    let workspace_cargo = regex::Regex::new(r"^Cargo\.(toml|lock)$").expect("static regex");
    let xtask_src = regex::Regex::new(r"^xtask/").expect("static regex");
    let any = |re: &regex::Regex| changed.iter().any(|p| re.is_match(p));
    Lanes {
        rust: any(&rust),
        docs: any(&docs),
        ci: any(&ci),
        install: any(&install),
        workspace_cargo: any(&workspace_cargo),
        xtask_src: any(&xtask_src),
    }
}

/// Mirrors the `full_run` decision in ci.yml: build + clippy fan-out.
/// Push-to-main is the CI-only "sanity check before merge"; we don't model
/// it here because the local workflow is always PR-equivalent.
fn decide_full_run(lanes: &Lanes) -> Decision {
    if lanes.ci {
        Decision { value: true, reason: "CI workflow changed" }
    } else if lanes.workspace_cargo {
        Decision { value: true, reason: "workspace Cargo.toml/Cargo.lock changed" }
    } else if lanes.xtask_src {
        Decision { value: true, reason: "xtask source changed" }
    } else {
        Decision { value: false, reason: "selective" }
    }
}

/// Mirrors the `full_test` decision: strictly narrower than `full_run`.
/// Workspace dep / lints drift can ripple anywhere, so it's the only PR-time
/// trigger for re-running the full nextest matrix.
fn decide_full_test(lanes: &Lanes) -> Decision {
    if lanes.workspace_cargo {
        Decision { value: true, reason: "workspace Cargo.toml/Cargo.lock changed" }
    } else {
        Decision { value: false, reason: "selective" }
    }
}

fn affected_crates(changed: &[String], lanes: &Lanes) -> BTreeSet<String> {
    // Direct: `crates/<name>/...` → `<name>`.
    let mut set: BTreeSet<String> = changed
        .iter()
        .filter_map(|p| p.strip_prefix("crates/"))
        .filter_map(|tail| tail.split('/').next())
        .map(|s| s.to_string())
        .collect();
    // xtask isn't under `crates/`; pull it in explicitly when its source changes
    // so the selective lane runs `cargo nextest -p xtask`.
    if lanes.xtask_src {
        set.insert("xtask".to_string());
    }
    // Schema-mirror rule: librefang-types changes can break the
    // `kernel_config_schema_matches_golden_fixture` golden in librefang-api.
    if set.contains("librefang-types") {
        set.insert("librefang-api".to_string());
    }
    set
}

fn build_plan(from: &str) -> Result<Plan, Box<dyn std::error::Error>> {
    let files = changed_files(from)?;
    let lanes = detect_lanes(&files);
    let full_run = decide_full_run(&lanes);
    let full_test = decide_full_test(&lanes);
    let crates = affected_crates(&files, &lanes);
    Ok(Plan { lanes, full_run, full_test, crates, files })
}

fn print_human(plan: &Plan) {
    println!("Changed files: {}", plan.files.len());
    if plan.files.is_empty() {
        println!("  (none — branch already merged or `--from` is HEAD)");
    } else {
        for f in &plan.files {
            println!("  {f}");
        }
    }
    println!();
    println!("Lanes:");
    println!("  rust            = {}", plan.lanes.rust);
    println!("  docs            = {}", plan.lanes.docs);
    println!("  ci              = {}", plan.lanes.ci);
    println!("  install         = {}", plan.lanes.install);
    println!("  workspace_cargo = {}", plan.lanes.workspace_cargo);
    println!("  xtask_src       = {}", plan.lanes.xtask_src);
    println!();
    println!(
        "full_run  = {} ({})",
        plan.full_run.value, plan.full_run.reason
    );
    println!(
        "full_test = {} ({})",
        plan.full_test.value, plan.full_test.reason
    );
    println!();
    if plan.crates.is_empty() {
        println!("Affected crates: <none>");
    } else {
        let joined = plan.crates.iter().cloned().collect::<Vec<_>>().join(" ");
        println!("Affected crates: {joined}");
    }
}

fn print_json(plan: &Plan) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::json!({
        "files": plan.files,
        "lanes": {
            "rust": plan.lanes.rust,
            "docs": plan.lanes.docs,
            "ci": plan.lanes.ci,
            "install": plan.lanes.install,
            "workspace_cargo": plan.lanes.workspace_cargo,
            "xtask_src": plan.lanes.xtask_src,
        },
        "full_run": { "value": plan.full_run.value, "reason": plan.full_run.reason },
        "full_test": { "value": plan.full_test.value, "reason": plan.full_test.reason },
        "crates": plan.crates.iter().collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

fn run_cargo_check(plan: &Plan) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(repo_root());
    if plan.full_run.value {
        cmd.args(["check", "--workspace", "--lib"]);
        println!("→ cargo check --workspace --lib");
    } else if plan.crates.is_empty() {
        println!("→ cargo check skipped (no affected crates)");
        return Ok(());
    } else {
        cmd.arg("check");
        for c in &plan.crates {
            cmd.args(["-p", c]);
        }
        cmd.arg("--lib");
        println!(
            "→ cargo check {} --lib",
            plan.crates
                .iter()
                .map(|c| format!("-p {c}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("cargo check failed".into());
    }
    Ok(())
}

fn run_cargo_clippy(plan: &Plan) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(repo_root());
    if plan.full_run.value {
        cmd.args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]);
        println!("→ cargo clippy --workspace --all-targets -- -D warnings");
    } else if plan.crates.is_empty() {
        println!("→ cargo clippy skipped (no affected crates)");
        return Ok(());
    } else {
        cmd.arg("clippy");
        for c in &plan.crates {
            cmd.args(["-p", c]);
        }
        cmd.args(["--all-targets", "--", "-D", "warnings"]);
        println!(
            "→ cargo clippy {} --all-targets -- -D warnings",
            plan.crates
                .iter()
                .map(|c| format!("-p {c}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("cargo clippy failed".into());
    }
    Ok(())
}

fn run_cargo_test(plan: &Plan) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(repo_root());
    if plan.full_test.value {
        cmd.args(["test", "--workspace"]);
        println!("→ cargo test --workspace");
    } else if plan.crates.is_empty() {
        println!("→ cargo test skipped (no affected crates)");
        return Ok(());
    } else {
        cmd.arg("test");
        for c in &plan.crates {
            cmd.args(["-p", c]);
        }
        println!(
            "→ cargo test {}",
            plan.crates
                .iter()
                .map(|c| format!("-p {c}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err("cargo test failed".into());
    }
    Ok(())
}

pub fn run(args: CheckChangedArgs) -> Result<(), Box<dyn std::error::Error>> {
    let plan = build_plan(&args.from)?;

    if args.json {
        print_json(&plan)?;
    } else {
        print_human(&plan);
    }

    for kind in &args.run {
        match kind.trim() {
            "" => continue,
            "check" => run_cargo_check(&plan)?,
            "clippy" => run_cargo_clippy(&plan)?,
            "test" => run_cargo_test(&plan)?,
            other => return Err(format!("unknown --run kind: {other}").into()),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lanes_from(paths: &[&str]) -> Lanes {
        let v: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        detect_lanes(&v)
    }

    #[test]
    fn detects_rust_via_crate_path() {
        let l = lanes_from(&["crates/librefang-kernel/src/foo.rs"]);
        assert!(l.rust);
        assert!(!l.docs);
        assert!(!l.ci);
        assert!(!l.workspace_cargo);
        assert!(!l.xtask_src);
    }

    #[test]
    fn detects_xtask_src_independent_of_workspace_cargo() {
        let l = lanes_from(&["xtask/src/check_changed.rs"]);
        assert!(l.rust);
        assert!(l.xtask_src);
        assert!(!l.workspace_cargo);
    }

    #[test]
    fn workspace_cargo_does_not_imply_xtask_src() {
        let l = lanes_from(&["Cargo.toml"]);
        assert!(l.rust);
        assert!(l.workspace_cargo);
        assert!(!l.xtask_src);
    }

    #[test]
    fn ci_workflow_paths_flag_ci_lane() {
        let l = lanes_from(&[".github/workflows/ci.yml"]);
        assert!(l.ci);
        assert!(!l.rust);
        assert!(!l.workspace_cargo);
    }

    #[test]
    fn install_paths_flag_install_lane() {
        let l = lanes_from(&[
            "web/public/install.sh",
            "scripts/tests/install_sh_test.sh",
        ]);
        assert!(l.install);
        assert!(!l.rust);
    }

    #[test]
    fn docs_paths_flag_docs_lane() {
        let l = lanes_from(&["docs/architecture/foo.md", "README.md"]);
        assert!(l.docs);
        assert!(!l.rust);
    }

    #[test]
    fn openapi_and_sdk_count_as_rust() {
        let l = lanes_from(&["openapi.json", "sdk/python/foo.py"]);
        assert!(l.rust);
    }

    #[test]
    fn full_run_triggers_for_ci_workspace_cargo_xtask() {
        for paths in &[
            vec![".github/workflows/ci.yml"],
            vec!["Cargo.toml"],
            vec!["xtask/src/main.rs"],
        ] {
            let v: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
            let lanes = detect_lanes(&v);
            assert!(decide_full_run(&lanes).value, "expected full_run for {paths:?}");
        }
    }

    #[test]
    fn full_run_does_not_trigger_for_pure_crate_change() {
        let l = lanes_from(&["crates/librefang-runtime/src/foo.rs"]);
        assert!(!decide_full_run(&l).value);
    }

    #[test]
    fn full_test_strictly_narrower_than_full_run() {
        // CI-only / xtask-only changes do NOT trigger full_test.
        let l_ci = lanes_from(&[".github/workflows/ci.yml"]);
        assert!(decide_full_run(&l_ci).value);
        assert!(!decide_full_test(&l_ci).value);

        let l_xtask = lanes_from(&["xtask/src/main.rs"]);
        assert!(decide_full_run(&l_xtask).value);
        assert!(!decide_full_test(&l_xtask).value);

        // Workspace Cargo flips both.
        let l_cargo = lanes_from(&["Cargo.lock"]);
        assert!(decide_full_run(&l_cargo).value);
        assert!(decide_full_test(&l_cargo).value);
    }

    #[test]
    fn affected_crates_extracts_direct_membership() {
        let files: Vec<String> = vec![
            "crates/librefang-kernel/src/mod.rs".into(),
            "crates/librefang-api/src/server.rs".into(),
            "README.md".into(),
        ];
        let lanes = detect_lanes(&files);
        let crates = affected_crates(&files, &lanes);
        assert_eq!(
            crates,
            ["librefang-api", "librefang-kernel"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
    }

    #[test]
    fn affected_crates_pulls_xtask_when_xtask_src_changed() {
        let files: Vec<String> = vec!["xtask/src/main.rs".into()];
        let lanes = detect_lanes(&files);
        let crates = affected_crates(&files, &lanes);
        assert!(crates.contains("xtask"));
    }

    #[test]
    fn affected_crates_schema_mirror_pulls_api_in_for_types_change() {
        let files: Vec<String> = vec!["crates/librefang-types/src/lib.rs".into()];
        let lanes = detect_lanes(&files);
        let crates = affected_crates(&files, &lanes);
        assert!(crates.contains("librefang-types"));
        assert!(
            crates.contains("librefang-api"),
            "schema-mirror rule should pull api in for a types-only change"
        );
    }
}
