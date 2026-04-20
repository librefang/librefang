//! Integration tests for the `purge_sentinels` CLI binary.
//!
//! Drives the compiled binary via `std::process::Command::cargo_bin`-style
//! discovery (we use `env!("CARGO_BIN_EXE_purge_sentinels")` which Cargo
//! sets for binary deps of integration tests). Verifies:
//!
//! - dry-run reports counts but writes nothing
//! - apply creates `.bak` and rewrites
//! - second apply is idempotent
//! - apply aborts when an existing `.bak` differs from the current file
//! - sentence-embedded sentinels are preserved (only whole-line removal)

use std::fs;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_purge_sentinels");

fn fixture_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    fs::write(
        dir.path().join("a.md"),
        "Hello\nNO_REPLY\nWorld\n[no reply needed]\nfinal line\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.md"),
        "I said NO_REPLY yesterday but real text follows\nplain note\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("c.md"),
        "totally clean file\nno sentinels here\n",
    )
    .unwrap();
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("d.md"), "nested\n  no_reply  \nkeep me\n").unwrap();
    dir
}

#[test]
fn dry_run_reports_counts_and_touches_nothing() {
    let dir = fixture_dir();
    let before_a = fs::read_to_string(dir.path().join("a.md")).unwrap();

    let out = Command::new(BIN)
        .args(["--dry-run", dir.path().to_str().unwrap()])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("removed=3 lines") || stdout.contains("removed=3"),
        "stdout: {stdout}"
    );

    let after_a = fs::read_to_string(dir.path().join("a.md")).unwrap();
    assert_eq!(before_a, after_a, "dry-run must not modify files");
    assert!(
        !dir.path().join("a.md.bak").exists(),
        "dry-run must not create .bak"
    );
}

#[test]
fn apply_creates_backup_and_rewrites() {
    let dir = fixture_dir();
    let original_a = fs::read_to_string(dir.path().join("a.md")).unwrap();

    let out = Command::new(BIN)
        .args(["--apply", dir.path().to_str().unwrap()])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Backup created with original content
    let bak = fs::read_to_string(dir.path().join("a.md.bak")).expect(".bak exists");
    assert_eq!(bak, original_a);

    // a.md no longer contains the sentinels
    let cleaned = fs::read_to_string(dir.path().join("a.md")).unwrap();
    assert!(!cleaned.contains("NO_REPLY"));
    assert!(!cleaned.contains("[no reply needed]"));
    assert!(cleaned.contains("Hello"));
    assert!(cleaned.contains("World"));
    assert!(cleaned.contains("final line"));

    // b.md unchanged — sentinel was embedded mid-sentence, not whole line
    assert_eq!(
        fs::read_to_string(dir.path().join("b.md")).unwrap(),
        "I said NO_REPLY yesterday but real text follows\nplain note\n"
    );
    assert!(
        !dir.path().join("b.md.bak").exists(),
        "no changes => no .bak"
    );

    // c.md is clean — no .bak, untouched
    assert!(!dir.path().join("c.md.bak").exists());

    // nested/d.md cleaned (lowercase + spaces variant)
    let nd = fs::read_to_string(dir.path().join("nested/d.md")).unwrap();
    assert!(!nd.contains("no_reply"));
    assert!(nd.contains("nested"));
    assert!(nd.contains("keep me"));
    assert!(dir.path().join("nested/d.md.bak").exists());
}

#[test]
fn apply_is_idempotent() {
    let dir = fixture_dir();
    let _ = Command::new(BIN)
        .args(["--apply", dir.path().to_str().unwrap()])
        .output()
        .expect("first run");
    let cleaned_a = fs::read_to_string(dir.path().join("a.md")).unwrap();
    let bak_a = fs::read_to_string(dir.path().join("a.md.bak")).unwrap();

    // Second run
    let out = Command::new(BIN)
        .args(["--apply", dir.path().to_str().unwrap()])
        .output()
        .expect("second run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("removed=0 lines") || stdout.contains("removed=0"),
        "stdout: {stdout}"
    );

    // Files and backups unchanged
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md")).unwrap(),
        cleaned_a
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md.bak")).unwrap(),
        bak_a
    );
}

#[test]
fn apply_aborts_when_existing_bak_differs() {
    let dir = fixture_dir();
    // Pre-seed a stale .bak that does NOT match the current file.
    fs::write(dir.path().join("a.md.bak"), "stale unrelated content\n").unwrap();

    let out = Command::new(BIN)
        .args(["--apply", dir.path().to_str().unwrap()])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("backup mismatch") || stderr.contains("ERROR"),
        "expected backup-mismatch error, got: {stderr}"
    );
    assert!(!out.status.success(), "should exit non-zero");

    // Stale .bak preserved
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md.bak")).unwrap(),
        "stale unrelated content\n"
    );
}

#[test]
fn nonexistent_path_exits_non_zero() {
    let out = Command::new(BIN)
        .args(["--apply", "/this/path/does/not/exist"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("does not exist"), "stderr: {stderr}");
}
