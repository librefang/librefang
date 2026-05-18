# [High] `POST /api/migrate` accepts arbitrary `source_dir` / `target_dir`

**Severity:** High · **Domain:** API attack surface · **Source:** `audit-02-api-attack-surface.md`

## Location
`crates/librefang-api/src/routes/config.rs:1779-1825`

```rust
PathBuf::from(req.source_dir.trim())
PathBuf::from(req.target_dir.trim())
```

No containment check — anything Admin-side can read/write.

## Problem
Admin-only route, but:
- The 200-vs-400 oracle leaks arbitrary filesystem paths (probe `.exists()` for anything readable as the daemon UID).
- `run_migrate` performs file writes under attacker-chosen directories — Admin can clobber `/etc/cron.d/`, `/var/spool/`, etc., effectively escalating from "config-write" to "filesystem-write under daemon UID".

## Fix
Require both paths canonicalize under `home_dir` or a configured `migrate.allowed_roots` allowlist:

```rust
fn validate_migrate_path(p: &Path, allowed: &[PathBuf]) -> Result<PathBuf> {
    let canon = p.canonicalize()?;
    if !allowed.iter().any(|root| canon.starts_with(root)) {
        bail!("path outside allowed migration roots");
    }
    Ok(canon)
}
```

## Tests
- `source_dir = "/etc"` → 400.
- `source_dir = "../../../etc"` → 400.
- Legit migration under `~/.openclaw` → 200.
