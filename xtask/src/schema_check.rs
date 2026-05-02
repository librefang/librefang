//! Schema drift detection.
//!
//! Hashes canonical schema artifacts (OpenAPI spec, sample config) and
//! compares them against committed `.sha256` baselines. CI runs
//! `schema-check check` to fail when a schema changes without the baseline
//! being regenerated; contributors run `schema-check gen` to refresh
//! baselines after an intentional schema change.
//!
//! See issue #3300.

use crate::common::repo_root;
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
pub struct SchemaCheckArgs {
    #[command(subcommand)]
    pub command: SchemaCheckCmd,
}

#[derive(Subcommand, Debug)]
pub enum SchemaCheckCmd {
    /// Regenerate the committed sha256 baselines from the current schema files.
    Gen,
    /// Verify schema files match the committed baselines (fails on drift).
    Check,
}

/// One tracked schema surface: a source artifact and its baseline file.
struct Surface {
    /// Human-readable label for log output.
    label: &'static str,
    /// Source file relative to the workspace root.
    source: &'static str,
    /// Baseline sha256 file relative to the workspace root.
    baseline: &'static str,
}

const SURFACES: &[Surface] = &[
    Surface {
        label: "openapi",
        source: "openapi.json",
        baseline: "xtask/baselines/openapi.sha256",
    },
    Surface {
        label: "config",
        source: "librefang.toml.example",
        baseline: "xtask/baselines/config.sha256",
    },
];

fn hash_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn resolve(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

fn run_gen(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for s in SURFACES {
        let src = resolve(root, s.source);
        let baseline = resolve(root, s.baseline);
        let digest = hash_file(&src)?;
        if let Some(parent) = baseline.parent() {
            fs::create_dir_all(parent)?;
        }
        // Format: "<sha256>  <relative-source-path>\n" (matches `sha256sum`).
        let line = format!("{digest}  {}\n", s.source);
        fs::write(&baseline, &line)?;
        println!("  wrote {} ({})", s.baseline, s.label);
    }
    println!("Schema baselines regenerated.");
    Ok(())
}

fn parse_baseline(content: &str) -> Option<&str> {
    content.split_whitespace().next()
}

fn run_check(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut drifted = Vec::new();
    for s in SURFACES {
        let src = resolve(root, s.source);
        let baseline = resolve(root, s.baseline);

        if !src.exists() {
            return Err(format!(
                "schema source missing: {} (run `cargo xtask codegen` first?)",
                src.display()
            )
            .into());
        }
        if !baseline.exists() {
            return Err(format!(
                "baseline missing: {} (run `cargo xtask schema-check gen`)",
                baseline.display()
            )
            .into());
        }

        let actual = hash_file(&src)?;
        let baseline_content = fs::read_to_string(&baseline)?;
        let expected = parse_baseline(&baseline_content)
            .ok_or_else(|| format!("malformed baseline: {}", baseline.display()))?;

        if actual == expected {
            println!("  ok      {} ({})", s.label, &actual[..12]);
        } else {
            println!(
                "  DRIFT   {}: expected {}, got {}",
                s.label,
                &expected[..expected.len().min(12)],
                &actual[..12]
            );
            drifted.push(s.label);
        }
    }

    if !drifted.is_empty() {
        return Err(format!(
            "schema drift detected in: {}. Regenerate the schema (e.g. `cargo xtask codegen`) \
             then run `cargo xtask schema-check gen` to update baselines.",
            drifted.join(", ")
        )
        .into());
    }

    println!("All schema baselines match.");
    Ok(())
}

pub fn run(args: SchemaCheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    match args.command {
        SchemaCheckCmd::Gen => run_gen(&root),
        SchemaCheckCmd::Check => run_check(&root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_baseline_handles_sha256sum_format() {
        let line = "abc123  openapi.json\n";
        assert_eq!(parse_baseline(line), Some("abc123"));
    }

    #[test]
    fn parse_baseline_handles_bare_hash() {
        assert_eq!(parse_baseline("deadbeef\n"), Some("deadbeef"));
    }

    #[test]
    fn parse_baseline_rejects_empty() {
        assert_eq!(parse_baseline(""), None);
    }

    #[test]
    fn hash_file_is_stable() {
        let dir = std::env::temp_dir().join("librefang-schema-check-test");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("sample.txt");
        fs::write(&p, b"hello").unwrap();
        let h = hash_file(&p).unwrap();
        // sha256("hello")
        assert_eq!(
            h,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
