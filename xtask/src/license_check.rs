use crate::common::repo_root;
use clap::Parser;
use std::path::Path;
use std::process::Command;

/// Licenses denied by default.  This list deliberately covers copyleft and
/// source-available licenses that are incompatible with commercial distribution
/// of a proprietary or permissively-licensed product.
///
/// The list is intentionally broad:
/// - GPL / LGPL (all versions and flavors)
/// - AGPL (all versions and flavors)
/// - SSPL (MongoDB server-side public license)
/// - BUSL (Business Source License — time-delayed open source)
///
/// Crates with `license = null` or `"UNKNOWN"` are flagged separately as
/// unverified rather than hard-blocked, because they often just have
/// non-SPDX license strings that need manual inspection.
const DEFAULT_DENIED_LICENSES: &str = concat!(
    "AGPL-3.0-only,AGPL-3.0-or-later,",
    "GPL-2.0,GPL-2.0-only,GPL-2.0-or-later,",
    "GPL-3.0,GPL-3.0-only,GPL-3.0-or-later,",
    "LGPL-2.0,LGPL-2.0-only,LGPL-2.0-or-later,",
    "LGPL-2.1,LGPL-2.1-only,LGPL-2.1-or-later,",
    "LGPL-3.0,LGPL-3.0-only,LGPL-3.0-or-later,",
    "SSPL-1.0,",
    "BUSL-1.1"
);

#[derive(Parser, Debug)]
pub struct LicenseCheckArgs {
    /// Check only Rust dependencies
    #[arg(long)]
    pub rust: bool,

    /// Check only web dependencies
    #[arg(long)]
    pub web: bool,

    /// Denied licenses (comma-separated).
    /// Defaults to GPL/LGPL/AGPL/SSPL/BUSL variants.
    #[arg(long, default_value = DEFAULT_DENIED_LICENSES)]
    pub deny: String,
}

/// Returns `true` if the license string contains any fragment that resembles the
/// "Commons Clause" rider (e.g. `"Commons Clause"` or `"Commons-Clause"`).
fn has_commons_clause(license: &str) -> bool {
    let lc = license.to_lowercase();
    lc.contains("commons clause") || lc.contains("commons-clause")
}

fn license_expression_is_denied(license: &str, denied: &[&str]) -> Result<bool, spdx::ParseError> {
    let expression = spdx::Expression::parse(license)?;
    let can_choose_permitted_terms = expression.evaluate(|requirement| {
        let Some(id) = requirement.license.id() else {
            return false;
        };
        !denied.iter().any(|denied_id| *denied_id == id.name)
    });
    Ok(!can_choose_permitted_terms)
}

fn load_cargo_metadata(root: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version=1"])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        return Err("cargo metadata failed".into());
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

fn check_cargo_deny(root: &Path, denied: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    // Try cargo-deny first
    let deny_check = Command::new("cargo").args(["deny", "--version"]).output();

    if deny_check.is_ok() && deny_check.unwrap().status.success() {
        println!("Using cargo-deny...");
        let status = Command::new("cargo")
            .args(["deny", "check", "licenses"])
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err("cargo deny check failed".into());
        }
        return Ok(());
    }

    // Fallback: use cargo metadata
    println!("cargo-deny not found, using cargo metadata fallback...");
    let metadata = load_cargo_metadata(root)?;
    let mut violations = Vec::new();
    let mut unverified = Vec::new();
    let mut checked = 0;

    if let Some(packages) = metadata["packages"].as_array() {
        for pkg in packages {
            let name = pkg["name"].as_str().unwrap_or("unknown");
            checked += 1;

            // `license` is null when the Cargo.toml field is absent.
            let license_opt = pkg["license"].as_str();
            let license = license_opt.unwrap_or("UNKNOWN");

            // Flag crates with no declared license for manual review.
            if license_opt.is_none() || license == "UNKNOWN" || license.is_empty() {
                unverified.push(format!(
                    "  {} — no license declared (manual review needed)",
                    name
                ));
                continue;
            }

            // Check for "Commons Clause" rider (often appended to Apache-2.0 etc.).
            if has_commons_clause(license) {
                violations.push(format!(
                    "  {} ({}) — Commons Clause rider detected",
                    name, license
                ));
                continue;
            }

            // Evaluate the full SPDX expression. An OR expression is accepted
            // when at least one branch avoids denied licenses; every term of
            // an AND expression must be permitted.
            match license_expression_is_denied(license, denied) {
                Ok(true) => {
                    violations.push(format!("  {} ({}) — denied SPDX expression", name, license));
                }
                Ok(false) => {}
                Err(_) => {
                    unverified.push(format!(
                        "  {} ({}) — invalid or non-SPDX expression (manual review needed)",
                        name, license
                    ));
                }
            }
        }
    }

    println!("  Checked {} Rust packages", checked);

    if !unverified.is_empty() {
        println!("  Unverified licenses — manual review required:");
        for u in &unverified {
            println!("WARN {}", u);
        }
    }

    if violations.is_empty() {
        println!("  No license violations found.");
    } else {
        println!("  License violations:");
        for v in &violations {
            println!("{}", v);
        }
        return Err(format!("{} license violation(s) found", violations.len()).into());
    }

    Ok(())
}

fn check_web_licenses(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let web_dir = root.join("web");
    if !web_dir.join("package.json").exists() {
        println!("Skipping web license check (no web/package.json)");
        return Ok(());
    }

    // Try pnpm licenses list
    let output = Command::new("pnpm")
        .args(["licenses", "list", "--json"])
        .current_dir(&web_dir)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            println!("  Web dependency licenses:");
            // Just report — pnpm licenses list shows the breakdown
            let lines: Vec<&str> = stdout.lines().take(20).collect();
            for line in lines {
                println!("    {}", line);
            }
            if stdout.lines().count() > 20 {
                println!("    ... (truncated)");
            }
        }
        _ => {
            println!("  pnpm licenses not available, skipping web license check");
        }
    }

    Ok(())
}

pub fn run(args: LicenseCheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let denied: Vec<&str> = args.deny.split(',').map(|s| s.trim()).collect();
    let check_all = !args.rust && !args.web;

    println!("License check");
    println!("  Denied: {}\n", args.deny);

    if check_all || args.rust {
        println!("=== Rust Dependencies ===");
        check_cargo_deny(&root, &denied)?;
        println!();
    }

    if check_all || args.web {
        println!("=== Web Dependencies ===");
        check_web_licenses(&root)?;
        println!();
    }

    println!("License check complete.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_metadata_fallback_includes_third_party_dependencies() {
        let metadata = load_cargo_metadata(&repo_root()).unwrap();
        let packages = metadata["packages"].as_array().unwrap();

        assert!(
            packages.iter().any(|pkg| pkg["source"].as_str().is_some()),
            "fallback metadata omitted every third-party dependency"
        );
    }

    #[test]
    fn spdx_or_expression_can_choose_a_permitted_license() {
        let denied = ["GPL-2.0-only"];
        assert!(!license_expression_is_denied("GPL-2.0-only OR BSD-3-Clause", &denied).unwrap());
    }

    #[test]
    fn spdx_and_expression_requires_every_license() {
        let denied = ["GPL-2.0-only"];
        assert!(license_expression_is_denied("MIT AND GPL-2.0-only", &denied).unwrap());
    }

    #[test]
    fn custom_license_refs_fail_closed_when_required() {
        assert!(license_expression_is_denied("MIT AND LicenseRef-Proprietary", &[]).unwrap());
    }
}
