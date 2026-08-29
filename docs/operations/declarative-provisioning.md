# Declarative resource provisioning

Managed configuration mode ([`managed-config.md`](managed-config.md)) locks `config.toml` as a whole: the deployment owns the file, and every API route that would persist into it answers `423 Locked`.

Declarative provisioning is the same idea one level down, for resources that do not live in `config.toml`.
A deployment-owned directory tree declares agents; the daemon reconciles it at boot; each declared agent is locked individually, and everything an operator creates at runtime stays editable.

The two features are independent.
Provisioning works in mutable mode, and a managed `config.toml` does not imply a provisioning tree.
A Kubernetes deployment normally wants both, and the same ConfigMap can carry both.

## The tree

```text
/etc/librefang/provisioning/
  agents/
    researcher.toml
    triager.toml
```

Each `*.toml` under `agents/` is one agent, in exactly the format of an `agent.toml`.

The **resource identifier is the manifest's `name`**, not the file name.
A file name is documentation — `01-researcher.toml` and `researcher.toml` declare the same resource if both say `name = "researcher"`, and the reconcile reports the second one as a duplicate rather than provisioning two agents.

`name` must be present in the file.
`AgentManifest` deserialises a missing `name` to `"unnamed"`, which would quietly provision an agent by that name and make every subsequent typo collide with it, so the reconcile refuses a manifest that does not declare one.

## The two environment variables

| Variable | Effect | Default |
|---|---|---|
| `LIBREFANG_PROVISIONING_PATH` | Absolute path of the provisioning root. Setting it switches the feature on. | unset — provisioning is off |
| `LIBREFANG_PROVISIONING_PRUNE` | `delete` removes a resource whose declaration left the tree. Any other value, including unset, releases it instead. | `keep` |

Both are read from the process environment and never from `config.toml`, for the same reason `LIBREFANG_CONFIG_MODE` is: a setting that decides what may be written must not itself be writable through the surface it governs.
A `[provisioning]` key would be settable from the dashboard in mutable mode, and a write that turns provisioning off is a write that unlocks every provisioned resource.

Only the exact word `delete` is destructive, case-insensitively and after trimming.
A typo resolves to `keep`, because a typo must never be the reason an agent is deleted.

## What a reconcile does

It runs once per boot, after the registry is restored from SQLite and before the "no agents exist, spawn a default assistant" fallback — a deployment that declares its own agents does not also get an `assistant` it never asked for.

For each declared agent:

| Situation | Action |
|---|---|
| Nothing by that name exists | **create** |
| Provisioned before, still here, file byte-identical | **unchanged** — no write at all |
| Provisioned before, file changed | **apply** the new manifest to the existing agent |
| Exists but was never provisioned | **adopt** — apply, and record the takeover |
| Provisioned before but the agent is gone | **create** again |

Adoption is `kubectl apply` semantics and is deliberately not treated as a conflict: a deployment declaring an agent named `researcher` is the deployment claiming `researcher`, whether or not something created it first.
The manifest is replaced; the agent's id, session, history and identity survive.

An **orphan** is a resource this daemon provisioned before whose file is no longer in the tree.
Under the default `keep` policy it is **released** — its provenance is dropped, it keeps running, and it becomes editable again, with one `WARN` naming it.
Releasing rather than remembering is what makes the removal reversible: putting the file back re-adopts the same agent instead of colliding with a tombstone.
Under `delete` the agent is killed, the same teardown `DELETE /api/agents/{id}` performs.

A reconcile is idempotent.
Rebooting with an unchanged tree performs no writes and logs nothing beyond a single summary line.

### The provisioning tree is an input, never a write target

A provisioned agent's `source_toml_path` is **not** set to its declaring file.
The agent's own workspace `agent.toml` stays the materialised copy that the daemon writes, so nothing ever tries to write back into a read-only mount — the failure mode managed mode's migration write-back was fixed to avoid.

### Failures never fail the boot

One file the reconcile cannot use — unreadable, not UTF-8, not valid TOML, no `name`, a duplicate name — is recorded and skipped; the rest of the tree still applies.
Refusing to start over one malformed manifest would turn an operator's typo into an outage.

A subdirectory of the root that is not `agents/` is reported as a failure too.
The RFC's tree also names `channels/` and `workflows/`; neither is implemented, and naming them tells the operator that rather than letting a deployment believe it had provisioned channels.

## What is locked

A provisioned agent's **definition** cannot be changed through the API.
These routes answer `423 Locked`:

- `PATCH /api/agents/{id}`
- `DELETE /api/agents/{id}` — and the entry is refused per item in `DELETE /api/agents/bulk`, so one deployment-owned agent does not fail the operator's other deletes
- `PUT /api/agents/{id}/model`
- `PUT /api/agents/{id}/tools`
- `PUT /api/agents/{id}/skills`
- `PUT /api/agents/{id}/mcp_servers`
- `PUT /api/agents/{id}/channels`
- `PATCH /api/agents/{id}/config`
- `PATCH /api/agents/{id}/identity`

The body is the same envelope shape managed mode uses, so a client handles both with one branch:

```json
{
  "ok": false,
  "error": "this resource is provisioned by the deployment",
  "code": "resource_provisioned",
  "kind": "agent",
  "name": "researcher",
  "source": "/etc/librefang/provisioning/agents/researcher.toml"
}
```

The `code` differs from `config_managed` because the remedy differs: one is fixed by editing `config.toml`, the other by editing a file in the provisioning tree and rolling the daemon.

## What is not locked

**Operating** a provisioned agent is unrestricted.
Suspend, resume, stop, message, sessions, history, files, logs and every read stay available, because those change runtime state the deployment never declared.

The dividing line is "would the next reconcile overwrite this".
If yes, the write is a lie and is refused. If no, it is the operator's to make.

**Everything the tree does not declare stays fully mutable.** Ownership is per resource, not a global switch — an agent created through `POST /api/agents` beside a provisioned one is edited and deleted exactly as before.

## Reading the state

`GET /api/provisioning/status` (authenticated, like every `/api/*` route):

```json
{
  "enabled": true,
  "root": "/etc/librefang/provisioning",
  "prune": "keep",
  "resources": [
    {
      "kind": "agent",
      "name": "researcher",
      "source": "/etc/librefang/provisioning/agents/researcher.toml",
      "checksum": "sha256:1f0c…",
      "applied_at": "2026-08-25T09:41:12+00:00",
      "source_checksum": "sha256:1f0c…",
      "drifted": false,
      "present": true
    }
  ],
  "failures": [],
  "report": {
    "created": 1,
    "applied": 0,
    "adopted": 0,
    "unchanged": 0,
    "pruned": 0,
    "released": 0,
    "failed": 0
  },
  "applied_at": "2026-08-25T09:41:12+00:00"
}
```

`checksum` is the declaration that is **in effect**; `source_checksum` is what the file says **now**.
`drifted` is true when they differ, which means the tree has moved on and the daemon has not been rolled — the resource-level equivalent of comparing a ConfigMap's `checksum/config` annotation against `GET /api/config/status`.
A drifted resource is not broken; it is running the previous declaration.

`present: false` means something removed the resource out of band. The next reconcile recreates it.

`failures[]` is the only place a refused file survives after the boot log has scrolled.

There is no write route here, by design.
A provisioning tree is deployment-owned, so the way to change it is to change the tree and roll the daemon — rollout-only, exactly as managed configuration is.

## Kubernetes

Add the tree to the same ConfigMap the managed config comes from, or a second one, and point the daemon at it:

```yaml
env:
  - name: LIBREFANG_PROVISIONING_PATH
    value: /etc/librefang/provisioning
volumeMounts:
  - name: provisioning
    mountPath: /etc/librefang/provisioning/agents
    readOnly: true
volumes:
  - name: provisioning
    configMap:
      name: librefang-agents
```

A ConfigMap's keys are mounted flat in one directory, which is why the mount points at `agents/` rather than at the root.

Updating an agent is the same procedure as updating the config: edit the file, bump the pod template so the rollout happens, `kubectl apply -k`, then confirm with `GET /api/provisioning/status` that `drifted` is `false` and `checksum` is what you expect.
See [`deploy/kubernetes/README.md`](../../deploy/kubernetes/README.md) for the checksum-annotation mechanics.

## Known gaps

- **Agents only.** `channels/` and `workflows/` from the RFC's tree are not implemented, and an unrecognised subdirectory is reported rather than ignored. Channels persist into `config.toml` itself and are already covered by the whole-config lock; workflows persist into a SQLite-backed registry with its own run state, so neither is a matter of pointing the same scan at another subdirectory.
- **Boot-time only.** There is no in-place reload of the tree and no filesystem watcher, matching the rollout-only contract managed configuration settled on.
- **CLI writes are not gated**, the same gap managed mode has: the guard is in the HTTP handlers, and `librefang` subcommands that mutate an agent manifest run in-process against the same files.
- **Ownership is per resource, not per field.** A provisioned agent's manifest is locked whole. Field-level overlays were considered and deliberately left out — see the design note in `managed-config.md`.
