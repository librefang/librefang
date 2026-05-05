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
use std::sync::OnceLock;

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
    // Two-dot diff to match ci.yml's `git diff --name-only "$BASE_SHA" "$HEAD_SHA"`.
    // Three-dot (`{from}...HEAD`) silently drops files that main moved past
    // since the branch forked, which would make the local plan disagree with
    // what CI actually runs.
    let output = Command::new("git")
        .args(["diff", "--name-only", from, "HEAD"])
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

struct LaneRegexes {
    rust: regex::Regex,
    docs: regex::Regex,
    ci: regex::Regex,
    install: regex::Regex,
    workspace_cargo: regex::Regex,
    xtask_src: regex::Regex,
}

fn lane_regexes() -> &'static LaneRegexes {
    // Same regexes as ci.yml's `Compute diff and route` step. Keep these
    // identical to the shell version — drift would silently make the local
    // command lie about CI behaviour.
    static REGEXES: OnceLock<LaneRegexes> = OnceLock::new();
    REGEXES.get_or_init(|| LaneRegexes {
        rust: regex::Regex::new(r"^(crates/|Cargo\.(toml|lock)$|xtask/|openapi\.json$|sdk/)")
            .expect("static regex"),
        docs: regex::Regex::new(r"^(docs/|.*\.md$)").expect("static regex"),
        ci: regex::Regex::new(r"^\.github/workflows/").expect("static regex"),
        install: regex::Regex::new(
            r"^web/public/install\.(sh|ps1)$|^scripts/tests/install_sh_test\.sh$",
        )
        .expect("static regex"),
        workspace_cargo: regex::Regex::new(r"^Cargo\.(toml|lock)$").expect("static regex"),
        xtask_src: regex::Regex::new(r"^xtask/").expect("static regex"),
    })
}

fn detect_lanes(changed: &[String]) -> Lanes {
    let r = lane_regexes();
    let any = |re: &regex::Regex| changed.iter().any(|p| re.is_match(p));
    Lanes {
        rust: any(&r.rust),
        docs: any(&r.docs),
        ci: any(&r.ci),
        install: any(&r.install),
        workspace_cargo: any(&r.workspace_cargo),
        xtask_src: any(&r.xtask_src),
    }
}

/// Mirrors the `full_run` decision in ci.yml: build + clippy fan-out.
/// Push-to-main is the CI-only "sanity check before merge"; we don't model
/// it here because the local workflow is always PR-equivalent.
fn decide_full_run(lanes: &Lanes) -> Decision {
    if lanes.ci {
        Decision {
            value: true,
            reason: "CI workflow changed",
        }
    } else if lanes.workspace_cargo {
        Decision {
            value: true,
            reason: "workspace Cargo.toml/Cargo.lock changed",
        }
    } else if lanes.xtask_src {
        Decision {
            value: true,
            reason: "xtask source changed",
        }
    } else {
        Decision {
            value: false,
            reason: "selective",
        }
    }
}

/// Mirrors the `full_test` decision: strictly narrower than `full_run`.
/// Workspace dep / lints drift can ripple anywhere, so it's the only PR-time
/// trigger for re-running the full nextest matrix.
fn decide_full_test(lanes: &Lanes) -> Decision {
    if lanes.workspace_cargo {
        Decision {
            value: true,
            reason: "workspace Cargo.toml/Cargo.lock changed",
        }
    } else {
        Decision {
            value: false,
            reason: "selective",
        }
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
    Ok(Plan {
        lanes,
        full_run,
        full_test,
        crates,
        files,
    })
}

fn print_human(plan: &Plan) {
    println!("Changed files: {}", plan.files.len());
    if plan.files.is_empty() {
        println!("  (none -- branch already merged or `--from` is HEAD)");
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

/// Build args for `cargo check`. `None` = nothing to do (selective with no
/// affected crates). Selective mode does NOT pass `--lib` because `-p X --lib`
/// errors with "no library targets found in package X" for binary-only crates
/// (librefang-cli, librefang-desktop); the same gotcha CI's selective `cargo
/// build` step explicitly works around. `--workspace --lib` is fine because
/// there `--lib` is a workspace-wide filter, not a per-package selector.
fn build_check_args(plan: &Plan) -> Option<Vec<String>> {
    if plan.full_run.value {
        Some(
            ["check", "--workspace", "--lib"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    } else if plan.crates.is_empty() {
        None
    } else {
        let mut v = vec!["check".to_string()];
        for c in &plan.crates {
            v.push("-p".into());
            v.push(c.clone());
        }
        Some(v)
    }
}

/// Build args for `cargo clippy`. Selective mode passes `--all-features`
/// to match ci.yml's selective clippy step (`cargo clippy $PFLAGS
/// --all-targets --all-features -- -D warnings`); without it a feature-gated
/// lint failure passes locally but trips CI. `--workspace` mode mirrors the
/// `cargo xtask ci` invocation, which doesn't pass `--all-features`.
fn build_clippy_args(plan: &Plan) -> Option<Vec<String>> {
    if plan.full_run.value {
        Some(
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        )
    } else if plan.crates.is_empty() {
        None
    } else {
        let mut v = vec!["clippy".to_string()];
        for c in &plan.crates {
            v.push("-p".into());
            v.push(c.clone());
        }
        v.extend(
            ["--all-targets", "--all-features", "--", "-D", "warnings"]
                .iter()
                .map(|s| s.to_string()),
        );
        Some(v)
    }
}

/// Build args for `cargo test`. `full_test=true` → workspace; otherwise
/// per-crate. The repo-wide guidance forbids unscoped `cargo test` because of
/// shared `target/` contention with the daemon, so the workspace branch is
/// only reached when CI itself would do the same (workspace Cargo manifest
/// changed); see ci.yml `full_test` decision.
fn build_test_args(plan: &Plan) -> Option<Vec<String>> {
    if plan.full_test.value {
        Some(
            ["test", "--workspace"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    } else if plan.crates.is_empty() {
        None
    } else {
        let mut v = vec!["test".to_string()];
        for c in &plan.crates {
            v.push("-p".into());
            v.push(c.clone());
        }
        Some(v)
    }
}

fn run_cargo(label: &str, args: Option<Vec<String>>) -> Result<(), Box<dyn std::error::Error>> {
    let Some(args) = args else {
        println!("-> cargo {label} skipped (no affected crates)");
        return Ok(());
    };
    println!("-> cargo {}", args.join(" "));
    let status = Command::new("cargo")
        .args(&args)
        .current_dir(repo_root())
        .status()?;
    if !status.success() {
        return Err(format!("cargo {label} failed").into());
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
            "check" => run_cargo("check", build_check_args(&plan))?,
            "clippy" => run_cargo("clippy", build_clippy_args(&plan))?,
            "test" => run_cargo("test", build_test_args(&plan))?,
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

    fn plan_from(paths: &[&str]) -> Plan {
        let files: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        let lanes = detect_lanes(&files);
        let full_run = decide_full_run(&lanes);
        let full_test = decide_full_test(&lanes);
        let crates = affected_crates(&files, &lanes);
        Plan {
            lanes,
            full_run,
            full_test,
            crates,
            files,
        }
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
        let l = lanes_from(&["web/public/install.sh", "scripts/tests/install_sh_test.sh"]);
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
            assert!(
                decide_full_run(&lanes).value,
                "expected full_run for {paths:?}"
            );
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

    // ── command-line shape tests ─────────────────────────────────────────
    // These pin the exact argv we hand to `cargo` so a future edit can't
    // silently reintroduce `-p X --lib` (errors on bin-only crates) or drop
    // `--all-features` from selective clippy (lets feature-gated lints pass
    // locally but fail CI).

    #[test]
    fn selective_check_does_not_pass_lib() {
        // Bin-only crate should NOT get `-p librefang-cli --lib` (cargo
        // errors with "no library targets found in package librefang-cli").
        let plan = plan_from(&["crates/librefang-cli/src/main.rs"]);
        let args = build_check_args(&plan).expect("should produce args");
        assert!(
            !args.iter().any(|a| a == "--lib"),
            "selective check must not pass --lib: {args:?}"
        );
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"librefang-cli".to_string()));
    }

    #[test]
    fn full_run_check_uses_workspace_lib() {
        let plan = plan_from(&[".github/workflows/ci.yml"]);
        let args = build_check_args(&plan).expect("should produce args");
        assert_eq!(args, vec!["check", "--workspace", "--lib"]);
    }

    #[test]
    fn selective_clippy_passes_all_features() {
        // ci.yml's selective lane runs `cargo clippy $PFLAGS --all-targets
        // --all-features -- -D warnings`. Drift here means a feature-gated
        // lint can pass locally but fail CI.
        let plan = plan_from(&["crates/librefang-runtime/src/foo.rs"]);
        let args = build_clippy_args(&plan).expect("should produce args");
        assert!(
            args.iter().any(|a| a == "--all-features"),
            "selective clippy must pass --all-features: {args:?}"
        );
        assert!(args.iter().any(|a| a == "--all-targets"));
        assert!(args.windows(2).any(|w| w == ["--", "-D"]));
    }

    #[test]
    fn full_run_clippy_workspace_shape() {
        let plan = plan_from(&[".github/workflows/ci.yml"]);
        let args = build_clippy_args(&plan).expect("should produce args");
        assert_eq!(
            args,
            vec![
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings"
            ]
        );
    }

    #[test]
    fn empty_plan_yields_no_cargo_args() {
        let plan = plan_from(&[]);
        assert!(build_check_args(&plan).is_none());
        assert!(build_clippy_args(&plan).is_none());
        assert!(build_test_args(&plan).is_none());
    }

    #[test]
    fn selective_test_per_crate() {
        let plan = plan_from(&["crates/librefang-kernel/src/mod.rs"]);
        let args = build_test_args(&plan).expect("should produce args");
        assert_eq!(args, vec!["test", "-p", "librefang-kernel"]);
    }

    #[test]
    fn workspace_cargo_change_runs_full_test() {
        let plan = plan_from(&["Cargo.lock"]);
        let args = build_test_args(&plan).expect("should produce args");
        assert_eq!(args, vec!["test", "--workspace"]);
    }
}
