# Managed configuration: letting the deployment own `config.toml`

By default LibreFang treats `config.toml` as application state.
The dashboard writes to it, several API routes persist into it, and boot-time schema migration rewrites it in place.
That is the right model for a desktop install or a single-operator box, and it is the default this page does not change.

It is the wrong model for a deployment where the configuration comes from a Kubernetes ConfigMap, a bind mount, or a configuration-management system.
There, the manifest is the source of truth, and a dashboard write is either lost on the next rollout or silently drifts the running config away from what the manifest says.
Managed mode makes that ownership explicit and enforces it server-side.

## The two environment variables

| Variable | Effect | Default |
|---|---|---|
| `LIBREFANG_CONFIG_PATH` | Loads `config.toml` from this exact path instead of `$LIBREFANG_HOME/config.toml`. | unset — `$LIBREFANG_HOME/config.toml` |
| `LIBREFANG_CONFIG_MODE` | `managed` locks the file. Any other value, including unset, empty, and a typo, means mutable. | unset — mutable |

**They are independent on purpose.**
Relocating the file is not a statement about who owns it: a Compose deployment may reasonably bind-mount the config directory outside `LIBREFANG_HOME` and still want to edit it from the dashboard.
Inferring the lock from a custom path would hand that operator a read-only UI they never asked for, with no way out short of moving the file back.

A typo in `LIBREFANG_CONFIG_MODE` resolves to **mutable**, not managed.
Defaulting a typo to the locked mode would take the dashboard away from a deployment that never asked for it; defaulting it to mutable preserves existing behaviour, which is the compatibility guarantee.

**The mode is read from the process environment and never from the config file.**
A `config_mode` key inside `config.toml` would let an API write unlock the very file it is being refused access to.

## What managed mode does

Every API surface that persists deployment configuration refuses with **`423 Locked`** and this body:

```json
{
  "ok": false,
  "error": "configuration is managed by the deployment",
  "code": "config_managed",
  "source": "/etc/librefang/config.toml"
}
```

The check runs **before** the handler reads the existing file, so a refused write never opens, truncates, or rewrites anything.

Enforcement lives in the handlers, not in the filesystem.
A read-only mount is still worth having as defence in depth, but it cannot be the mechanism: an `EACCES` surfaces as a `500` with an errno, which tells an operator nothing about *why* the write was refused, and it does not apply at all to a deployment that leaves the file writable while still expecting the manifest to win.

Covered today: `POST /api/config/set`, the budget persistence path (`PUT /api/budget`, `PUT /api/providers/{name}/budget`, and the per-user budget routes), and the user-persistence path (`POST/PUT/DELETE /api/users`, key rotation).

Not covered today, and honestly so: the skills, memory, and dashboard-credential routes also reach `config.toml`, and they do not yet take `config_write_lock` either — a pre-existing gap this change surfaces rather than introduces.
See the "Known gaps" section below before relying on managed mode as a complete seal.

## Reading the mode from the API

`GET /api/config/status` (authenticated, like every `/api/*` route):

```json
{
  "mode": "managed",
  "source": "/etc/librefang/config.toml",
  "writable": false,
  "checksum": "sha256:9f2b…",
  "modified_at": "2026-08-04T09:41:12+00:00"
}
```

`writable` is the field to branch on.
It is equivalent to `mode == "mutable"`, exposed separately so a client uses a boolean rather than string-matching a mode name.

The checksum is over the config file's raw bytes and carries no value from inside it.
Use it to confirm that a rollout actually replaced the file, rather than inferring it from a pod restart.

## Updating a managed configuration

**Rollout, not in-place reload.**
Change the ConfigMap, roll the StatefulSet — a checksum annotation on the pod template is the usual way to make the change trigger the rollout.

This is the only supported update path today, and it is chosen deliberately.
In-place reload of a ConfigMap has to handle Kubernetes' atomic symlink swap without ever reading a half-written file, and then guarantee that an invalid new file never partially replaces the last valid effective configuration.
A rollout works for restart-required fields too, which is the superset, so it is the contract worth supporting first.

`POST /api/config/reload` still works in managed mode — it re-reads the file, which is a read.
Whether a given field takes effect without a restart is unchanged and still answered by [`config-reload.md`](./config-reload.md).

## Schema migration under managed mode

When LibreFang loads a `config.toml` written against an older schema version it migrates it in memory and, in mutable mode, writes the migrated form back so later boots skip the work.

In managed mode that write is **skipped**, and a single warning is logged naming the file and both schema versions.

This is not a limitation to work around — it is the correct behaviour, and the alternative is worse.
Attempting the write against a read-only mount fails, and because the failure was only a `warn!` the migration would silently re-run on every boot forever with nothing but a repeating log line to show for it.
The in-memory config is migrated either way, so nothing is degraded at runtime.
What the warning tells you is that **your manifest is a schema version behind** and should be updated at the source.

## Known gaps

Stated plainly rather than left to be discovered:

- **Coverage is not yet complete.**
  The skills routes (`crates/librefang-api/src/routes/skills/mod.rs`), the memory route, and the dashboard-credential change in `server.rs` write `config.toml` without going through the guard.
  They also do not take `config_write_lock`, so they can already race a `config/set` in mutable mode — a pre-existing bug that predates managed mode.
- **The lock is whole-config, not field-level.**
  There is no way to declare "the deployment owns `[external_auth]` but the dashboard may still edit `[channels]`".
  The existing `WRITABLE_EXACT_PATHS` / `WRITABLE_SECTION_PREFIXES` allowlist is already a field-level model and is already load-bearing for security; layering a second, orthogonal ownership axis over it would produce a matrix where the interesting cases are the corners.
  Revisit when a deployment actually needs it.
- **CLI writes are not gated.**
  `librefang config set` and friends still write the file.
  An operator running the CLI inside the pod is doing so deliberately; managed mode is about the API and dashboard surface that a user reaches without shell access.
