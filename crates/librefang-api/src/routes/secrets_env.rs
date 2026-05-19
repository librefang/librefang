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

pub fn upsert_secret(path: &Path, key: &str, value: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "secret value for `{key}` must not contain a newline"
        ));
    }
    if key.contains('=') || key.trim() != key || key.is_empty() {
        return Err(format!("invalid secret key `{key}`"));
    }

    let original = fs::read_to_string(path).unwrap_or_default();
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
    let tmp = parent.join(format!(
        ".secrets.env.tmp.{}.{seq}",
        std::process::id()
    ));
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| format!("open {tmp:?}: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perm = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&tmp, perm).map_err(|e| format!("chmod 600 {tmp:?}: {e}"))?;
        }
        f.write_all(out.as_bytes())
            .map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}
