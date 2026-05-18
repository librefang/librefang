# [Low] Subprocess sandbox hardening — container naming, launcher PATH anchoring, script argv, unsafe-mode warning

**Severity:** Low
**Category:** Sandbox · Defense in depth
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | Low | `sanitize_container_name` replaces every disallowed character with `-`, then `safe_truncate_str(agent_id, 8)` → `"foo/bar"` and `"foo-bar"` collide |
| launcher PATH | Low | `bash` / `python` / `node` / `docker` launchers all go through `$PATH` with no absolute-path anchoring; `which_binary` on Unix treats an empty PATH segment as `.` |
| session-start script | Low | `on_session_start_script` uses unvalidated `agent_id` / `session_id` as argv and inherits the entire daemon environment |
| Full-mode warning | Info | `ExecSecurityMode::Full` skips every metachar / allowlist check and only emits a `warn!` |

## Affected files

- `crates/librefang-runtime-sandbox-docker/src/lib.rs:28-46` (container name)
- `crates/librefang-runtime-sandbox-docker/src/lib.rs:79, 106, 183, 230` (`docker` via PATH)
- `crates/librefang-skills/src/loader.rs:1052-1090` (`find_shell` / `find_python` / `find_node`)
- `crates/librefang-hands/src/registry.rs:1420-1443` (`which_binary` empty PATH segment)
- `crates/librefang-kernel/src/kernel/session_ops.rs:969-998` (`on_session_start_script`)
- `crates/librefang-runtime/src/subprocess_sandbox.rs:508-514` (`Full` mode)

## Why merged

All four are subprocess-sandbox-adjacent hygiene items; any PR will end up touching the config and launcher-lookup logic together.

## Combined fix plan

1. **Deterministic container naming (this)**: replace the lossy sanitize + truncate with a SHA-256 hex short prefix:
   ```rust
   let suffix = &format!("{:x}", Sha256::digest(agent_id.as_bytes()))[..8];
   ```
   This is bijective.
2. **Absolute-path anchoring for launchers (launcher PATH)**: at startup, resolve `bash` / `python` / `node` / `docker` from a vetted directory list (`/usr/bin`, `/usr/local/bin`, plus operator-configured), cache the absolute paths, and use them at runtime. `which_binary` must reject empty PATH segments rather than treating them as `.`.
3. **Harden session-start script (session-start script)**:
   - At `SessionId` / `AgentId` construction, reject any character outside `[A-Za-z0-9._:-]` (mirroring `plugin_manager.rs:55-79`);
   - On spawn, `env_clear()` + curated allowlist (mirroring `cron_script_wake_gate`).
4. **`Full` mode fail-loud (Full-mode warning)**: when `Full` is loaded, emit an `error!` startup banner; additionally require the CLI flag `--allow-unsafe-exec` to permit startup (not just a config-file toggle).

## Related

- [whatsapp-gateway-set-var-bypass-lock](whatsapp-gateway-set-var-bypass-lock.md) — `whatsapp_gateway.rs` shares the same `node`-launcher issue (covered by item (launcher PATH) above).

## Tests

- (this) Spawning agents with id `"foo/bar"` vs `"foo-bar"` → distinct container names.
- (launcher PATH) Boot with `$PATH` containing a writable `/tmp/x` before `/usr/bin` → still resolves to `/usr/bin/bash`; `/tmp/x/bash` is never picked up.
- (session-start script) `on_session_start_script` with an id containing `;` / `$` → construction fails.
- (Full-mode warning) Daemon starting with `Full` mode emits `error!` and refuses to start without `--allow-unsafe-exec`.
