//! Build-timing collection and regression tracking.
//!
//! `cargo xtask build-timings` runs `cargo build --workspace --timings`,
//! parses the generated HTML report's embedded `UNIT_DATA` array, and
//! writes a slim JSON snapshot to `bench-results/build-timings/<sha>.json`.
//!
//! `cargo xtask compare-build-timings` diffs the latest snapshot against
//! `bench-results/build-timings/baseline.json` and exits non-zero when any
//! crate's compile time has regressed by more than the configured threshold
//! (default 10%). The non-zero exit is intended to be wired into a weekly
//! CI job as a soft alert (`continue-on-error: true`), not a blocking gate.

use crate::common::repo_root;
use clap::Parser;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct BuildTimingsArgs {
    /// Skip the cargo build step and parse an existing report (debug aid).
    #[arg(long)]
    pub no_build: bool,

    /// Override the output JSON path. Defaults to
    /// `bench-results/build-timings/<git-sha>.json`.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct CompareBuildTimingsArgs {
    /// Baseline JSON file. Defaults to
    /// `bench-results/build-timings/baseline.json`.
    #[arg(long)]
    pub baseline: Option<PathBuf>,

    /// Latest snapshot to compare. Defaults to the newest `<sha>.json` under
    /// `bench-results/build-timings/` other than `baseline.json`.
    #[arg(long)]
    pub current: Option<PathBuf>,

    /// Regression threshold as a fraction (0.10 = 10%).
    #[arg(long, default_value_t = 0.10)]
    pub threshold: f64,
}

/// Parsed snapshot we persist to disk. Map ordered for deterministic diffs
/// (issue #3298 style — ordered output across runs / processes).
#[derive(Debug, Default)]
struct Snapshot {
    /// crate name -> total self-compile seconds (sum across all units of that
    /// package, e.g. lib + tests + benches). Summing is intentional: an
    /// individual unit can move around (test added, integration test split)
    /// without changing real cost; per-package total is the stable signal.
    crates: BTreeMap<String, f64>,
}

pub fn run_collect(args: BuildTimingsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    if !args.no_build {
        println!("Running `cargo build --workspace --timings` …");
        let status = Command::new("cargo")
            .args(["build", "--workspace", "--timings"])
            .current_dir(&root)
            .status()?;
        if !status.success() {
            return Err("cargo build --timings failed".into());
        }
    }

    let snapshot = parse_latest_timing_report(&root)?;
    let out = match args.out {
        Some(p) => p,
        None => {
            let sha = git_sha(&root).unwrap_or_else(|| "unknown".to_string());
            root.join("bench-results")
                .join("build-timings")
                .join(format!("{sha}.json"))
        }
    };
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    write_snapshot(&out, &snapshot)?;
    println!(
        "Wrote {} crates of timing data to {}",
        snapshot.crates.len(),
        out.display()
    );
    Ok(())
}

pub fn run_compare(args: CompareBuildTimingsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let dir = root.join("bench-results").join("build-timings");

    let baseline_path = args.baseline.unwrap_or_else(|| dir.join("baseline.json"));
    if !baseline_path.exists() {
        // First weekly run — nothing to compare against. Print a notice and
        // exit 0 so the workflow can upload the artifact for seeding.
        println!(
            "No baseline at {} — nothing to compare. Seed it from the latest snapshot.",
            baseline_path.display()
        );
        return Ok(());
    }
    let current_path = match args.current {
        Some(p) => p,
        None => find_latest_snapshot(&dir, &baseline_path)?
            .ok_or("No snapshot files in bench-results/build-timings/ to compare")?,
    };

    let baseline = read_snapshot(&baseline_path)?;
    let current = read_snapshot(&current_path)?;

    println!(
        "Comparing {} (baseline) vs {} (current), threshold {:.0}%",
        baseline_path.display(),
        current_path.display(),
        args.threshold * 100.0,
    );

    let mut regressions: Vec<(String, f64, f64, f64)> = Vec::new();
    for (name, &cur_secs) in &current.crates {
        let Some(&base_secs) = baseline.crates.get(name) else {
            continue;
        };
        if base_secs <= 0.5 {
            // Skip near-zero crates — small absolute deltas blow up percentage
            // wise without representing real cost.
            continue;
        }
        let delta_pct = (cur_secs - base_secs) / base_secs;
        if delta_pct > args.threshold {
            regressions.push((name.clone(), base_secs, cur_secs, delta_pct));
        }
    }
    regressions.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

    if regressions.is_empty() {
        println!("No regressions over threshold.");
        return Ok(());
    }
    println!("Build-time regressions:");
    for (name, base, cur, pct) in &regressions {
        println!(
            "  {name:<40} {base:>7.2}s -> {cur:>7.2}s  ({:+.1}%)",
            pct * 100.0
        );
    }
    Err(format!(
        "{} crate(s) regressed by more than {:.0}%",
        regressions.len(),
        args.threshold * 100.0
    )
    .into())
}

fn parse_latest_timing_report(root: &Path) -> Result<Snapshot, Box<dyn std::error::Error>> {
    let dir = root.join("target").join("cargo-timings");
    if !dir.exists() {
        return Err(format!(
            "{} does not exist — did `cargo build --timings` run?",
            dir.display()
        )
        .into());
    }
    let html_path = pick_timing_html(&dir)?;
    let html = fs::read_to_string(&html_path)?;
    let unit_data = extract_unit_data(&html).ok_or_else(|| {
        format!(
            "could not locate UNIT_DATA in {} — cargo report format may have changed",
            html_path.display()
        )
    })?;

    let parsed: Value = serde_json::from_str(&unit_data)?;
    let arr = parsed.as_array().ok_or("UNIT_DATA was not a JSON array")?;

    let mut crates: BTreeMap<String, f64> = BTreeMap::new();
    for unit in arr {
        let name = unit
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        // `duration` is the unit's self compile time in seconds (cargo report
        // uses this field name for both stable and recent nightlies).
        let duration = unit.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
        *crates.entry(name).or_insert(0.0) += duration;
    }
    Ok(Snapshot { crates })
}

fn pick_timing_html(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Prefer the unsuffixed `cargo-timing.html` (cargo writes it as the latest
    // alongside `cargo-timing-<timestamp>.html`). Fall back to the newest
    // timestamped file if the unsuffixed one is missing on this cargo version.
    let unsuffixed = dir.join("cargo-timing.html");
    if unsuffixed.exists() {
        return Ok(unsuffixed);
    }
    let mut candidates: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("cargo-timing-") && n.ends_with(".html"))
                .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates
        .pop()
        .ok_or_else(|| format!("no cargo-timing*.html found in {}", dir.display()).into())
}

/// Cargo's HTML report embeds the unit array as either:
///
/// ```js
/// const UNIT_DATA = [...];
/// ```
///
/// or (older versions):
///
/// ```js
/// const UNIT_DATA = JSON.parse('...');
/// ```
///
/// We handle both. Returns the JSON array text ready for `serde_json::from_str`.
fn extract_unit_data(html: &str) -> Option<String> {
    // Form 1: literal array. Walk forward from `UNIT_DATA = ` to the matching
    // closing bracket. This is robust to embedded `]` inside string values.
    let needle = "UNIT_DATA = ";
    let start = html.find(needle)? + needle.len();
    let bytes = html.as_bytes();
    if bytes.get(start) == Some(&b'[') {
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut esc = false;
        for (i, &b) in bytes.iter().enumerate().skip(start) {
            if in_str {
                if esc {
                    esc = false;
                } else if b == b'\\' {
                    esc = true;
                } else if b == b'"' {
                    in_str = false;
                }
                continue;
            }
            match b {
                b'"' => in_str = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(html[start..=i].to_string());
                    }
                }
                _ => {}
            }
        }
        return None;
    }

    // Form 2: JSON.parse('...') — single-quoted JS string literal.
    let parse_needle = "JSON.parse('";
    let p_start = html[start..].find(parse_needle)? + start + parse_needle.len();
    let p_end = html[p_start..].find("')")? + p_start;
    let raw = &html[p_start..p_end];
    // JS string un-escape: \\ -> \, \' -> ', \" -> " (others left as-is —
    // cargo only uses these three).
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('\'') => out.push('\''),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn git_sha(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn write_snapshot(path: &Path, snap: &Snapshot) -> Result<(), Box<dyn std::error::Error>> {
    // Write as a stable, sorted JSON object so diffs in git are minimal.
    let map: serde_json::Map<String, Value> = snap
        .crates
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                Value::from(((*v) * 1000.0).round() / 1000.0), // 3 decimal places
            )
        })
        .collect();
    let value = Value::Object(map);
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

fn read_snapshot(path: &Path) -> Result<Snapshot, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text)?;
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;
    let mut crates = BTreeMap::new();
    for (k, v) in obj {
        if let Some(secs) = v.as_f64() {
            crates.insert(k.clone(), secs);
        }
    }
    Ok(Snapshot { crates })
}

fn find_latest_snapshot(
    dir: &Path,
    baseline: &Path,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    if !dir.exists() {
        return Ok(None);
    }
    let baseline_canon = baseline.canonicalize().ok();
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s == "json")
                .unwrap_or(false)
        })
        .filter(|e| {
            // Skip the baseline itself.
            match (e.path().canonicalize().ok(), baseline_canon.as_ref()) {
                (Some(p), Some(b)) => &p != b,
                _ => e.path() != baseline,
            }
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let t = m.modified().ok()?;
            Some((t, e.path()))
        })
        .collect();
    entries.sort_by_key(|(t, _)| *t);
    Ok(entries.pop().map(|(_, p)| p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_unit_data_handles_literal_array_form() {
        let html = r#"<html><script>
            const UNIT_DATA = [{"name":"foo","duration":1.5},{"name":"bar","duration":2.0}];
        </script></html>"#;
        let extracted = extract_unit_data(html).expect("found");
        let v: Value = serde_json::from_str(&extracted).expect("valid JSON");
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn extract_unit_data_handles_json_parse_form() {
        let raw = r#"[{"name":"foo","duration":1.5}]"#;
        // Simulate cargo's older form with `'` un-escaped (no apostrophes inside payload).
        let html = format!("const UNIT_DATA = JSON.parse('{raw}');");
        let extracted = extract_unit_data(&html).expect("found");
        let v: Value = serde_json::from_str(&extracted).expect("valid JSON");
        assert_eq!(v[0]["name"], "foo");
    }

    #[test]
    fn snapshot_roundtrip_preserves_crate_totals() {
        let mut crates = BTreeMap::new();
        crates.insert("librefang-kernel".to_string(), 12.345);
        crates.insert("librefang-api".to_string(), 7.89);
        let snap = Snapshot { crates };

        let dir = tempdir();
        let path = dir.join("snap.json");
        write_snapshot(&path, &snap).unwrap();
        let back = read_snapshot(&path).unwrap();
        assert!((back.crates["librefang-kernel"] - 12.345).abs() < 0.01);
        assert!((back.crates["librefang-api"] - 7.89).abs() < 0.01);
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "librefang-build-timings-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }
}
