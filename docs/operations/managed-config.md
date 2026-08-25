# Managed configuration: letting the deployment own `config.toml`

By default LibreFang treats `config.toml` as application state.
The dashboard writes to it, several API routes persist into it, and boot-time schema migration rewrites it in place.
That is the right model for a desktop install or a single-operator box, and it is the default this page does not change.

It is the wrong model for a deployment where the configuration comes from a Kubernetes ConfigMap, a bind mount, or a configuration-management system.
There, the manifest is the source of truth, and a dashboard write is either lost on the next rollout or silently drifts the running config away from what the manifest says.
Managed mode makes that ownership explicit and enforces it server-side.

Resources that do not live in `config.toml` — agents — have their own deployment-owned contract: [`declarative-provisioning.md`](declarative-provisioning.md).
The two features are independent and compose; a Kubernetes deployment normally wants both.

## The two environment variables

| Variable | Effect | Default |
|---|---|---|
| `LIBREFANG_CONFIG_PATH` | Resolves `config.toml` to this exact path instead of `$LIBREFANG_HOME/config.toml`, for reads **and** writes. | unset — `$LIBREFANG_HOME/config.toml` |
| `LIBREFANG_CONFIG_MODE` | `managed` locks the file. Any other value, including unset, empty, and a typo, means mutable. | unset — mutable |

**They are independent on purpose.**
Relocating the file is not a statement about who owns it: a Compose deployment may reasonably bind-mount the config directory outside `LIBREFANG_HOME` and still want to edit it from the dashboard.
Inferring the lock from a custom path would hand that operator a read-only UI they never asked for, with no way out short of moving the file back.

A typo in `LIBREFANG_CONFIG_MODE` resolves to **mutable**, not managed.
Defaulting a typo to the locked mode would take the dashboard away from a deployment that never asked for it; defaulting it to mutable preserves existing behaviour, which is the compatibility guarantee.

**The mode is read from the process environment and never from the config file.**
A `config_mode` key inside `config.toml` would let an API write unlock the very file it is being refused access to.

## One resolution, recorded at boot

The path is resolved exactly once, in `LibreFangKernel::boot`, and stored on the kernel as `config_path_boot`.
Everything downstream reads it back through `KernelApi::config_path()`: the hot-reload path, the 30-second change watcher, the `source` field of `GET /api/config/status`, the `source` in every `423` body, and every route that persists into the file.
The CLI resolves through the same helper (`librefang_kernel::config::default_config_path`), so `librefang config set` inside the pod writes the file the daemon will reload.
`deploy/docker-entrypoint.sh` honours `LIBREFANG_CONFIG_PATH` too, which it did not until the Kubernetes overlay landed: it derived `$LIBREFANG_HOME/config.toml`, found that absent on a fresh PVC, and ran `librefang init` — which resolved the *mounted* file, took its upgrade branch because that file exists, and exited non-zero trying to merge new default sections into a read-only mount, on every boot.

This is worth stating because it was not true before #6695.
The loader honoured `LIBREFANG_CONFIG_PATH` while the writers re-derived `<home_dir>/config.toml`, so relocating the file produced a daemon that read the mount and wrote a second copy into `LIBREFANG_HOME` — one the next reload never looked at.
Managed mode's refusal named the mounted file; a *mutable* deployment with the same mount silently drifted instead.

Two consequences follow from the resolution order (`LIBREFANG_CONFIG_PATH` > `$LIBREFANG_HOME/config.toml` > `~/.librefang/config.toml`):

- **The filename does not have to be `config.toml`.**
  Three write paths used to reject anything else as an "invalid config file path" — a check that was meaningful when the path came from a request body and was pure obstruction once it came from the operator's own environment.
- **A `home_dir` key inside the config file does not move the config file.**
  It still relocates the data directory and everything else under the home, but the file cannot live at a path determined by its own contents; that is what used to send writes to a location the loader never read.

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
- `PATCH /api/memory/config`, which writes `[memory]` and `[proactive_memory]`.
- `POST /api/init`, which writes the whole file — though only when it does not already exist, which is not the managed case.
- The budget persistence path: `PUT /api/budget`, `PUT /api/providers/{name}/budget`, and the per-user budget routes.
- The user-persistence path: `POST` / `PUT` / `DELETE /api/users`, and key rotation.
- The provider routes that persist model and endpoint selection: `POST /api/providers/{name}/key`, `PUT /api/providers/{name}/url`, and `POST /api/providers/{name}/default`.
  They write `[default_model]`, `[provider_urls]` and `[provider_proxy_urls]`, which are deployment configuration under any reading.
- `POST /api/auth/change-password`, which writes `dashboard_user` and `dashboard_pass_hash`.
- The sidecar-channel routes: `POST /api/channels/sidecar/{name}/configure` and `DELETE /api/channels/sidecar/{name}`, which write `[[sidecar_channels]]`.
- The extension aliases: `POST /api/extensions/install` and `POST /api/extensions/uninstall`, which write `[[mcp_servers]]`.
- The MCP server routes — `POST` / `PUT /api/mcp/servers`, `DELETE /api/mcp/servers/{name}` and the taint patch — **but only under `mcp_runtime_store = "file"`**.
  See "MCP servers are locked per store, not per route" below.

That covers every route known to persist into `config.toml`.
The two entries that carry a condition rather than a flat refusal are explained in their own sections, because in both cases the condition is the point.

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

`POST /api/channels/sidecar/{name}/configure` is refused in full for the same reason and with the same shape.
It writes the adapter's non-secret fields into `[[sidecar_channels]]` and its secrets into `secrets.env` inside a single blocking call, so a guard at the config write would already have persisted the credential half before refusing.
In a managed deployment a sidecar channel is declared in the manifest and its secrets come from the pod environment.

## MCP servers are locked per store, not per route

`POST` / `PUT /api/mcp/servers` and `DELETE /api/mcp/servers/{name}` are refused under `mcp_runtime_store = "file"` (the default) and **left writable** under `mcp_runtime_store = "db"`.

The guard sits inside the store match in `persist_mcp_upsert` / `persist_mcp_delete` (`routes/skills/mcp.rs`) rather than at the top of each handler, because the two branches are not doing the same thing.
The `file` branch rewrites `config.toml`; the `db` branch writes the SQLite `mcp_server_configs` table and never opens the config file at all.
That branch exists precisely for read-only-`config.toml` deployments — it is what #6021 added it for.

Refusing it anyway would be over-locking: it would take the dashboard's whole MCP install surface away from a deployment that has *already* moved this persistence off the managed file, in the name of protecting a write that is not happening.
So the refusal doubles as the remedy. An operator who hits `423` here sets `mcp_runtime_store = "db"` in the manifest, rolls, and gets the install surface back with no drift, because nothing it writes lives in the file the manifest owns.

**The `db` escape does not extend to the extension aliases.**
`install_extension` / `uninstall_extension` (`routes/skills/extensions.rs`) call `upsert_mcp_server_config` / `remove_mcp_server_config` unconditionally, with no `mcp_runtime_store` check, so they rewrite `config.toml` even when the store is `"db"`.
While that remains true their guard has to be unconditional or it would miss the write it exists to stop.
A managed deployment that wants an install surface uses `POST /api/mcp/servers` with the `db` store.
The inconsistency itself is a `mcp_runtime_store` bug rather than a managed-mode one, and is tracked with #6021 rather than papered over here.

## What managed mode does *not* lock

Over-locking a managed deployment is its own failure, so the routes below stay writable deliberately.
Each is named with the reason, so the list can be argued with rather than assumed.

| Route | Why it stays writable |
|---|---|
| `GET`-shaped reads, and `POST /api/config/reload` | Reads. Reload re-reads the managed file; it does not write it. |
| `/api/mcp/servers/{name}/auth/*` (OAuth start / callback / revoke) | Tokens and any dynamically registered client credentials go to the encrypted vault (`KernelOAuthProvider`), never to `config.toml`. Locking these would leave a managed deployment able to *declare* an MCP server in its manifest but never able to authenticate it — the mode would break its own supported path. |
| `POST /api/mcp/servers/{name}/reconnect`, `POST /api/mcp/reload`, `POST /api/channels/reload` | Runtime actions against the live daemon. Nothing is persisted. |
| `/api/plugins/*` | Plugins live under `~/.librefang/plugins/`; no plugin route writes `config.toml`. |
| `PUT /api/agents/{id}/mcp_servers` and the rest of the per-agent surface | Writes the agent's `agent.toml` manifest, not `config.toml`. Managed mode's contract is the config file; extending it to per-agent manifests is a separate ownership question and would silently freeze agent management for every managed deployment. |
| `PUT /api/providers/{name}/discovery` | Writes `providers/{name}.toml`, a per-provider catalog fragment under `providers_dir`. |
| `DELETE /api/providers/{name}/key` | `secrets.env` only — see above. |

## Reading the mode from the API

`GET /api/config/status` (authenticated, like every `/api/*` route):

```json
{
  "mode": "managed",
  "source": "/etc/librefang/config.toml",
  "writable": false,
  "checksum": "sha256:9f2b…",
  "includes": [],
  "modified_at": "2026-08-04T09:41:12+00:00"
}
```

`writable` is the field to branch on.
It is equivalent to `mode == "mutable"`, exposed separately so a client uses a boolean rather than string-matching a mode name.

The checksum carries no value from inside the configuration.
Use it to confirm that a rollout actually replaced the file, rather than inferring it from a pod restart.

### What the checksum covers when the config uses `include`

With no `include`, the checksum is sha256 over the primary file's raw bytes, and `includes` is empty.
That is the same digest the field has always carried, so an existing `checksum/config` annotation keeps matching.

With `include = [...]`, hashing the primary file alone would be a false negative: an edit to an included file changes the effective configuration and leaves the primary byte-identical, so an operator using the checksum to confirm a rollout landed would be told nothing happened.
The checksum therefore covers the whole include closure, and `includes` lists the contributing files relative to the primary file's directory, in include order.

The composite is the digest of `sha256sum` output over those files, which reproduces as:

```bash
(cd /etc/librefang && sha256sum config.toml extra.toml) | sha256sum
```

`modified_at` is likewise the newest modification time across every contributing file, for the same reason.

The walk that finds those files is deliberately more forgiving than the loader: an include that is absolute, contains `..`, is missing, or forms a cycle is skipped rather than reported as an error, because this endpoint answers "what is in effect" and must not fail while the daemon is running.
A broken chain therefore shows up as a shorter `includes` list — the loader is where it is diagnosed.

## What the dashboard shows

The config editor reads `GET /api/config/status` and renders the whole page read-only when `writable` is `false`.

- A banner at the top of every category names the file the deployment owns and its checksum — the two facts an operator needs to go change the setting at its source and to confirm the change landed.
- Each field renders inside an `inert` container, so it is still readable but cannot be focused, typed into, or activated by keyboard.
  The values stay visible on purpose: being unable to *edit* a security setting is the point of the mode, being unable to *read* one is not.
- The per-field explanation switches to "this daemon's configuration is owned by the deployment".
  It deliberately does not reuse the existing non-writable-path copy, which tells the operator to "change it in `config.toml` and reload" — advice that is actively wrong for a file the next rollout overwrites.
- The "Save All" bar is suppressed, since nothing can become pending.

If the status query has not answered — an older daemon with no `/api/config/status`, or a transient failure — the page **stays editable**.
The lock is enforced server-side, so an optimistic UI costs at worst one honest `423` on save.
A pessimistic one would grey out every control on a daemon that is perfectly writable, with nothing on screen to explain why.

## Updating a managed configuration

**Rollout, not in-place reload.**
Change the ConfigMap, roll the StatefulSet — a checksum annotation on the pod template is the usual way to make the change trigger the rollout.

`deploy/kubernetes/overlays/managed-config/` is that shape as a working kustomize overlay: `config.toml` rendered into a ConfigMap, mounted read-only at `/etc/librefang/` outside the PVC, both environment variables set, and a `checksum/config` annotation carrying the same `sha256:` string this endpoint reports.
Pinning the annotation to the file's own digest rather than to kustomize's name-suffix hash makes the value that causes the rollout and the value that confirms it landed the same value, so `GET /api/config/status` answers "did my change reach the daemon?" with one string comparison.
`scripts/check-k8s-manifests.py` recomputes the digest from the rendered ConfigMap and fails when the annotation is stale, because a stale annotation applies cleanly and rolls nothing.
It also rejects `include = [...]` inside a managed ConfigMap: the checksum covers the primary file's bytes only, so an included file could change with the checksum unmoved, and the annotation this overlay is built around would stop meaning what it says.
That settles the `include` question for the overlay's own scope without changing what the daemon computes — `include` is unaffected in a mutable deployment.
The deployment procedure, the update procedure and the rollback procedure are in [`deploy/kubernetes/README.md`](../../deploy/kubernetes/README.md#managed-configuration).

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

- **Rotating the dashboard credential needs a rollout.**
  `POST /api/auth/change-password` is refused, so in a managed deployment `dashboard_user` / `dashboard_pass_hash` change in the manifest and nowhere else.
  This is the intended consequence of the lock rather than a limitation to work around — a password changed through the API would be reverted by the next rollout, which is a worse outcome than being told no.
  The guard runs *before* the current-password check, so the endpoint also stops being a password oracle in this mode.

- **Most of the guarded writers do not take `config_write_lock`**, so they can already race a `POST /api/config/set` in mutable mode — a pre-existing bug that predates managed mode and is orthogonal to it.
  The sidecar-channel routes are the exception: both acquire it around the rewrite (`routes/channels.rs`).
  The guarded provider routes do **not** take it, which is a narrower version of the same pre-existing race and is deliberately left alone rather than changed under a managed-mode fix.

- **One `.toml` write under `providers/` is intentionally outside the lock.**
  `PUT /api/providers/{name}/discovery` writes `discover_models` into `providers/{name}.toml` (`upsert_provider_discover_models` in `routes/providers.rs`), a per-provider catalog fragment under `providers_dir`.
  It is not `config.toml` and managed mode does not claim it.
  Likewise the skill-secret writes in `routes/skills/mod.rs` go through `atomic_write_secret_file` into a secrets file, not the config file.

- **The lock is whole-config, not field-level.**
  There is no way to declare "the deployment owns `[external_auth]` but the dashboard may still edit `[channels]`".
  The existing `WRITABLE_EXACT_PATHS` / `WRITABLE_SECTION_PREFIXES` allowlist is already a field-level model and is already load-bearing for security; layering a second, orthogonal ownership axis over it would produce a matrix where the interesting cases are the corners.
  Ownership is instead expressed at the *resource* level — see [`declarative-provisioning.md`](declarative-provisioning.md), where each declared agent is locked individually and everything else stays mutable.
  That is the granularity a deployment has actually asked for; per-field ownership within `config.toml` stays out until one needs it.
- **CLI writes are not gated.**
  `librefang config set` and friends still write the file — the file the daemon loaded, since #6695, but they write it.
  An operator running the CLI inside the pod is doing so deliberately; managed mode is about the API and dashboard surface that a user reaches without shell access.
