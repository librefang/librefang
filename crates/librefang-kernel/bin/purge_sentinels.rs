//! `purge_sentinels` — one-shot CLI to remove silent-response sentinel
//! lines (`NO_REPLY`, `[no reply needed]`, …) from agent memory files.
//!
//! Background (Phase 2 §B, OB-02 / OB-03): historical agent runs leaked
//! the runtime's silent-reply sentinels into stored memory notes. The
//! model then reads its own memory on subsequent turns and parrots the
//! literal back into chat. The runtime fixes are necessary but not
//! sufficient — already-polluted memory keeps the bug alive.
//!
//! This binary scans `*.md` files under a path (default
//! `/data/workspaces/agents/`) and removes any line whose trimmed
//! content is recognised as a sentinel by the canonical detector
//! (`librefang_runtime::silent_response::is_silent_response`). Partial
//! matches inside a sentence (e.g. "I said NO_REPLY yesterday") are
//! preserved — only whole-line sentinels are removed.
//!
//! Safety:
//! - `--dry-run` (default) reports what WOULD be removed; touches no
//!   files.
//! - `--apply` performs the rewrite. Before writing, a `.bak` is created
//!   alongside each modified file. If a `.bak` already exists and its
//!   contents differ from the current file, the run aborts with a
//!   "backup mismatch" error to avoid destroying an earlier safety net.
//! - Idempotent: a second `--apply` run is a no-op (0 removals reported,
//!   no `.bak` rewrite).
//!
//! Exit code: 0 on success, non-zero on any I/O / backup-mismatch error.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use librefang_runtime::silent_response::is_silent_response;

const DEFAULT_ROOT: &str = "/data/workspaces/agents/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    DryRun,
    Apply,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut mode = Mode::DryRun;
    let mut path: Option<PathBuf> = None;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--dry-run" => mode = Mode::DryRun,
            "--apply" => mode = Mode::Apply,
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("unknown flag: {other}");
                print_help();
                return ExitCode::from(2);
            }
            other => path = Some(PathBuf::from(other)),
        }
        let _ = iter.size_hint();
    }
    let root = path.unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));

    if !root.exists() {
        eprintln!("path does not exist: {}", root.display());
        return ExitCode::from(1);
    }

    let mut stdout = io::stdout().lock();
    let _ = writeln!(
        stdout,
        "purge_sentinels mode={mode:?} root={}",
        root.display()
    );

    let mut total_files = 0usize;
    let mut total_removed = 0usize;
    let mut errors = 0usize;
    let mut md_files = Vec::new();
    if let Err(e) = collect_md_files(&root, &mut md_files) {
        eprintln!("walk error: {e}");
        return ExitCode::from(1);
    }
    md_files.sort();

    for file in &md_files {
        match process_file(file, mode) {
            Ok(removed) => {
                total_files += 1;
                total_removed += removed;
                if removed > 0 {
                    let _ = writeln!(
                        stdout,
                        "{}: {removed} line(s) {}",
                        file.display(),
                        if mode == Mode::Apply {
                            "removed"
                        } else {
                            "would be removed"
                        }
                    );
                }
            }
            Err(e) => {
                errors += 1;
                eprintln!("{}: ERROR {e}", file.display());
            }
        }
    }

    let _ = writeln!(
        stdout,
        "\nsummary: scanned={} files, removed={} lines, errors={}",
        total_files, total_removed, errors
    );
    if errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_help() {
    println!(
        "purge_sentinels — remove silent-response sentinel lines from agent memory files

USAGE:
    purge_sentinels [--dry-run|--apply] [PATH]

ARGS:
    PATH        Directory to scan recursively for *.md files
                (default: {DEFAULT_ROOT})

FLAGS:
    --dry-run   Report what would be removed; touch no files (default)
    --apply     Rewrite each modified file after creating a .bak backup
    -h, --help  Show this message

SAFETY:
    Refuses to overwrite an existing .bak whose content differs from the
    current file — abort and resolve the prior backup first."
    );
}

/// Recursively collect `*.md` files under `dir`. Skips `.bak` files.
fn collect_md_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if dir.is_file() {
        if dir.extension().and_then(|e| e.to_str()) == Some("md")
            && !dir.to_string_lossy().ends_with(".bak")
        {
            out.push(dir.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ftype = entry.file_type()?;
        if ftype.is_dir() {
            collect_md_files(&path, out)?;
        } else if ftype.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("md")
            && !path.to_string_lossy().ends_with(".bak.md")
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Process a single file. Returns the count of removed lines.
///
/// In `DryRun` mode, computes the new content but writes nothing.
/// In `Apply` mode, creates a `.bak` (or verifies the existing one
/// matches the current file) and writes the cleaned content.
fn process_file(path: &Path, mode: Mode) -> io::Result<usize> {
    let original = fs::read_to_string(path)?;
    let (cleaned, removed) = clean_content(&original);
    if removed == 0 {
        return Ok(0);
    }
    if mode == Mode::DryRun {
        return Ok(removed);
    }

    let bak = bak_path(path);
    if bak.exists() {
        let bak_content = fs::read_to_string(&bak)?;
        if bak_content != original {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "backup mismatch — refusing to overwrite: {} differs from current file. \
                     Resolve the previous backup before retrying.",
                    bak.display()
                ),
            ));
        }
        // Backup matches current content: no need to rewrite the .bak.
    } else {
        fs::write(&bak, &original)?;
    }
    fs::write(path, &cleaned)?;
    Ok(removed)
}

fn bak_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

/// Remove lines whose trimmed content the canonical detector classifies
/// as silent. Returns `(new_content, removed_count)`.
///
/// Preserves original line endings: lines are split on `'\n'` and rejoined
/// with `'\n'`; a trailing newline (if present) is preserved.
fn clean_content(content: &str) -> (String, usize) {
    let had_trailing_newline = content.ends_with('\n');
    let mut removed = 0;
    let kept: Vec<&str> = content
        .split('\n')
        .filter(|line| {
            let t = line.trim();
            if t.is_empty() {
                return true;
            }
            if is_silent_response(t) {
                removed += 1;
                false
            } else {
                true
            }
        })
        .collect();
    let mut out = kept.join("\n");
    if had_trailing_newline && !out.ends_with('\n') {
        out.push('\n');
    }
    (out, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_whole_line_sentinels() {
        let input = "Hello\nNO_REPLY\nWorld\n";
        let (out, n) = clean_content(input);
        assert_eq!(n, 1);
        assert_eq!(out, "Hello\nWorld\n");
    }

    #[test]
    fn clean_preserves_embedded_sentinels() {
        let input = "I said NO_REPLY yesterday\nplain text\n";
        let (out, n) = clean_content(input);
        assert_eq!(n, 0);
        assert_eq!(out, input);
    }

    #[test]
    fn clean_removes_bracketed_form() {
        let input = "[no reply needed]\nreal note\n";
        let (out, n) = clean_content(input);
        assert_eq!(n, 1);
        assert_eq!(out, "real note\n");
    }

    #[test]
    fn clean_is_idempotent() {
        let input = "Hello\nNO_REPLY\n[no reply needed]\nWorld\n";
        let (first, n1) = clean_content(input);
        assert_eq!(n1, 2);
        let (second, n2) = clean_content(&first);
        assert_eq!(n2, 0);
        assert_eq!(first, second);
    }
}
