use crate::common::repo_root;
use clap::Parser;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct DepsArgs {
    /// Run cargo audit for security vulnerabilities
    #[arg(long)]
    pub audit: bool,

    /// Run cargo outdated to check for updates
    #[arg(long)]
    pub outdated: bool,

    /// Include frontend (pnpm audit)
    #[arg(long)]
    pub web: bool,

    /// Ignore specific RUSTSEC advisories (can be repeated)
    #[arg(long = "ignore", value_name = "RUSTSEC_ID")]
    pub ignore_ids: Vec<String>,
}

fn has_command(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn has_cargo_subcommand(sub: &str) -> bool {
    Command::new("cargo")
        .args([sub, "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_cargo_audit(root: &Path, ignore_ids: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    if !has_cargo_subcommand("audit") {
        println!("  Installing cargo-audit...");
        let status = Command::new("cargo")
            .args(["install", "cargo-audit"])
            .status()?;
        if !status.success() {
            return Err("failed to install cargo-audit".into());
        }
    }

    println!("=== cargo audit ===");
    let mut cmd = Command::new("cargo");
    cmd.arg("audit").current_dir(root);
    for id in ignore_ids {
        cmd.args(["--ignore", id]);
    }
    let status = cmd.status()?;
    println!();

    Ok(status.success())
}

fn run_cargo_outdated(root: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    if !has_cargo_subcommand("outdated") {
        println!("  Installing cargo-outdated...");
        let status = Command::new("cargo")
            .args(["install", "cargo-outdated"])
            .status()?;
        if !status.success() {
            return Err("failed to install cargo-outdated".into());
        }
    }

    println!("=== cargo outdated ===");
    let status = Command::new("cargo")
        .args(["outdated", "--workspace", "--root-deps-only"])
        .current_dir(root)
        .status()?;
    println!();

    Ok(status.success())
}

/// npm retired the legacy audit endpoints (`/-/npm/v1/security/audits`,
/// `…/audits/quick`); they now answer HTTP 410 with a body telling callers to
/// migrate to the bulk advisory endpoint. pnpm v10 still calls the legacy path,
/// so `pnpm audit` exits non-zero even when there is no advisory to report.
/// Detect that failure mode so an audit-infrastructure outage does not read as a
/// dependency vulnerability. Real advisories still surface through the normal
/// non-zero exit and are NOT matched here.
fn is_audit_endpoint_retired(output: &str) -> bool {
    let out = output.to_ascii_lowercase();
    out.contains("err_pnpm_audit_bad_response")
        || out.contains("endpoint is being retired")
        || (out.contains("audit") && out.contains("responded with 410"))
}

/// Outcome of a single `pnpm audit` invocation, distinguishing a genuine
/// advisory finding from the audit service being unreachable.
enum PnpmAuditOutcome {
    /// pnpm reported no advisories (or the directory has no manifest).
    Clean,
    /// pnpm reported one or more advisories — a real dependency issue.
    Vulnerable,
    /// The audit could not run (retired endpoint / network); not a vulnerability.
    Skipped,
}

fn run_pnpm_audit(dir: &Path, label: &str) -> PnpmAuditOutcome {
    if !dir.join("package.json").exists() {
        return PnpmAuditOutcome::Clean;
    }

    println!("--- pnpm audit: {} ---", label);
    // --prod: devDependencies (vite, esbuild) never ship to users; their advisories are not runtime exposures.
    let result = Command::new("pnpm")
        .args(["audit", "--prod"])
        .current_dir(dir)
        .output();

    let output = match result {
        Ok(o) => o,
        Err(e) => {
            println!("  pnpm audit could not be spawned: {e}");
            println!();
            return PnpmAuditOutcome::Vulnerable;
        }
    };

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{combined}");
    println!();

    if output.status.success() {
        PnpmAuditOutcome::Clean
    } else if is_audit_endpoint_retired(&combined) {
        println!(
            "  ⚠ pnpm audit endpoint unavailable (npm retired the legacy audit API); \
             skipping — not counted as a vulnerability."
        );
        println!();
        PnpmAuditOutcome::Skipped
    } else {
        PnpmAuditOutcome::Vulnerable
    }
}

pub fn run(args: DepsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // If no flags set, run all
    let run_all = !args.audit && !args.outdated && !args.web;
    let mut issues = 0;

    if run_all || args.audit {
        match run_cargo_audit(&root, &args.ignore_ids) {
            Ok(true) => println!("Cargo audit: no vulnerabilities found"),
            Ok(false) => {
                println!("Cargo audit: vulnerabilities found!");
                issues += 1;
            }
            Err(e) => {
                println!("Cargo audit error: {}", e);
                issues += 1;
            }
        }
        println!();
    }

    if run_all || args.outdated {
        match run_cargo_outdated(&root) {
            Ok(_) => {}
            Err(e) => {
                println!("Cargo outdated error: {}", e);
                issues += 1;
            }
        }
    }

    if run_all || args.web {
        if has_command("pnpm") {
            println!("=== pnpm audit ===");
            let web_dirs = [
                (root.join("web"), "web"),
                (root.join("crates/librefang-api/dashboard"), "dashboard"),
                (root.join("docs"), "docs"),
            ];
            for (dir, label) in &web_dirs {
                match run_pnpm_audit(dir, label) {
                    PnpmAuditOutcome::Clean | PnpmAuditOutcome::Skipped => {}
                    PnpmAuditOutcome::Vulnerable => issues += 1,
                }
            }
        } else {
            println!("pnpm not found — skipping frontend audit");
        }
    }

    println!("=== Summary ===");
    if issues > 0 {
        println!("{} issue(s) found — review output above", issues);
        Err(format!("{} dependency issue(s) found", issues).into())
    } else {
        println!("All clean.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verbatim body npm returns for both retired audit endpoints (HTTP 410),
    // as surfaced by pnpm v10 in CI.
    const RETIRED_410: &str = "\
 ERR_PNPM_AUDIT_BAD_RESPONSE  The audit endpoint (at \
https://registry.npmjs.org/-/npm/v1/security/audits/quick) responded with 410: \
{\"error\":\"This endpoint is being retired. Use the bulk advisory endpoint instead. \
See the following docs for more info: https://api-docs.npmjs.com/#tag/Audit\"}. \
Fallback endpoint (at https://registry.npmjs.org/-/npm/v1/security/audits) responded \
with 410: {\"error\":\"This endpoint is being retired.\"}";

    #[test]
    fn retired_endpoint_410_is_classified_as_outage() {
        assert!(is_audit_endpoint_retired(RETIRED_410));
    }

    #[test]
    fn retired_detection_is_case_insensitive() {
        assert!(is_audit_endpoint_retired(&RETIRED_410.to_uppercase()));
    }

    #[test]
    fn only_the_fallback_endpoint_410_still_counts_as_outage() {
        // The single-endpoint variant (docs dir) has no "/quick" line.
        let single = " ERR_PNPM_AUDIT_BAD_RESPONSE  The audit endpoint (at \
https://registry.npmjs.org/-/npm/v1/security/audits) responded with 410: \
{\"error\":\"This endpoint is being retired.\"}";
        assert!(is_audit_endpoint_retired(single));
    }

    #[test]
    fn real_advisory_output_is_not_treated_as_outage() {
        // A genuine `pnpm audit` finding must still fail the build.
        let advisory = "\
┌─────────────────────┬────────────────────────────────────────────────────────┐
│ high                │ Prototype Pollution in some-pkg                        │
├─────────────────────┼────────────────────────────────────────────────────────┤
│ Package             │ some-pkg                                               │
└─────────────────────┴────────────────────────────────────────────────────────┘
1 vulnerabilities found";
        assert!(!is_audit_endpoint_retired(advisory));
    }

    #[test]
    fn clean_output_is_not_treated_as_outage() {
        assert!(!is_audit_endpoint_retired("No known vulnerabilities found"));
    }

    #[test]
    fn unrelated_410_without_audit_context_is_not_matched() {
        // A 410 that is not about the audit endpoint must not be swallowed.
        assert!(!is_audit_endpoint_retired(
            "GET https://example.com/thing responded with 410"
        ));
    }
}
