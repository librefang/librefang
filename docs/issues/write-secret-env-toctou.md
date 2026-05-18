# [High] `write_secret_env` TOCTOU — provider API keys readable at 0644 before chmod 0600

**Severity:** High · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
`crates/librefang-api/src/routes/skills.rs:5462-5471`

```rust
std::fs::write(path, lines.join("\n") + "\n")?;
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!("Failed to set file permissions: {e}");
    }
}
```

## Problem
`std::fs::write` opens the file at the process umask (typically `0022` → resulting mode `0644`). For the duration between the `write` syscall completing and the `set_permissions` syscall executing, any local user can `cat ~/.librefang/secrets.env` and read every provider API key the daemon has stored.

This is exactly the bug `save_sessions` was hardened against in #3939/#3725 (`server.rs:948-987` uses `OpenOptions::mode(0o600)` on a temp file then atomic-renames). The secrets-write path missed that rewrite; the TOCTOU window re-opens on every "save key" dashboard action.

## Exploit
Multi-user box (shared CI runner, dev container, jump host). Attacker polls `secrets.env` and grabs `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / `GROQ_API_KEY` / `GITHUB_TOKEN` / channel bot tokens the instant a user adds them from the dashboard.

## Fix
Mirror `save_sessions`:

```rust
let tmp = path.with_extension("env.tmp");
let mut f = OpenOptions::new()
    .write(true).create(true).truncate(true)
    .mode(0o600)
    .open(&tmp)?;
f.write_all(content.as_bytes())?;
f.sync_all()?;
drop(f);
std::fs::rename(&tmp, path)?;
```

Apply the same treatment to `remove_secret_env`.

## Tests
- Strace / `inotify` test (Linux): file is never observed at mode `0644`.
- Unit: write a key, assert `metadata().mode() & 0o777 == 0o600` immediately.
