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

Covered today:

- `POST /api/config/set`.
- The budget persistence path: `PUT /api/budget`, `PUT /api/providers/{name}/budget`, and the per-user budget routes.
- The user-persistence path: `POST` / `PUT` / `DELETE /api/users`, and key rotation.
- The provider routes that persist model and endpoint selection: `POST /api/providers/{name}/key`, `PUT /api/providers/{name}/url`, and `POST /api/providers/{name}/default`.
  They write `[default_model]`, `[provider_urls]` and `[provider_proxy_urls]`, which are deployment configuration under any reading.

The refusal is not complete.
Several routes in other domains still write `config.toml` without passing through the guard; they are enumerated with their write sites in the "Known gaps" section below.
Read that section before relying on managed mode as a complete seal.

### One route is refused more broadly than `config.toml` alone

`POST /api/providers/{name}/key` is refused **in full**, including the `secrets.env` write it performs before it ever reaches `config.toml`.

That is wider than the rest of managed mode, so here is the reason.
The route's config write is *conditional*: it persists `[default_model]` only when the current default has no working key, or when an OpenRouter free model it was pinned to has been delisted.
Whether either branch fires depends on live daemon state the caller cannot see, so a guard placed at the config write would accept or refuse the identical request depending on when it arrived — and in the accepted-then-refused case it would already have rewritten `secrets.env` and mutated the running process environment before refusing.
Refusing the request up front keeps it atomic and keeps the contract simple to state: in a managed deployment, provider credentials come from the environment or a secret manifest, not from the dashboard.

Two consequences worth knowing before you enable managed mode:

- Adding a provider key through the dashboard or the API is not available.
  Set the provider's `*_API_KEY` variable in the pod environment (or via your secret manager) and roll.
- `DELETE /api/providers/{name}/key` is **not** refused, because it never touches `config.toml` — it removes the entry from `secrets.env` and suppresses the provider (`delete_provider_key` in `routes/providers.rs`).
  The asymmetry is deliberate rather than an oversight: managed mode's contract is the config file, and refusing the delete would extend the lock over a file the mode does not claim.

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
  These routes still rewrite `config.toml` in managed mode.
  Each is named with the call that performs the write, so the list can be checked rather than trusted:

  | Route | Write site | What it rewrites |
  |---|---|---|
  | `PATCH /api/memory/config` | `routes/memory.rs` — `std::fs::write(&config_path, …)` in `memory_config_patch` | `[memory]`, `[proactive_memory]` |
  | `POST` / `PUT /api/mcp/servers`, `DELETE /api/mcp/servers/{name}` | `routes/skills/mod.rs` — `upsert_mcp_server_config` / `remove_mcp_server_config`, reached from `routes/skills/mcp.rs` | `[[mcp_servers]]` |
  | `POST /api/extensions/install`, `POST /api/extensions/uninstall` | `routes/skills/extensions.rs` — the same two helpers | `[[mcp_servers]]` |
  | `POST /api/channels/sidecar/{name}/configure`, `DELETE /api/channels/sidecar/{name}` | `routes/sidecar_toml.rs` — `upsert_sidecar_block` / `remove_sidecar_block` | `[[sidecar_channels]]` |
  | `POST /api/auth/change-password` | `server.rs` — `change_password` | `dashboard_user`, `dashboard_pass_hash` |
  | `POST /api/init` | `routes/config/system.rs` — `quick_init` | the whole file, but only when it does not already exist |

  Two of those need a qualifier.
  The direct MCP-server routes (`POST` / `PUT /api/mcp/servers`, `DELETE /api/mcp/servers/{name}`) write `config.toml` under `mcp_runtime_store = "file"`, which is the **default**; setting `mcp_runtime_store = "db"` moves that persistence into SQLite so it never reaches the config file, and is worth doing in a managed deployment for that reason alone.
  That escape does **not** extend to the extension routes: `install_extension` / `uninstall_extension` in `routes/skills/extensions.rs` call `upsert_mcp_server_config` / `remove_mcp_server_config` unconditionally, with no `mcp_runtime_store` check, so they keep writing `config.toml` even when the store is `"db"`.
  `quick_init` returns `already_initialized` and writes nothing when the file exists, which is the normal case for a managed deployment mounting a ConfigMap — so it is a live gap in principle and close to unreachable in practice.

  These are gaps rather than oversights: locking them is a product decision, not a mechanical extension of the guard.
  Refusing `change-password` means a managed deployment can only rotate the dashboard password through a rollout; refusing the MCP and extension routes takes the dashboard's whole install surface away.
  Those trade-offs want a maintainer's call, and the remaining Kubernetes and dashboard work in #6695 is where it belongs.

- **Most of the writers above do not take `config_write_lock` either**, so they can already race a `POST /api/config/set` in mutable mode — a pre-existing bug that predates managed mode and is orthogonal to it.
  The sidecar-channel routes are the exception: both acquire it around the rewrite (`routes/channels.rs`).
  The guarded provider routes do **not** take it, which is a narrower version of the same pre-existing race and is deliberately left alone rather than changed under a managed-mode fix.

- **One `.toml` write under `providers/` is intentionally outside the lock.**
  `PUT /api/providers/{name}/discovery` writes `discover_models` into `providers/{name}.toml` (`upsert_provider_discover_models` in `routes/providers.rs`), a per-provider catalog fragment under `providers_dir`.
  It is not `config.toml` and managed mode does not claim it.
  Likewise the skill-secret writes in `routes/skills/mod.rs` go through `atomic_write_secret_file` into a secrets file, not the config file.

- **The lock is whole-config, not field-level.**
  There is no way to declare "the deployment owns `[external_auth]` but the dashboard may still edit `[channels]`".
  The existing `WRITABLE_EXACT_PATHS` / `WRITABLE_SECTION_PREFIXES` allowlist is already a field-level model and is already load-bearing for security; layering a second, orthogonal ownership axis over it would produce a matrix where the interesting cases are the corners.
  Revisit when a deployment actually needs it.
- **CLI writes are not gated.**
  `librefang config set` and friends still write the file.
  An operator running the CLI inside the pod is doing so deliberately; managed mode is about the API and dashboard surface that a user reaches without shell access.
