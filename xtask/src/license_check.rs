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
        // Case-insensitive: `denied` comes straight from `--deny` / the CLI default and is never itself run through SPDX parsing, so a custom deny entry with different casing than the canonical SPDX id (e.g. `gpl-3.0-only` vs `GPL-3.0-only`) must still match rather than silently letting the license through.
        !denied
            .iter()
            .any(|denied_id| denied_id.eq_ignore_ascii_case(id.name))
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

fn check_metadata_license_policy(
    metadata: &serde_json::Value,
    denied: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
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

fn check_rust_licenses_with_tools<RunCargoDeny, LoadMetadata>(
    denied: &[&str],
    cargo_deny_available: bool,
    run_cargo_deny: RunCargoDeny,
    load_metadata: LoadMetadata,
) -> Result<(), Box<dyn std::error::Error>>
where
    RunCargoDeny: FnOnce() -> Result<(), Box<dyn std::error::Error>>,
    LoadMetadata: FnOnce() -> Result<serde_json::Value, Box<dyn std::error::Error>>,
{
    if cargo_deny_available {
        println!("Using cargo-deny...");
        run_cargo_deny()?;
        println!("Applying the explicit denied-license policy...");
    } else {
        println!("cargo-deny not found, using cargo metadata fallback...");
    }

    let metadata = load_metadata()?;
    check_metadata_license_policy(&metadata, denied)
}

fn check_cargo_deny(root: &Path, denied: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let cargo_deny_available = matches!(
        Command::new("cargo").args(["deny", "--version"]).output(),
        Ok(output) if output.status.success()
    );

    check_rust_licenses_with_tools(
        denied,
        cargo_deny_available,
        || {
            let status = Command::new("cargo")
                .args(["deny", "check", "licenses"])
                .current_dir(root)
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err("cargo deny check failed".into())
            }
        },
        || load_cargo_metadata(root),
    )
}

fn check_web_license_policy(
    report: &serde_json::Value,
    denied: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let groups = report
        .as_object()
        .ok_or("pnpm license report was not a JSON object")?;
    let mut violations = Vec::new();
    let mut unverified = Vec::new();
    let mut checked = 0;

    for (group_license, packages) in groups {
        if group_license.trim().is_empty() {
            return Err("pnpm license report contained an empty license group".into());
        }
        let packages = packages
            .as_array()
            .ok_or_else(|| format!("pnpm license group {group_license:?} was not an array"))?;
        for (index, package) in packages.iter().enumerate() {
            let package = package.as_object().ok_or_else(|| {
                format!("pnpm license group {group_license:?} entry {index} was not an object")
            })?;
            let name = package
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "pnpm license group {group_license:?} entry {index} had no package name"
                    )
                })?;
            let versions = match package.get("versions") {
                None => String::new(),
                Some(serde_json::Value::Array(values)) => {
                    let mut versions = Vec::with_capacity(values.len());
                    for version in values {
                        let version = version
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .ok_or_else(|| {
                                format!("pnpm package {name:?} had an invalid version")
                            })?;
                        versions.push(version);
                    }
                    versions.join(", ")
                }
                Some(_) => return Err(format!("pnpm package {name:?} had invalid versions").into()),
            };
            let label = if versions.is_empty() {
                name.to_owned()
            } else {
                format!("{name}@{versions}")
            };
            let license = match package.get("license") {
                None => group_license.as_str(),
                Some(serde_json::Value::String(license)) => license.as_str(),
                Some(_) => {
                    return Err(format!("pnpm package {name:?} had an invalid license").into())
                }
            };
            checked += 1;

            if license.is_empty() || license == "UNKNOWN" {
                unverified.push(format!(
                    "  {label} — no license declared (manual review needed)"
                ));
                continue;
            }

            if has_commons_clause(license) {
                violations.push(format!(
                    "  {label} ({license}) — Commons Clause rider detected"
                ));
                continue;
            }

            match license_expression_is_denied(license, denied) {
                Ok(true) => {
                    violations.push(format!("  {label} ({license}) — denied SPDX expression"))
                }
                Ok(false) => {}
                Err(_) => unverified.push(format!(
                    "  {label} ({license}) — invalid or non-SPDX expression (manual review needed)"
                )),
            }
        }
    }

    println!("  Checked {checked} web packages");
    if !unverified.is_empty() {
        println!("  Unverified web licenses — manual review required:");
        for item in &unverified {
            println!("WARN {item}");
        }
    }

    if violations.is_empty() {
        println!("  No web license violations found.");
        Ok(())
    } else {
        println!("  Web license violations:");
        for violation in &violations {
            println!("{violation}");
        }
        Err(format!("{} web license violation(s) found", violations.len()).into())
    }
}

fn check_web_licenses(root: &Path, denied: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let web_dir = root.join("web");
    if !web_dir.join("package.json").exists() {
        println!("Skipping web license check (no web/package.json)");
        return Ok(());
    }

    let output = Command::new("pnpm")
        .args(["licenses", "list", "--json"])
        .current_dir(&web_dir)
        .output()
        .map_err(|error| format!("failed to run pnpm licenses list: {error}"))?;
    if !output.status.success() {
        return Err("pnpm licenses list failed".into());
    }

    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid pnpm license JSON: {error}"))?;
    check_web_license_policy(&report, denied)
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
        check_web_licenses(&root, &denied)?;
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

    #[test]
    fn deny_list_matches_regardless_of_case() {
        // `denied` is raw CLI input, never SPDX-parsed, while `id.name` is the canonical SPDX casing — a differently-cased custom `--deny` entry must still catch the license rather than silently passing it.
        assert!(license_expression_is_denied("GPL-3.0-only", &["gpl-3.0-only"]).unwrap());
        assert!(license_expression_is_denied("GPL-3.0-only", &["GPL-3.0-ONLY"]).unwrap());
    }

    #[test]
    fn custom_deny_policy_runs_after_successful_cargo_deny() {
        let cargo_deny_ran = std::cell::Cell::new(false);
        let metadata = serde_json::json!({
            "packages": [{
                "name": "custom-denied",
                "license": "MIT"
            }]
        });

        let result = check_rust_licenses_with_tools(
            &["MIT"],
            true,
            || {
                cargo_deny_ran.set(true);
                Ok(())
            },
            || Ok(metadata),
        );

        assert!(
            cargo_deny_ran.get(),
            "the repository cargo-deny policy must still run"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 license violation"),
            "a successful cargo-deny run must not bypass the custom deny list"
        );
    }

    #[test]
    fn cargo_deny_failure_stops_before_the_custom_policy() {
        let metadata_loaded = std::cell::Cell::new(false);
        let result = check_rust_licenses_with_tools(
            &["MIT"],
            true,
            || Err("cargo-deny rejected the graph".into()),
            || {
                metadata_loaded.set(true);
                Ok(serde_json::json!({ "packages": [] }))
            },
        );

        assert_eq!(
            result.unwrap_err().to_string(),
            "cargo-deny rejected the graph"
        );
        assert!(!metadata_loaded.get());
    }

    #[test]
    fn web_license_report_enforces_the_explicit_deny_list() {
        let report = serde_json::json!({
            "MIT": [{
                "name": "blocked-web-package",
                "versions": ["1.0.0"],
                "license": "MIT"
            }]
        });

        let result = check_web_license_policy(&report, &["MIT"]);

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 web license violation"),
            "pnpm's JSON report must be enforced rather than merely printed"
        );
    }

    #[test]
    fn web_license_report_respects_spdx_or_choices() {
        let report = serde_json::json!({
            "GPL-3.0-only OR MIT": [{
                "name": "dual-licensed-web-package",
                "versions": ["1.0.0"],
                "license": "GPL-3.0-only OR MIT"
            }]
        });

        check_web_license_policy(&report, &["GPL-3.0-only"]).unwrap();
    }

    #[test]
    fn malformed_web_license_report_fails_closed() {
        let result = check_web_license_policy(&serde_json::json!([]), &["MIT"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "pnpm license report was not a JSON object"
        );
    }

    #[test]
    fn malformed_web_package_entries_fail_closed() {
        for report in [
            serde_json::json!({ "MIT": [null] }),
            serde_json::json!({
                "MIT": [{
                    "name": "package",
                    "versions": "1.0.0",
                    "license": "MIT"
                }]
            }),
            serde_json::json!({
                "MIT": [{
                    "name": "package",
                    "versions": ["1.0.0"],
                    "license": []
                }]
            }),
        ] {
            assert!(check_web_license_policy(&report, &["GPL-3.0-only"]).is_err());
        }
    }
}
