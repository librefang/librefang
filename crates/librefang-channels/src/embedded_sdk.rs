//! Bundle the `librefang-sdk` Python package into the daemon binary
//! and extract it on first sidecar spawn, so new users with just a
//! `python3` on PATH can enable a channel sidecar without first
//! running `pip install librefang-sdk`.
//!
//! ## Why this exists
//!
//! Every channel adapter was migrated out-of-process (see the channels
//! crate `lib.rs` header). Spawning a sidecar runs
//! `python3 -m librefang.sidecar.adapters.<name>`, which requires the
//! `librefang` package to be importable. For developers running from a
//! source checkout the natural answer is `pip install -e sdk/python/`
//! — but for end users the daemon has no business demanding pip
//! literacy. This module makes the sidecar work zero-setup as long as
//! a `python3` (any 3.8+) is on PATH.
//!
//! ## Precedence
//!
//! The embedded copy is a **fallback**, not a hijack. On every spawn
//! we run a one-shot `<command> -c "import librefang.sidecar"` (cached
//! per command path) and:
//!
//! - If the interpreter already imports `librefang.sidecar`
//!   successfully (the developer case — editable / pip / venv), we do
//!   **nothing**: no PYTHONPATH mutation, no extract on first need.
//!   Their workflow is unchanged.
//! - Otherwise (the new-user case — no SDK installed anywhere), we
//!   lazily extract the embedded tree once to `<home>/sidecar-python/
//!   <content_hash>/` and prepend that directory to the child's
//!   `PYTHONPATH`.
//!
//! Skipping the inject when a real install exists is what keeps
//! developers' editable installs authoritative — the embedded copy
//! never gets a chance to shadow a freshly-edited
//! `sdk/python/librefang/sidecar/adapters/telegram.py`.
//!
//! ## Filesystem layout
//!
//! ```text
//! <home>/sidecar-python/
//!   <hash>/                 # short SHA-256 of the embedded tree
//!     librefang/
//!       __init__.py
//!       sidecar/...
//!       sdk/...
//!     .complete             # marker written last (atomic completion)
//! ```
//!
//! The hash-namespaced directory means a daemon upgrade carrying a
//! newer SDK extracts to a new subdirectory; the old one stays put
//! until the user cleans `<home>/sidecar-python/` themselves. That's
//! deliberate — running `librefang start` with the previous binary
//! after a partial upgrade keeps working.
//!
//! ## Concurrency
//!
//! Two sidecars (e.g. telegram and discord) spawning at once both
//! enter `ensure_extracted`. The function is idempotent (the marker
//! file gates re-extraction) and uses `rename`-into-place from a
//! per-pid temporary to make the visible final directory atomic with
//! respect to other readers / racing processes. The `OnceLock`
//! around the content hash avoids re-hashing 1MB of embedded files
//! on every spawn.

use include_dir::{include_dir, Dir, DirEntry};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tracing::{debug, warn};

/// The `librefang/` package tree, embedded at compile time. Path is
/// relative to `crates/librefang-channels/`, hence the `../../` to
/// reach the workspace `sdk/python/` directory.
///
/// `include_dir!` emits `cargo:rerun-if-changed` for every file under
/// this tree, so editing any Python source in `sdk/python/librefang/`
/// triggers a rebuild of this crate.
static EMBEDDED_SDK: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../sdk/python/librefang");

/// Files / directories that should never reach the extracted copy.
/// `__pycache__/` is local CPython cache state — embedding it bloats
/// the binary and writing it back out at extraction time would shadow
/// the running interpreter's own bytecode generation. `.pyc` files
/// outside `__pycache__/` (rare, legacy) are dropped for the same
/// reason.
fn should_skip(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str();
        s == "__pycache__" || s == ".DS_Store"
    }) || path.extension().is_some_and(|e| e == "pyc")
}

/// Short content hash of the embedded tree. Computed once per process
/// (the result is cached) by walking entries in a stable order and
/// folding each kept file's relative path + bytes into SHA-256.
///
/// Twelve hex chars (48 bits) is plenty for distinguishing
/// daemon-binary builds on a single user's machine — we are not
/// defending against adversarial collisions, only avoiding extracting
/// over a divergent older copy.
fn embedded_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut hasher = Sha256::new();
        // Stable traversal: collect all kept (path, bytes) into a
        // sorted vec before hashing, so two builds with identical
        // sources but different filesystem walk orders hash the same.
        let mut entries: Vec<(PathBuf, &[u8])> = Vec::new();
        collect_files(&EMBEDDED_SDK, &mut entries);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, bytes) in entries {
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(b"\0");
            hasher.update(bytes);
        }
        let digest = hasher.finalize();
        hex::encode(digest)[..12].to_string()
    })
}

fn collect_files<'a>(dir: &'a Dir<'a>, out: &mut Vec<(PathBuf, &'a [u8])>) {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(d) => {
                if should_skip(d.path()) {
                    continue;
                }
                collect_files(d, out);
            }
            DirEntry::File(f) => {
                if should_skip(f.path()) {
                    continue;
                }
                out.push((f.path().to_path_buf(), f.contents()));
            }
        }
    }
}

/// Idempotent extract. Returns the directory that should be prepended
/// to `PYTHONPATH` (i.e. the directory that has `librefang/` as an
/// immediate child).
///
/// The marker file `.complete` is written **after** every other file
/// has been flushed and renamed into place, so a torn extraction
/// from a previous run (process killed mid-write) is detected and
/// retried automatically.
pub(crate) fn ensure_extracted(home_dir: &Path) -> std::io::Result<PathBuf> {
    let hash = embedded_hash();
    let root = home_dir.join("sidecar-python");
    let target = root.join(hash);
    let marker = target.join(".complete");
    if marker.exists() {
        return Ok(target);
    }

    std::fs::create_dir_all(&root)?;

    // Torn previous run? An existing `target` directory without the
    // `.complete` marker means a prior extract was killed before it
    // could finish — its tree may be partial, byte-corrupt, or stale.
    // POSIX `rename` of a directory onto a non-empty directory is
    // ENOTEMPTY, so we have to clear it before staging. Concurrent
    // recovery is best-effort: if a racing process removes it first,
    // the NotFound is swallowed.
    if target.exists() {
        if let Err(e) = std::fs::remove_dir_all(&target) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e);
            }
        }
    }

    // Extract into a sibling pid-tagged staging dir, then atomically
    // rename onto `target`. Concurrent extractions (multi-daemon or
    // racing threads inside one daemon) all converge on the same
    // final path — whoever loses the rename simply cleans their tmp.
    let tmp = root.join(format!("{hash}.tmp.{}", std::process::id()));
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    let librefang_root = tmp.join("librefang");
    std::fs::create_dir_all(&librefang_root)?;
    extract_tree(&EMBEDDED_SDK, &librefang_root)?;
    // Marker last. Readers MUST trust nothing without it.
    std::fs::write(tmp.join(".complete"), hash)?;

    match std::fs::rename(&tmp, &target) {
        Ok(()) => Ok(target),
        Err(rename_err) => {
            // Lost the race: another extractor finished first. If the
            // final target now has a marker, treat that as success
            // and drop our staging dir.
            if marker.exists() {
                let _ = std::fs::remove_dir_all(&tmp);
                Ok(target)
            } else {
                // Either the rename failed for an unrelated reason
                // (cross-device, permission), or the loser's directory
                // is still incomplete. Surface the original error so
                // the supervisor logs it and the spawn fails loudly
                // rather than silently shipping a half-extracted SDK.
                let _ = std::fs::remove_dir_all(&tmp);
                Err(rename_err)
            }
        }
    }
}

/// Recursively write every kept entry under `dir` into `target_root`
/// preserving relative structure. Paths embedded via `include_dir!`
/// are already relative to the include root (which IS the `librefang`
/// package), so a top-level `__init__.py` lands directly under
/// `target_root`.
fn extract_tree(dir: &Dir<'_>, target_root: &Path) -> std::io::Result<()> {
    for entry in dir.entries() {
        let rel = entry.path();
        if should_skip(rel) {
            continue;
        }
        let dest = target_root.join(rel);
        match entry {
            DirEntry::Dir(d) => {
                std::fs::create_dir_all(&dest)?;
                extract_tree(d, target_root)?;
            }
            DirEntry::File(f) => {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, f.contents())?;
            }
        }
    }
    Ok(())
}

/// True when the basename of `command` looks like a CPython
/// interpreter we are willing to pre-check and inject PYTHONPATH for.
///
/// Conservative on purpose. Operators who wrap their interpreter
/// (`uv run python …`, `bash -c …`, `nix-shell …`) opt out of the
/// fallback automatically — they're already in the business of
/// owning their Python environment and benefit from the daemon NOT
/// silently shadowing it.
///
/// Splits on both `/` and `\\` so Windows-style paths still classify
/// correctly when the test host is POSIX (`Path::file_stem` honours
/// only the host's native separator). `.exe` / `.EXE` is the only
/// extension stripped — anything else (`python3.x`, `python3.dev`)
/// fails the digit-only suffix check below and is rejected.
fn command_is_python_interpreter(command: &str) -> bool {
    let basename = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let basename = basename
        .strip_suffix(".exe")
        .or_else(|| basename.strip_suffix(".EXE"))
        .unwrap_or(basename);
    if basename == "python" || basename == "python3" {
        return true;
    }
    if let Some(rest) = basename.strip_prefix("python3.") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// Returns `true` iff `<command> -c "import librefang.sidecar"` exits 0.
/// Cached per command string for the lifetime of the daemon — the SDK's
/// installed/missing state on a developer machine does not flicker
/// under a running daemon, and the cache keeps the spawn-time pre-check
/// at one subprocess per unique command path.
fn has_real_sdk_installed(command: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(&v) = guard.get(command) {
            return v;
        }
    }
    let probed = Command::new(command)
        .args(["-c", "import librefang.sidecar"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(command.to_string(), probed);
    }
    probed
}

/// Single platform-correct PATH separator for `PYTHONPATH` composition.
#[cfg(windows)]
const PYTHONPATH_SEP: &str = ";";
#[cfg(not(windows))]
const PYTHONPATH_SEP: &str = ":";

/// Decide whether to inject the embedded SDK on `PYTHONPATH` for the
/// upcoming child, and if so, return the composed value to set.
///
/// Returns `None` when:
/// - `command` doesn't look like a Python interpreter (operator opted
///   out via wrapper),
/// - the interpreter already imports `librefang.sidecar` (developer
///   has it installed; their copy must remain authoritative),
/// - extraction failed for any reason (logged at WARN; the spawn will
///   still proceed and fail with the sidecar's own diagnostic, which
///   is more actionable than a silently-injected partial path).
///
/// `existing_pythonpath` is the value already in the merged env about
/// to be passed to the child — either operator-explicit
/// `[sidecar_channels.env]` or inherited from the daemon's own env.
/// When set, our extracted dir is **prepended** so the user's
/// PYTHONPATH still wins for any module they're explicitly overriding;
/// our entry only provides resolution for names that nobody else
/// claimed.
pub(crate) fn pythonpath_with_embedded(
    command: &str,
    home_dir: &Path,
    existing_pythonpath: Option<&str>,
) -> Option<String> {
    if !command_is_python_interpreter(command) {
        return None;
    }
    if has_real_sdk_installed(command) {
        debug!(
            command,
            "librefang-sdk already importable by interpreter; skipping embedded fallback"
        );
        return None;
    }
    let extracted = match ensure_extracted(home_dir) {
        Ok(p) => p,
        Err(e) => {
            warn!(
                home = %home_dir.display(),
                "Failed to extract embedded librefang-sdk: {e}; sidecar spawn will rely on system install (which will likely fail)"
            );
            return None;
        }
    };
    let entry = extracted.to_string_lossy().into_owned();
    let composed = match existing_pythonpath {
        Some(existing) if !existing.is_empty() => {
            format!("{entry}{PYTHONPATH_SEP}{existing}")
        }
        _ => entry,
    };
    Some(composed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn embedded_tree_contains_sidecar_package() {
        // Sanity: the embed picked up the expected layout from
        // sdk/python/librefang/. If this breaks, either `include_dir!`
        // failed to find the path, or the SDK was restructured —
        // either way the supervisor's fallback is broken.
        assert!(
            EMBEDDED_SDK.get_file("__init__.py").is_some(),
            "expected librefang/__init__.py in embed"
        );
        assert!(
            EMBEDDED_SDK.get_dir("sidecar").is_some(),
            "expected librefang/sidecar/ in embed"
        );
    }

    #[test]
    fn hash_is_stable_and_short() {
        let h1 = embedded_hash();
        let h2 = embedded_hash();
        assert_eq!(h1, h2, "hash must be deterministic across calls");
        assert_eq!(h1.len(), 12);
        assert!(
            h1.chars().all(|c| c.is_ascii_hexdigit()),
            "hash must be lowercase hex"
        );
    }

    #[test]
    fn skip_filter_drops_pycache_and_pyc() {
        assert!(should_skip(Path::new("sidecar/__pycache__/x.pyc")));
        assert!(should_skip(Path::new(
            "__pycache__/__init__.cpython-313.pyc"
        )));
        assert!(should_skip(Path::new("foo/bar.pyc")));
        assert!(should_skip(Path::new(".DS_Store")));
        assert!(!should_skip(Path::new("sidecar/adapters/telegram.py")));
        assert!(!should_skip(Path::new("__init__.py")));
    }

    #[test]
    fn extract_writes_complete_tree_and_marker() {
        let tmp = TempDir::new().unwrap();
        let out = ensure_extracted(tmp.path()).expect("extract ok");
        assert!(out.join(".complete").exists(), "marker missing");
        assert!(out.join("librefang/__init__.py").exists(), "init missing");
        assert!(
            out.join("librefang/sidecar/__init__.py").exists(),
            "sidecar package missing"
        );
        // No pycache pollution made it out. Walk manually so we don't
        // pull in an extra dev-dep for one assertion.
        fn scan_for_pycache(dir: &Path) -> bool {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return false;
            };
            for entry in rd.flatten() {
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if ft.is_dir() {
                    if entry.file_name() == "__pycache__" {
                        return true;
                    }
                    if scan_for_pycache(&entry.path()) {
                        return true;
                    }
                }
            }
            false
        }
        assert!(
            !scan_for_pycache(&out),
            "should_skip failed to keep __pycache__ out"
        );
    }

    #[test]
    fn extract_is_idempotent_when_marker_present() {
        let tmp = TempDir::new().unwrap();
        let first = ensure_extracted(tmp.path()).unwrap();
        let mtime_before = std::fs::metadata(first.join("librefang/__init__.py"))
            .unwrap()
            .modified()
            .unwrap();
        // Second call: marker exists, must not rewrite files.
        let second = ensure_extracted(tmp.path()).unwrap();
        assert_eq!(first, second);
        let mtime_after = std::fs::metadata(second.join("librefang/__init__.py"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "files must not be rewritten on cached extract"
        );
    }

    #[test]
    fn extract_recovers_from_torn_previous_run() {
        // Simulate a crash mid-extract: directory exists, files
        // partially written, marker absent. Next call must redo the
        // work and end up with a valid tree.
        let tmp = TempDir::new().unwrap();
        let hash = embedded_hash();
        let target = tmp.path().join("sidecar-python").join(hash);
        std::fs::create_dir_all(target.join("librefang")).unwrap();
        std::fs::write(target.join("librefang/garbage.py"), b"truncated").unwrap();
        // No `.complete` marker.

        let out = ensure_extracted(tmp.path()).expect("recovers");
        assert!(out.join(".complete").exists());
        assert!(out.join("librefang/__init__.py").exists());
    }

    #[test]
    fn command_python_detector_accepts_canonical_names() {
        assert!(command_is_python_interpreter("python"));
        assert!(command_is_python_interpreter("python3"));
        assert!(command_is_python_interpreter("python3.12"));
        assert!(command_is_python_interpreter("python3.13"));
        assert!(command_is_python_interpreter("/usr/local/bin/python3"));
        assert!(command_is_python_interpreter(
            "/opt/homebrew/bin/python3.12"
        ));
        // Windows: `.exe` is stripped by `file_stem`.
        assert!(command_is_python_interpreter("C:\\Python313\\python.exe"));
    }

    #[test]
    fn command_python_detector_rejects_wrappers_and_garbage() {
        assert!(!command_is_python_interpreter("uv"));
        assert!(!command_is_python_interpreter("bash"));
        assert!(!command_is_python_interpreter("pypy"));
        assert!(!command_is_python_interpreter("python3.x"));
        assert!(!command_is_python_interpreter("python-thing"));
        assert!(!command_is_python_interpreter(""));
    }

    #[test]
    fn pythonpath_composition_prepends_extracted_dir() {
        // Drives the no-real-sdk branch via an interpreter path whose
        // basename matches the Python detector (`python3`) but whose
        // file does NOT exist, so `has_real_sdk_installed` reliably
        // returns false (subprocess spawn fails) regardless of the
        // test host's actual python3.
        let tmp = TempDir::new().unwrap();
        let fake_python = tmp.path().join("nonexistent-bin-dir").join("python3");
        let result = pythonpath_with_embedded(
            fake_python.to_str().unwrap(),
            tmp.path(),
            Some("/operator/explicit/path"),
        );
        let composed = result.expect("should compose when sdk absent");
        let sep = PYTHONPATH_SEP;
        assert!(
            composed.ends_with(&format!("{sep}/operator/explicit/path")),
            "operator PYTHONPATH must be preserved at the tail: got {composed}"
        );
        let extract_target = tmp.path().join("sidecar-python").join(embedded_hash());
        assert!(
            composed.starts_with(&extract_target.to_string_lossy().to_string()),
            "extracted dir must be prepended: got {composed}"
        );
    }

    /// End-to-end: extract, then spawn the host's `python3` with
    /// `-S` (skip `site-packages`) and `PYTHONPATH=<extracted_dir>`.
    /// The `-S` flag is essential — it hides whatever developer
    /// install lives in this machine's site-packages, so a green
    /// outcome here can only mean "the extracted tree is itself
    /// importable", which is exactly the new-user case we are
    /// trying to defend.
    #[test]
    fn extracted_tree_imports_under_isolated_python() {
        // Skip if no python3 is available — channel sidecar usage
        // requires one regardless, but the embed path is a fallback,
        // not a hard dep on the test runner. Channels CI runs on
        // Ubuntu / macOS / Windows, all of which ship a python3.
        let python = if Command::new("python3")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            "python3"
        } else {
            return;
        };
        let tmp = TempDir::new().unwrap();
        let extracted = ensure_extracted(tmp.path()).expect("extract ok");
        // Probe a real adapter module so we exercise more than the
        // top-level package init — telegram.py imports from
        // `librefang.sidecar.{common,protocol,runtime,formatter}`,
        // which catches a botched recursive copy where intermediate
        // package `__init__.py` files went missing.
        let status = Command::new(python)
            .args([
                "-S",
                "-c",
                "import librefang.sidecar; \
                 import librefang.sidecar.adapters.telegram; \
                 print(librefang.__file__)",
            ])
            .env("PYTHONPATH", &extracted)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("spawn python3");
        assert!(
            status.success(),
            "extracted tree must be importable under `python3 -S` with \
             PYTHONPATH={} — got exit {:?}",
            extracted.display(),
            status.code()
        );
    }

    #[test]
    fn pythonpath_skips_non_python_commands() {
        let tmp = TempDir::new().unwrap();
        // Wrapper commands → no injection, no extraction triggered.
        assert!(pythonpath_with_embedded("uv", tmp.path(), None).is_none());
        assert!(pythonpath_with_embedded("bash", tmp.path(), Some("/x")).is_none());
        // And no extraction happened as a side effect.
        assert!(
            !tmp.path().join("sidecar-python").exists(),
            "non-python command must not trigger extraction"
        );
    }
}
