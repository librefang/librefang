//! Append/replace a single `KEY=VALUE` line in ~/.librefang/secrets.env.
//!
//! The file is loaded into `std::env` at daemon boot by
//! `librefang_extensions::dotenv::load_dotenv()`; any non-system-env
//! KEY in this file becomes visible to sidecar child processes through
//! normal env inheritance. We only ever touch ONE line per call —
//! comments, ordering, and unrelated keys are preserved.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

static SECRET_WRITE_LOCK: Mutex<()> = Mutex::new(());

fn lock_secret_writes() -> MutexGuard<'static, ()> {
    match SECRET_WRITE_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Secret write lock was poisoned; recovering protected state");
            SECRET_WRITE_LOCK.clear_poison();
            poisoned.into_inner()
        }
    }
}

fn write_secret_staging_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Exclusive creation prevents a pre-positioned file or symbolic link at
    // the predictable staging path from being followed. An open failure means
    // this call never owned the path, so the caller must not remove it.
    let mut file = options
        .open(path)
        .map_err(|error| format!("create {path:?}: {error}"))?;
    let write_result = file
        .write_all(contents)
        .map_err(|error| format!("write {path:?}: {error}"))
        .and_then(|()| {
            file.sync_all()
                .map_err(|error| format!("sync {path:?}: {error}"))
        });
    if let Err(error) = write_result {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

pub fn upsert_secret(path: &Path, key: &str, value: &str) -> Result<(), String> {
    // The dotenv reader (`librefang_extensions::dotenv`) silently strips
    // a matched outer pair of `"..."` / `'...'` and processes escape
    // sequences `\n` / `\\` / `\"` inside double quotes. If we accepted
    // values that started with a quote, or that contained CR/LF/NUL, the
    // round-trip from write to read would corrupt the value: an operator
    // who typed `"abc"` would see `abc` come back. Leading/trailing
    // whitespace would likewise be lost by trim semantics common in
    // dotenv parsers. Reject those shapes loudly so the dashboard can
    // surface a useful message instead of producing silent corruption.
    if value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "secret value for `{key}` must not contain a newline or carriage return"
        ));
    }
    if value.contains('\0') {
        return Err(format!(
            "secret value for `{key}` must not contain a NUL byte"
        ));
    }
    if value.starts_with(char::is_whitespace) || value.ends_with(char::is_whitespace) {
        return Err(format!(
            "secret value for `{key}` must not have leading or trailing whitespace"
        ));
    }
    if value.starts_with('"') || value.starts_with('\'') {
        return Err(format!(
            "secret value for `{key}` must not start with a quote character (dotenv reader would strip it)"
        ));
    }
    // The key must never break the `KEY=VALUE\n` line framing. A key with an
    // interior newline (e.g. `FOO\nBAR`) passes the `=` / trim / empty checks
    // below — the newline is not at an edge — and would emit `FOO\nBAR=value`,
    // injecting an extra `BAR=value` line into secrets.env (which is loaded
    // into the process environment at boot and inherited by sidecar children).
    // Mirror the hardened `routes::skills::write_secret_env` key check.
    if key.contains('\n') || key.contains('\r') || key.contains('\0') {
        return Err(format!(
            "secret key `{key}` must not contain a newline, carriage return, or NUL byte"
        ));
    }
    if key.contains('=') || key.trim() != key || key.is_empty() {
        return Err(format!("invalid secret key `{key}`"));
    }

    // Serialize the complete read-modify-write transaction. Unique staging
    // names only prevent tempfile collisions; without this lock, concurrent
    // callers can both read the same original and the last rename silently
    // discards the other caller's key.
    let _write_guard = lock_secret_writes();

    let original = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("read {path:?}: {error}")),
    };
    let mut out = String::with_capacity(original.len() + key.len() + value.len() + 2);
    let mut replaced = false;
    for line in original.lines() {
        let trimmed = line.trim_start();
        if !replaced && !trimmed.starts_with('#') {
            if let Some((existing_key, _)) = trimmed.split_once('=') {
                if existing_key.trim() == key {
                    out.push_str(&format!("{key}={value}\n"));
                    replaced = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !replaced {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("{key}={value}\n"));
    }

    // Atomic write to a sibling tempfile then rename.
    let parent = path.parent().ok_or("secrets path has no parent dir")?;
    fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    // Disambiguate parallel callers: PID guards against other daemon
    // processes touching the same dir; the per-process atomic counter
    // guards against concurrent threads within this process (e.g. parallel
    // tests, or two HTTP handlers racing on the same secrets file).
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".secrets.env.tmp.{}.{seq}", std::process::id()));
    write_secret_staging_file(&tmp, out.as_bytes())?;

    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("rename {tmp:?} -> {path:?}: {error}"));
    }

    // `rename(2)` is atomic, but the directory entry it rewrites is only
    // guaranteed durable once the *directory's* metadata is synced — an
    // fsync of the file itself (above) does not cover that. Without this,
    // a crash right after a successful rename can roll the directory back
    // to pointing at the old inode (or, on some filesystems, no inode at
    // all) even though callers already observed `Ok(())`. Unix-only:
    // Windows durably commits renames as part of the NTFS transaction log
    // without a separate directory-handle fsync step.
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| format!("sync parent directory {parent:?}: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn key_with_newline_is_rejected_and_does_not_inject_a_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.env");

        let err = upsert_secret(&path, "FOO\nBAR", "value").unwrap_err();
        assert!(err.contains("newline"), "got: {err}");
        // Nothing must have been written.
        assert!(!path.exists() || !read(&path).contains("BAR=value"));
    }

    #[test]
    fn key_with_carriage_return_or_nul_is_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.env");
        assert!(upsert_secret(&path, "FOO\rBAR", "v").is_err());
        assert!(upsert_secret(&path, "FOO\0BAR", "v").is_err());
    }

    #[test]
    fn well_formed_key_still_writes_a_single_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.env");
        upsert_secret(&path, "OPENAI_API_KEY", "sk-123").unwrap();
        assert_eq!(read(&path), "OPENAI_API_KEY=sk-123\n");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "successful writes must not leave secret-bearing staging files"
        );
    }

    #[test]
    fn existing_secret_read_error_does_not_replace_target() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.env");
        // A directory at the target path reliably produces a non-NotFound
        // read error on every supported platform.
        fs::create_dir(&path).unwrap();

        let err = upsert_secret(&path, "OPENAI_API_KEY", "sk-123").unwrap_err();
        assert!(err.contains("read"), "got: {err}");
        assert!(path.is_dir(), "the unreadable target must remain untouched");

        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "a read failure must not create a secret-bearing staging file"
        );
    }

    #[test]
    fn preexisting_staging_file_is_not_overwritten_or_removed() {
        let dir = TempDir::new().unwrap();
        let staging = dir.path().join("staging");
        fs::write(&staging, b"owned by another writer").unwrap();

        let error = write_secret_staging_file(&staging, b"SECRET=value\n").unwrap_err();

        assert!(error.contains("create"), "got: {error}");
        assert_eq!(fs::read(&staging).unwrap(), b"owned by another writer");
    }

    #[cfg(unix)]
    #[test]
    fn preexisting_staging_symlink_is_not_followed_or_removed() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        let staging = dir.path().join("staging");
        fs::write(&target, b"safe").unwrap();
        std::os::unix::fs::symlink(&target, &staging).unwrap();

        assert!(write_secret_staging_file(&staging, b"SECRET=value\n").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"safe");
        assert!(fs::symlink_metadata(&staging)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn concurrent_upserts_preserve_every_key() {
        const WRITERS: usize = 16;
        let dir = TempDir::new().unwrap();
        let path = Arc::new(dir.path().join("secrets.env"));
        let barrier = Arc::new(Barrier::new(WRITERS));
        let threads: Vec<_> = (0..WRITERS)
            .map(|index| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    upsert_secret(&path, &format!("KEY_{index}"), &format!("value-{index}"))
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap().unwrap();
        }

        let content = read(&path);
        for index in 0..WRITERS {
            assert!(
                content
                    .lines()
                    .any(|line| line == format!("KEY_{index}=value-{index}")),
                "missing KEY_{index} from {content:?}"
            );
        }
        assert_eq!(content.lines().count(), WRITERS);
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "concurrent writes must not leave staging files"
        );
    }

    #[test]
    fn poisoned_secret_write_lock_recovers() {
        let panic = std::thread::spawn(|| {
            let _guard = SECRET_WRITE_LOCK.lock().unwrap();
            panic!("poison secret write lock");
        })
        .join();
        assert!(panic.is_err());

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.env");
        upsert_secret(&path, "RECOVERED", "true").unwrap();

        assert_eq!(read(&path), "RECOVERED=true\n");
        assert!(!SECRET_WRITE_LOCK.is_poisoned());
    }
}
