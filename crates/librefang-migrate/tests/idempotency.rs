//! End-to-end idempotency + forward-compat tests for the migrate crate
//! (issue #3407).
//!
//! In-crate tests in `src/openclaw.rs` already cover unit-level idempotency
//! by asserting `report.imported.is_empty()` on a second run. These tests
//! sit one level out and verify the *filesystem-level* contract that
//! callers actually care about:
//!
//!   * a second run produces a byte-identical destination tree
//!     (no duplicate sessions, no clobbered configs, no rewritten
//!     timestamps);
//!   * a partially-completed migration (file deleted between runs,
//!     simulating a process killed mid-write) can be re-driven to a
//!     correct state without corrupting the surviving entries; and
//!   * the prior major version's `KernelConfig` shape still
//!     deserialises and round-trips through `run_migrations`.
//!
//! See `tests/fixtures/legacy_config/config_v1.toml` for the
//! forward-compat fixture.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use librefang_migrate::{MigrateOptions, MigrateSource, openclaw, openfang};
use tempfile::TempDir;

/// Read every regular file under `root` and return a sorted map from
/// path-relative-to-root to its byte contents.
///
/// `BTreeMap` keeps iteration order deterministic across runs so that any
/// `assert_eq!(snapshot_a, snapshot_b)` failure points at the first
/// differing path rather than depending on `HashMap` insertion order.
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    if !root.exists() {
        return out;
    }
    for entry in walkdir_iter(root) {
        if entry.is_file() {
            let rel = entry
                .strip_prefix(root)
                .expect("entry under root")
                .to_path_buf();
            let bytes = std::fs::read(&entry)
                .unwrap_or_else(|e| panic!("read {} failed: {e}", entry.display()));
            out.insert(rel, bytes);
        }
    }
    out
}

/// Walk a directory tree without pulling `walkdir` into this crate's dev-deps
/// (it's already a runtime dep but the test crate compiles separately and we
/// keep dependencies minimal). A small recursive walk over `read_dir` is
/// enough for the migrated trees we produce.
fn walkdir_iter(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.file_type() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.push(path);
            }
            // Symlinks are not produced by either migrator and we ignore
            // them deliberately to keep snapshots stable across platforms.
        }
    }
    out.sort();
    out
}

/// Produce a minimal but representative openclaw source workspace at `dir`.
///
/// We mirror the shape used by the in-crate `create_json5_workspace` helper
/// (see `src/openclaw.rs`), but trimmed to exactly what the idempotency
/// assertions need so that diffs in the snapshot remain readable when a
/// regression breaks them.
fn write_openclaw_workspace(dir: &Path) {
    let openclaw_json = r##"{
  agents: {
    list: [
      {
        id: "coder",
        name: "Coder",
        model: "deepseek/deepseek-chat",
        tools: { allow: ["Read", "Write", "Bash"] },
        identity: "You are an expert software engineer."
      },
      {
        id: "researcher",
        model: "google/gemini-2.5-flash",
        tools: { profile: "research" }
      }
    ]
  },
  channels: {
    telegram: {
      botToken: "tg-token-123",
      allowFrom: ["user1"],
      groupPolicy: "open",
      dmPolicy: "allowlist"
    }
  },
  memory: { backend: "builtin" },
  session: { scope: "per-sender" }
}"##;
    std::fs::write(dir.join("openclaw.json"), openclaw_json).unwrap();

    // Per-agent memory.
    let mem = dir.join("memory").join("coder");
    std::fs::create_dir_all(&mem).unwrap();
    std::fs::write(mem.join("MEMORY.md"), "## Coder Memory\n- Prefers Rust\n").unwrap();

    // A session file (idempotency must not duplicate it on re-run).
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("agent_coder_main.jsonl"),
        "{\"role\":\"user\",\"content\":\"hello\"}\n",
    )
    .unwrap();
}

/// Produce a minimal openfang source workspace at `dir`.
///
/// OpenFang migration is a recursive copy with `.toml`/`.env` rewriting,
/// so the fixture only needs a couple of files of each interesting kind.
fn write_openfang_workspace(dir: &Path) {
    std::fs::write(
        dir.join("config.toml"),
        "config_version = 2\n\
         api_listen = \"0.0.0.0:4545\"\n\
         log_level = \"info\"\n\
         \n\
         [default_model]\n\
         provider = \"openfang-auto\"\n\
         model = \"gpt-4\"\n",
    )
    .unwrap();

    std::fs::write(dir.join("secrets.env"), "OPENFANG_API_KEY=keep-me-verbatim\n").unwrap();

    let agent_dir = dir.join("agents").join("coder");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("agent.toml"),
        "name = \"coder\"\nframework = \"openfang\"\n",
    )
    .unwrap();

    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("index.db"), b"binary-bytes-here").unwrap();
}

fn opts(source: MigrateSource, src: &Path, dst: &Path) -> MigrateOptions {
    MigrateOptions {
        source,
        source_dir: src.to_path_buf(),
        target_dir: dst.to_path_buf(),
        dry_run: false,
    }
}

// ---------------------------------------------------------------------------
// A. openclaw second-run is a no-op (filesystem-level byte equality)
// ---------------------------------------------------------------------------

/// After a successful openclaw migration, running it again must leave the
/// destination tree byte-identical. The in-crate test asserts the report
/// is empty on the second run; this test asserts the actual files on disk
/// (including the timestamped marker body) are unchanged, which is the
/// stronger contract callers depend on.
#[test]
fn openclaw_second_run_is_byte_identical() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    write_openclaw_workspace(src.path());
    let options = opts(MigrateSource::OpenClaw, src.path(), dst.path());

    openclaw::migrate(&options).expect("first run succeeds");
    let snapshot_a = snapshot_tree(dst.path());
    assert!(
        !snapshot_a.is_empty(),
        "first openclaw run must produce some output"
    );

    // The marker short-circuits a second run before any writes happen, so
    // the on-disk tree (marker body included) must be byte-identical.
    let report = openclaw::migrate(&options).expect("second run succeeds");
    assert!(
        report.imported.is_empty(),
        "second openclaw run should not import anything (got {} entries)",
        report.imported.len()
    );

    let snapshot_b = snapshot_tree(dst.path());
    assert_eq!(
        snapshot_a, snapshot_b,
        "openclaw second run mutated the destination tree"
    );
}

// ---------------------------------------------------------------------------
// B. openfang second-run is a no-op
// ---------------------------------------------------------------------------

/// OpenFang migration has no marker file; it relies on per-entry
/// `dest_path.exists()` skips. After a clean run, every source path is
/// already present at the destination, so a second run must be a complete
/// no-op on disk.
#[test]
fn openfang_second_run_is_byte_identical() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    write_openfang_workspace(src.path());
    let options = opts(MigrateSource::OpenFang, src.path(), dst.path());

    let first = openfang::migrate(&options).expect("first run succeeds");
    let snapshot_a = snapshot_tree(dst.path());
    assert!(
        !first.imported.is_empty(),
        "first openfang run must import something"
    );
    assert!(
        first.skipped.is_empty(),
        "fresh openfang dst should not skip anything (got {:?})",
        first.skipped
    );

    let second = openfang::migrate(&options).expect("second run succeeds");
    assert!(
        second.imported.is_empty(),
        "second openfang run should not import anything (got {} entries)",
        second.imported.len()
    );
    // Each source file should now be a "skipped: already exists" entry.
    assert_eq!(
        second.skipped.len(),
        first.imported.len(),
        "every previously-imported entry should be skipped on re-run"
    );

    let snapshot_b = snapshot_tree(dst.path());
    assert_eq!(
        snapshot_a, snapshot_b,
        "openfang second run mutated the destination tree"
    );
}

// ---------------------------------------------------------------------------
// C. Partial-write recovery
// ---------------------------------------------------------------------------

/// Simulate a migration that died after writing some files: delete one of
/// the produced files (and the marker, since openclaw refuses to re-run
/// while it's present), then run again. The deleted file must be recreated
/// with its original content, and no surviving file may be clobbered.
#[test]
fn openclaw_partial_write_is_recoverable() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    write_openclaw_workspace(src.path());
    let options = opts(MigrateSource::OpenClaw, src.path(), dst.path());

    openclaw::migrate(&options).expect("first run succeeds");
    let baseline = snapshot_tree(dst.path());

    // Pick a deterministic file produced by the first run that we will
    // pretend the killed process never managed to write.
    let victim_rel = baseline
        .keys()
        .find(|p| {
            // Prefer an agent manifest if one is present; otherwise any
            // non-marker file under the destination tree will do.
            let s = p.to_string_lossy();
            s.ends_with("agent.toml") && s.contains("coder")
        })
        .or_else(|| {
            baseline
                .keys()
                .find(|p| !p.to_string_lossy().starts_with(".openclaw_migrated"))
        })
        .cloned()
        .expect("first run produced at least one non-marker file");
    let victim_abs = dst.path().join(&victim_rel);
    let original_bytes = baseline
        .get(&victim_rel)
        .cloned()
        .expect("victim was in the baseline snapshot");

    // Crash simulation: drop the victim file and the marker so the next
    // run is allowed to proceed past the short-circuit guard.
    std::fs::remove_file(&victim_abs).expect("remove victim");
    let marker = dst.path().join(".openclaw_migrated");
    if marker.exists() {
        std::fs::remove_file(&marker).expect("remove marker");
    }

    openclaw::migrate(&options).expect("recovery run succeeds");

    // The victim file is back with its original byte content.
    assert!(
        victim_abs.exists(),
        "recovery run failed to recreate {}",
        victim_abs.display()
    );
    let recreated = std::fs::read(&victim_abs).expect("read recreated victim");
    assert_eq!(
        recreated, original_bytes,
        "recreated {} differs from the original migration output",
        victim_rel.display()
    );

    // No surviving file may have been overwritten — `promote_staging`
    // implements never-clobber semantics (#3795). Compare every other
    // non-marker entry against the baseline.
    let after = snapshot_tree(dst.path());
    for (path, bytes) in &baseline {
        if path == &victim_rel {
            continue;
        }
        // The marker body contains a wall-clock timestamp; compare its
        // existence only.
        if path.to_string_lossy().starts_with(".openclaw_migrated") {
            assert!(after.contains_key(path), "marker missing after recovery");
            continue;
        }
        let after_bytes = after
            .get(path)
            .unwrap_or_else(|| panic!("file {} disappeared after recovery", path.display()));
        assert_eq!(
            after_bytes,
            bytes,
            "surviving file {} was clobbered by recovery run",
            path.display()
        );
    }
}

/// Same shape as the openclaw recovery test, but for openfang. Since
/// openfang has no marker, we only need to drop the victim file before
/// re-running.
#[test]
fn openfang_partial_write_is_recoverable() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    write_openfang_workspace(src.path());
    let options = opts(MigrateSource::OpenFang, src.path(), dst.path());

    openfang::migrate(&options).expect("first run succeeds");
    let baseline = snapshot_tree(dst.path());

    // Pick the agent manifest as the victim — it's a rewritten file, so
    // recreating it also exercises the rewrite path on recovery.
    let victim_rel: PathBuf = ["agents", "coder", "agent.toml"].iter().collect();
    let victim_abs = dst.path().join(&victim_rel);
    assert!(
        victim_abs.exists(),
        "fixture must produce {}",
        victim_abs.display()
    );
    let original_bytes = std::fs::read(&victim_abs).expect("read victim");

    std::fs::remove_file(&victim_abs).expect("remove victim");
    openfang::migrate(&options).expect("recovery run succeeds");

    assert!(
        victim_abs.exists(),
        "recovery run failed to recreate {}",
        victim_abs.display()
    );
    let recreated = std::fs::read(&victim_abs).expect("read recreated victim");
    assert_eq!(
        recreated, original_bytes,
        "recreated {} differs from the original migration output",
        victim_rel.display()
    );

    // Nothing else moved.
    let after = snapshot_tree(dst.path());
    for (path, bytes) in &baseline {
        let after_bytes = after
            .get(path)
            .unwrap_or_else(|| panic!("file {} disappeared after recovery", path.display()));
        assert_eq!(
            after_bytes,
            bytes,
            "surviving file {} was clobbered by recovery run",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// Forward-compat: prior major version's KernelConfig shape still parses.
// ---------------------------------------------------------------------------

/// The fixture under `tests/fixtures/legacy_config/config_v1.toml`
/// represents the prior major version's `KernelConfig` shape (the v1
/// layout described in `crates/librefang-types/src/config/version.rs`,
/// before the v1->v2 migration hoisted `[api].api_key/api_listen/log_level`
/// to root level).
///
/// The fixture is intentionally narrow: only `config_version` plus the
/// hoisted `[api]` table. This is the smallest possible representation
/// that exercises both:
///
///   1. raw deserialisation into the *current* `KernelConfig` (which has
///      `#[serde(default)]` and ignores unknown top-level fields), and
///   2. the documented `run_migrations(_, 1)` -> `CONFIG_VERSION`
///      forward path.
///
/// We deliberately do NOT try to construct a "complete v1 config" because
/// the surface area has grown enormously and we cannot safely re-construct
/// every legacy field-by-field default from this side of the schema. The
/// minimal fixture is enough to assert "the load path still works".
#[test]
fn legacy_v1_config_parses_into_current_kernel_config() {
    use librefang_types::config::KernelConfig;

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("legacy_config")
        .join("config_v1.toml");
    let content = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", fixture.display()));

    // (1) Raw deserialisation must succeed. Unknown `[api]` table is
    //     ignored under `#[serde(default)]`; missing root fields fall
    //     back to `Default`.
    let cfg: KernelConfig = toml::from_str(&content).unwrap_or_else(|e| {
        panic!("legacy v1 config no longer deserialises into KernelConfig: {e}")
    });
    assert_eq!(
        cfg.config_version, 1,
        "fixture should declare config_version = 1 verbatim"
    );
}

#[test]
fn legacy_v1_config_migrates_forward_to_current_version() {
    // `mod version` is private inside `librefang-types::config`, but its items
    // are re-exported with `pub use version::*;` — so we reach them at the
    // `config` module level rather than `config::version`.
    use librefang_types::config::{CONFIG_VERSION, run_migrations};

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("legacy_config")
        .join("config_v1.toml");
    let content = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", fixture.display()));

    // (2) `run_migrations` must lift the v1 fixture to the current
    //     `CONFIG_VERSION` and hoist the `[api]` table fields to root.
    let mut raw: toml::Value = toml::from_str(&content).expect("fixture is valid TOML");
    let final_version = run_migrations(&mut raw, 1)
        .unwrap_or_else(|e| panic!("v1 -> current migration failed: {e}"));
    assert_eq!(
        final_version, CONFIG_VERSION,
        "run_migrations must reach CONFIG_VERSION starting from v1"
    );

    let tbl = raw.as_table().expect("migrated raw is still a table");
    assert!(
        !tbl.contains_key("api"),
        "v1 -> v2 migration must remove the [api] table"
    );
    assert_eq!(
        tbl.get("api_key").and_then(|v| v.as_str()),
        Some("legacy-secret-key"),
        "v1 -> v2 migration must hoist api_key to root"
    );
    assert_eq!(
        tbl.get("api_listen").and_then(|v| v.as_str()),
        Some("127.0.0.1:4545"),
        "v1 -> v2 migration must hoist api_listen to root"
    );
    assert_eq!(
        tbl.get("log_level").and_then(|v| v.as_str()),
        Some("info"),
        "v1 -> v2 migration must hoist log_level to root"
    );
}
