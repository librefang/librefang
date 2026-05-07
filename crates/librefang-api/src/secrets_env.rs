//! Load `~/.librefang/secrets.env` into the current process environment.
//!
//! Background — see #4701: the dashboard endpoint
//! `POST /api/providers/{name}/key` (`routes/providers.rs::set_provider_key`)
//! writes provider API keys to `<home>/secrets.env` and `set_var`s the running
//! process so the in-memory driver chain picks them up. The packaged systemd
//! unit (`deploy/librefang.service`) and the user-level unit produced by
//! `librefang service install` (`librefang-cli/src/main.rs::service_install_linux`)
//! both reference a different file (`<home>/env` or `/etc/librefang/env`), so
//! the next `systemctl restart` boots a daemon that has never seen the
//! dashboard-saved key. The provider then 401s with the empty `Bearer ` header
//! the driver builds.
//!
//! This module is the bootstrap loader: a parser + two thin entry points
//! (sync, for use before any tokio runtime exists; async, for hot-reload from
//! within a running runtime). The `channel_bridge` reload path used to inline
//! the same parser — it now delegates here so the two paths cannot drift.

use std::path::Path;

/// Parse a `KEY=value` env file into ordered `(key, value)` pairs.
///
/// Skips blank lines and `#` comments, trims surrounding whitespace, and
/// strips a single matched pair of `"…"` or `'…'` quotes from the value.
/// Lines without `=` and lines whose key is empty after trimming are dropped.
/// Order is preserved so callers see the same `set_var` sequence the file
/// declared (later entries with a duplicate key win — same as systemd).
pub fn parse_secrets_env(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq_pos].trim();
        if key.is_empty() {
            continue;
        }
        let mut value = trimmed[eq_pos + 1..].trim().to_string();
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = value[1..value.len() - 1].to_string();
        }
        out.push((key.to_string(), value));
    }
    out
}

/// Synchronously load `<home>/secrets.env` into `std::env`.
///
/// Intended for `cmd_start` in the CLI — call this **before** constructing a
/// tokio runtime and **before** `LibreFangKernel::boot`, so the driver chain
/// the kernel builds reads the just-loaded keys. Returns the number of vars
/// set, `Ok(0)` if the file is absent.
///
/// Safety: the underlying `std::env::set_var` is unsound when another thread
/// concurrently reads the environment. The contract here is that the caller
/// has not yet spawned any other thread that touches `std::env`. The CLI
/// `cmd_start` path satisfies this — the detached-spawn parent returns
/// before reaching this loader (the spawn loop calls `return` once the child
/// is up), so only the foreground or `--spawned` child invocation actually
/// runs the loader, and both call it from `main()` before the tokio runtime
/// is built.
pub fn load_into_process_blocking(home: &Path) -> std::io::Result<usize> {
    let path = home.join("secrets.env");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let entries = parse_secrets_env(&content);
    let n = entries.len();
    for (k, v) in entries {
        // SAFETY: caller contract — no other thread is reading std::env yet.
        unsafe { std::env::set_var(k, v) };
    }
    Ok(n)
}

/// Async variant — re-load `<home>/secrets.env` from inside a running tokio
/// runtime. The `set_var` calls run on a dedicated `spawn_blocking` thread so
/// they do not race tokio workers. Returns the number of vars set; a missing
/// file or read error logs and returns 0 (callers treat it as a no-op).
///
/// Used by `channel_bridge::reload_channels_from_disk` so a dashboard edit
/// that adds a fresh provider key is visible to the rebuilt channel adapters
/// without restarting the daemon.
pub async fn load_into_process_async(home: &Path) -> usize {
    let path = home.join("secrets.env");
    if !path.exists() {
        return 0;
    }
    match tokio::task::spawn_blocking(move || {
        let content = std::fs::read_to_string(&path).ok()?;
        let entries = parse_secrets_env(&content);
        let n = entries.len();
        for (k, v) in entries {
            // SAFETY: spawn_blocking serialises the env mutation against
            // tokio worker threads that might read std::env concurrently.
            unsafe { std::env::set_var(k, v) };
        }
        Some(n)
    })
    .await
    {
        Ok(Some(n)) => n,
        Ok(None) => 0,
        Err(e) => {
            tracing::warn!("spawn_blocking for secrets.env reload failed: {e}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kv_skipping_comments_blanks_and_invalid_lines() {
        let content = "\
# header
FOO=bar

  BAZ = qux
QUOTED=\"hello world\"
SINGLE='abc'
EMPTY=
=oops
no_equals_here
";
        let entries = parse_secrets_env(content);
        assert_eq!(
            entries,
            vec![
                ("FOO".into(), "bar".into()),
                ("BAZ".into(), "qux".into()),
                ("QUOTED".into(), "hello world".into()),
                ("SINGLE".into(), "abc".into()),
                ("EMPTY".into(), "".into()),
            ]
        );
    }

    #[test]
    fn duplicate_keys_keep_order_so_last_wins_when_caller_set_vars_in_sequence() {
        let entries = parse_secrets_env("K=first\nK=second\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1], ("K".into(), "second".into()));
    }

    #[test]
    fn unmatched_quote_is_left_intact() {
        let entries = parse_secrets_env("X=\"oops\nY='dangling\n");
        assert_eq!(entries.len(), 2, "both lines should parse");
        assert_eq!(entries[0], ("X".into(), "\"oops".into()));
        assert_eq!(entries[1], ("Y".into(), "'dangling".into()));
    }

    #[test]
    fn load_into_process_blocking_returns_zero_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let n = load_into_process_blocking(tmp.path()).unwrap();
        assert_eq!(n, 0);
    }

    /// Acceptance test for #4701: writing `secrets.env` and re-loading it must
    /// land the parsed keys in `std::env`, simulating a fresh daemon start.
    /// Var names are UUID-tagged so concurrent test binaries inside this crate
    /// cannot collide (project policy avoids global env mutation in shared
    /// tests; this module owns the loader, so the mutation is in-scope here).
    #[test]
    fn load_into_process_blocking_populates_std_env() {
        let tmp = tempfile::tempdir().unwrap();
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let key_a = format!("LIBREFANG_TEST_4701_A_{suffix}");
        let key_b = format!("LIBREFANG_TEST_4701_B_{suffix}");
        let content = format!("# header\n{key_a}=alpha\n  {key_b} = \"two words\"\n=oops\n");
        std::fs::write(tmp.path().join("secrets.env"), content).unwrap();

        let n = load_into_process_blocking(tmp.path()).unwrap();
        assert_eq!(n, 2, "two valid entries (header + empty-key dropped)");

        assert_eq!(std::env::var(&key_a).unwrap(), "alpha");
        assert_eq!(std::env::var(&key_b).unwrap(), "two words");

        // SAFETY: cleanup of the same UUID-tagged vars we just set; no other
        // thread reads these names — they are local to this test.
        unsafe {
            std::env::remove_var(&key_a);
            std::env::remove_var(&key_b);
        }
    }
}
