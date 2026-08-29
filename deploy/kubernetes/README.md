# LibreFang on Kubernetes

Supported deployment shape: **one stateful replica.**
This is not a placeholder for a multi-replica setup that is coming later — it is the shape the current architecture actually supports, and running more than one replica against the same state is prevented rather than merely discouraged.
See [Support boundary](#support-boundary) for what breaks and [docs/architecture/multi-replica-rfc.md](../../docs/architecture/multi-replica-rfc.md) for what it would take to lift it.

The manifests live under `base/` and are Kustomize-based, so `kubectl` applies them with no extra tooling.

## Quick start

```bash
# 1. Namespace.
kubectl create namespace librefang

# 2. Authentication + vault Secret. The daemon binds 0.0.0.0 inside the pod
#    (the container's loopback is unreachable from the kubelet), and a
#    non-loopback bind refuses to start without configured authentication —
#    so this Secret is mandatory, not optional hardening.
kubectl -n librefang create secret generic librefang-auth \
  --from-literal=api-key="$(openssl rand -hex 32)" \
  --from-literal=vault-key="$(openssl rand -base64 32)" \
  --from-literal=dashboard-user=admin \
  --from-literal=dashboard-pass="$(openssl rand -hex 24)"

# 2b. If you enable `[external_auth]`, add the OAuth state-signing key too.
#     The daemon refuses to boot with external auth on and this missing or
#     malformed, because a per-process random key breaks every callback.
#     Same shape as vault-key: base64 decoding to exactly 32 bytes.
kubectl -n librefang patch secret librefang-auth --type=merge \
  -p "{\"stringData\":{\"state-secret\":\"$(openssl rand -base64 32)\"}}"

# 3. Provider keys. Optional — skip entirely for a local-model-only cluster.
kubectl -n librefang create secret generic librefang-providers \
  --from-literal=anthropic-api-key="$ANTHROPIC_API_KEY"

# 4. Apply.
kubectl -n librefang apply -k deploy/kubernetes/base

# 5. Watch it come up. First boot runs `librefang init`, which writes
#    config.toml and seeds the agent registry, so the startupProbe may take
#    up to a minute on slow storage.
kubectl -n librefang rollout status statefulset/librefang

# 6. Reach the API.
kubectl -n librefang port-forward svc/librefang 4545:4545
curl -H "Authorization: Bearer $API_KEY" http://127.0.0.1:4545/api/agents
```

`secrets.example.yaml` in this directory documents every key name the StatefulSet reads.
It is a reference, not something to apply — filling it in and committing it puts credentials in git.

## Pod Security: `restricted`

The manifests satisfy the `restricted` Pod Security Standard as published, with no exemptions:

| Requirement | Where |
| --- | --- |
| `runAsNonRoot: true`, `runAsUser: 1001`, `runAsGroup: 1001` | pod `securityContext` |
| `allowPrivilegeEscalation: false` | container `securityContext` |
| `capabilities.drop: [ALL]` | container `securityContext` |
| `seccompProfile.type: RuntimeDefault` | pod `securityContext` |

Label the namespace to have the API server enforce it:

```bash
kubectl label namespace librefang \
  pod-security.kubernetes.io/enforce=restricted \
  pod-security.kubernetes.io/enforce-version=latest
```

The published image declares no `USER` directive, because under plain Docker its entrypoint starts as root to chown a bind-mounted volume and then drops to uid 1001 via `gosu`.
Pinning `runAsUser: 1001` here takes that root start away, and `deploy/docker-entrypoint.sh` detects the drop (`id -u` is not 0), skips both the chown and the `gosu` call, and execs the daemon directly.
One image serves both worlds; there is no separate rootless tag to track.

`readOnlyRootFilesystem` is deliberately not set. `restricted` does not require it, and the daemon writes outside `/data`: the python venv it provisions for MCP servers, plugin dependency installs, and `$TMPDIR` scratch during media conversion.

## Volume ownership

An unprivileged process cannot take ownership of a freshly provisioned volume, so something else has to hand it over.
The manifests use `fsGroup: 1001`, which makes the kubelet `chgrp` the volume to gid 1001 at mount time — the supported path, and enough for every in-tree CSI driver that reports `fsGroupPolicy: File` or `ReadWriteOnceWithFSType`.

`fsGroupChangePolicy: OnRootMismatch` keeps that relabelling off the critical path once ownership already matches, which matters as `/data` grows.

**Some drivers ignore `fsGroup` entirely** — most NFS and CIFS provisioners, and any driver reporting `fsGroupPolicy: None`.
On those the volume arrives owned by something the daemon cannot write, and the entrypoint exits non-zero with a message naming this section rather than letting SQLite fail later on an opaque `EACCES`.
Two ways out:

- Pre-own the volume as `1001:1001` out of band (a one-shot privileged Job, or the storage backend's own export options — e.g. NFS `all_squash` with `anonuid=1001,anongid=1001`).
- Use a StorageClass whose driver honours `fsGroup`.

Check what your driver claims:

```bash
kubectl get csidriver -o custom-columns=NAME:.metadata.name,FSGROUP:.spec.fsGroupPolicy
```

## Storage: `ReadWriteOnce` only

The PVC is `ReadWriteOnce`, and shared network storage is **not supported** for this workload unless you have explicitly validated its locking guarantees.

Runtime state under `/data` is a SQLite database in WAL mode, whose consistency depends on POSIX advisory locking and on `mmap`-visible shared-memory (`-shm`) semantics that NFS and CIFS commonly implement incorrectly or not at all.
The same applies to `/data/daemon.lock`, the `flock` that stops two daemons from sharing a state directory: on a filesystem where `flock` is a no-op, that safety check silently passes and both processes proceed to corrupt each other's writes.

A ReadWriteMany volume also does not buy anything here — it would let a second pod mount the data, which is exactly the thing that must not happen.

## Health contracts

Three probes, two contracts.
Confusing them causes restart loops.

| Probe | Endpoint | Meaning | On failure |
| --- | --- | --- | --- |
| `startupProbe` | `/api/ready` | First boot finished (`librefang init`, registry seed) | Pod restarted after 24 × 5 s |
| `livenessProbe` | `/api/health` | The process is responsive | **Pod restarted** |
| `readinessProbe` | `/api/ready` | The daemon can accept work | Pod removed from Service endpoints |

`/api/health` returns 200 whenever the HTTP server can answer, even when its body reports `status: degraded`.
That is intentional and is why it is safe as a liveness signal: a recoverable storage or provider incident must not get the pod killed and restarted into the same incident.

`/api/ready` returns 503 when a dependency required to accept work is unavailable — the SQLite substrate, or an embedding provider the operator pinned explicitly while leaving vector search on.
An unset or `"auto"` embedding provider is an optional enhancement and never fails readiness, because falling back to FTS text search is a supported mode rather than a degradation.

Both endpoints are public: the kubelet issues probes with no credential, and a 401 would pin the pod out of Service endpoints permanently.
Their payloads are minimal by design — check names and a coarse status, no version, hostname, provider id, or error text.
Detailed diagnostics stay behind the authenticated `/api/health/detail`.

## Updates and restarts

A StatefulSet terminates its pod before creating the replacement, so two daemons never contend for the volume.
State survives pod replacement because the PVC created from `volumeClaimTemplates` outlives the pod — and outlives the StatefulSet itself, which is why deleting the StatefulSet does not delete your agents.

`terminationGracePeriodSeconds: 60` covers the SQLite WAL checkpoint plus any in-flight agent turn.
The daemon stops accepting new work on SIGTERM; the window is for finishing what it already started, so a turn mid-LLM-call is not killed with its tool results unpersisted.

If you prefer a Deployment, it must set `strategy: Recreate`.
The default `RollingUpdate` briefly runs both pods, and the new one either fails to attach the ReadWriteOnce PVC or — on a driver that permits multi-attach — fails the `daemon.lock` flock.

## Managed configuration

By default the daemon treats `config.toml` as its own state: the dashboard writes to it, several API routes persist into it, and boot-time schema migration rewrites it in place.
On a cluster that is the wrong model — the manifest is the source of truth, and a dashboard write is either lost on the next rollout or silently drifts the running daemon away from what the manifest says.

`overlays/managed-config/` is the shape that fixes it.
It renders `config.toml` into a ConfigMap, mounts it read-only at `/etc/librefang/config.toml` (outside the PVC), and locks it server-side.
[docs/operations/managed-config.md](../../docs/operations/managed-config.md) is the full reference for what the mode does and does not lock; this section is the deployment procedure.

```bash
kubectl create namespace librefang
# The Secrets are exactly the same as the base quick start above — managed mode
# changes who owns config.toml, not where credentials come from.
kubectl -n librefang create secret generic librefang-auth \
  --from-literal=api-key="$(openssl rand -hex 32)" \
  --from-literal=vault-key="$(openssl rand -base64 32)" \
  --from-literal=dashboard-user=admin \
  --from-literal=dashboard-pass="$(openssl rand -hex 24)"

kubectl -n librefang apply -k deploy/kubernetes/overlays/managed-config
kubectl -n librefang rollout status statefulset/librefang
```

Confirm the mode took effect before relying on it — a typo in `LIBREFANG_CONFIG_MODE` resolves to **mutable**, silently and by design:

```bash
curl -H "Authorization: Bearer $API_KEY" http://127.0.0.1:4545/api/config/status
# {"mode":"managed","source":"/etc/librefang/config.toml","writable":false,
#  "checksum":"sha256:…","modified_at":"…"}
```

### What the overlay changes

| Piece | Why |
| --- | --- |
| `config.toml` rendered into a ConfigMap | The manifest, and the repository it lives in, become the source of truth. |
| Mounted read-only at `/etc/librefang` | Outside `/data`, so it survives no PVC state and shares no lifecycle with it. |
| `LIBREFANG_CONFIG_PATH=/etc/librefang/config.toml` | Relocation. Since #6695 every reader **and** writer resolves through it, so nothing deposits a second copy in `/data` that the daemon never reads. |
| `LIBREFANG_CONFIG_MODE=managed` | The lock. Every API route that would persist into the file answers `423 Locked` with `code: "config_managed"`. |
| `checksum/config` annotation on the pod template | Makes a config edit roll the StatefulSet, and is the same `sha256:` string `GET /api/config/status` reports back. |
| `agents/*.toml` rendered into a second ConfigMap | The deployment declares its agents the same way it declares its configuration. |
| `LIBREFANG_PROVISIONING_PATH=/etc/librefang/provisioning` | Switches declarative provisioning on. Each declared agent is locked individually with `code: "resource_provisioned"`; anything created at runtime stays editable. |
| `checksum/provisioning` annotation | Same contract for the declarations. The tree is reconciled at boot only, so an edit that does not roll the pod changes nothing. |

There is no init container, and nothing copies the file into `/data`.
The daemon resolves `LIBREFANG_CONFIG_PATH` itself and boots straight off the read-only mount; `scripts/check-k8s-manifests.py` fails the build if an `initContainers` block appears, because a writable duplicate under `/data` would be a second source of truth the deployment does not control.

### No credential belongs in the ConfigMap

A ConfigMap is stored unencrypted in etcd, is readable by anything with `get configmaps` in the namespace, and — for the copy in this repository — is in version control.
Credentials reach the daemon as environment variables sourced from Secrets, exactly as in the base manifests; `secrets.example.yaml` documents every key name.

What may appear in the ConfigMap is the *name* of a variable a provider reads (`api_key_env = "ANTHROPIC_API_KEY"`), which is a reference rather than a value.
`scripts/check-k8s-manifests.py` scans the rendered ConfigMap for `api_key`, `dashboard_pass`, `vault_key`, `state_secret`, `client_secret` and `password` assigned a literal, and fails the build on a hit.

Two consequences of the lock worth knowing before you enable it:

- **Provider keys cannot be added through the dashboard.**
  `POST /api/providers/{name}/key` is refused in full, because it writes `secrets.env` before it reaches `config.toml` and a guard at the config write would already have persisted the credential half.
  Set the provider's `*_API_KEY` in the `librefang-providers` Secret and roll.
- **Rotating the dashboard password needs a rollout.**
  `POST /api/auth/change-password` is refused, so `dashboard_user` / `dashboard_pass` change in the Secret and nowhere else.
  A password changed through the API would be reverted by the next rollout, which is worse than being told no.

The overlay also sets `mcp_runtime_store = "db"`, which keeps the dashboard's MCP install surface working by persisting those writes to SQLite on the PVC instead of to the managed file (#6021).
Leaving it at the default `file` is supported; it just means `POST /api/mcp/servers` answers `423` too.

### OAuth / OIDC

`[external_auth]` follows the same reference pattern: the config names an environment variable and the value comes from a Secret.

```toml
[external_auth]
enabled = true
provider = "google"
client_id = "…"                                    # not a secret
client_secret_env = "GOOGLE_OAUTH_CLIENT_SECRET"   # a reference, not a value
```

```yaml
- name: GOOGLE_OAUTH_CLIENT_SECRET
  valueFrom:
    secretKeyRef:
      name: librefang-auth
      key: oauth-client-secret
```

Add `LIBREFANG_STATE_SECRET` at the same time.
It is `optional: true` in the base manifests only while external auth is off, where an absent value means each boot derives a random per-process key and an in-flight login fails across a pod replacement.
With external auth on the daemon **refuses to boot** without it, so the `state-secret` key in `librefang-auth` stops being optional in practice.

Managed mode does not add a restriction here — `external_auth.*` was never writable through `/api/config/set` even in mutable mode, because flipping an endpoint or the verification gate post-authentication is the #3703 impersonation vector.
Managed mode only makes the file it lives in match that.

### Splitting the config across several files

One file is the simple case and what this overlay ships.
`include = [...]` also works, as long as every file it names is another key of the same ConfigMap.

The daemon resolves an include relative to the primary file's directory, and a directory-mounted ConfigMap puts every data key in that one directory, so `include = ["extra.toml"]` resolves exactly when `extra.toml` is in the same `configMapGenerator`:

```yaml
configMapGenerator:
  - name: librefang-config
    files:
      - config.toml
      - extra.toml
```

`scripts/check-k8s-manifests.py` verifies that, and fails the build on an include this manifest does not render, on a `/`-containing target (a ConfigMap key cannot contain one, so a subdirectory include can never resolve), on an absolute or `..` path (the daemon skips those silently), and on an `include` in a file mounted with a `subPath`, which exposes that one file and nothing else.

The checksum annotation then covers the whole set rather than the primary file alone, so editing any of them still rolls the StatefulSet.
Recompute it over the files in include order:

```bash
printf 'sha256:%s\n' "$( (cd deploy/kubernetes/overlays/managed-config && sha256sum config.toml extra.toml) | sha256sum | awk '{print $1}')"
```

`GET /api/config/status` reports that same string, and lists the included files in its `includes` field.

### Provisioned agents

The overlay also renders `agents/researcher.toml` into a `librefang-agents` ConfigMap and mounts it read-only at `/etc/librefang/provisioning/agents`.
`LIBREFANG_PROVISIONING_PATH` points the daemon at the root; the mount is one level down because a ConfigMap exposes its keys flat in a single directory.

The resource identifier is the manifest's `name`, not the file name.
At boot the daemon creates each declared agent, applies a changed declaration to the agent that already exists, and leaves an unchanged one alone.
Afterwards every API route that would rewrite that agent's manifest answers `423 Locked`; suspending, resuming and messaging it are untouched.

Removing a declaration **releases** the agent by default — it keeps running and becomes editable again.
Set `LIBREFANG_PROVISIONING_PRUNE=delete` (nothing else is destructive) if the tree should be the only source of agents.

Add a second agent by dropping a file in `agents/` and listing it in the `configMapGenerator`:

```yaml
  - name: librefang-agents
    files:
      - agents/researcher.toml
      - agents/triager.toml
```

Then regenerate the annotation over the whole set, in sorted key order:

```bash
printf 'sha256:%s\n' "$( (cd deploy/kubernetes/overlays/managed-config/agents && sha256sum *.toml) | sha256sum | awk '{print $1}')"
```

`scripts/check-k8s-manifests.py` recomputes it, and additionally fails the build on a declaration that is not valid TOML, declares no `name`, is not a `.toml` key, or assigns a credential a literal value — all of which the running daemon would otherwise turn into a missing agent visible only in `GET /api/provisioning/status`.

Confirm a rollout landed:

```bash
curl -sS -H "Authorization: Bearer $LIBREFANG_API_KEY" \
  http://127.0.0.1:4545/api/provisioning/status | jq '.resources[] | {name, drifted, present}'
```

`drifted: true` means the ConfigMap moved and the pod did not.
Full semantics: [`docs/operations/declarative-provisioning.md`](../../docs/operations/declarative-provisioning.md).

### Updating a managed configuration

**Rollout, not in-place reload.** Edit the config, re-hash it, apply:

```bash
cd deploy/kubernetes/overlays/managed-config
$EDITOR config.toml
# The annotation IS the rollout trigger.
# Without this line the pod template is unchanged, `kubectl apply -k` reports no change, and the daemon keeps the old file — so CI recomputes the hash and fails when the two disagree.
printf 'sha256:%s\n' "$(openssl dgst -sha256 -r config.toml | awk '{print $1}')"
$EDITOR statefulset-managed-config.yaml   # paste it into checksum/config

kubectl -n librefang apply -k deploy/kubernetes/overlays/managed-config
kubectl -n librefang rollout status statefulset/librefang
```

Then confirm the running daemon is on the file you edited, rather than inferring it from a restart count:

```bash
curl -sH "Authorization: Bearer $API_KEY" http://127.0.0.1:4545/api/config/status | jq -r .checksum
# must equal the checksum/config annotation you just set
```

The overlay pins `[reload] mode = "off"` for this reason.
The kubelet syncs an edited ConfigMap into the mount within about a minute whether or not anyone bumped the annotation; under the default `hybrid` the change watcher would notice that swap and hot-apply part of it, leaving a pod running configuration that matches neither the annotation on its own template nor any rollout anyone performed.
`POST /api/config/reload` still answers in this mode — it re-reads and validates the file and reports the plan — it just does not swap the live config.

In-place reload is deliberately not supported.
It would have to handle Kubernetes' atomic symlink swap without ever reading a half-written file, and then guarantee that an invalid new file never partially replaces the last valid effective configuration.
A rollout covers restart-required fields too, which is the superset.

### Rolling back

The ConfigMap is a plain Kubernetes object and the StatefulSet keeps its revision history, so a bad config has two exits.

**Preferred — revert the source and re-apply.** The manifest is the source of truth, so the rollback belongs there:

```bash
git revert <commit>        # or restore the previous config.toml and its checksum
kubectl -n librefang apply -k deploy/kubernetes/overlays/managed-config
kubectl -n librefang rollout status statefulset/librefang
```

**Faster — undo the rollout, then reconcile.** `kubectl rollout undo` restores the previous *pod template*, including the previous `checksum/config`:

```bash
kubectl -n librefang rollout history statefulset/librefang
kubectl -n librefang rollout undo statefulset/librefang --to-revision=<n>
```

This is a genuine partial rollback and the caveat matters: it does **not** revert the ConfigMap, which `apply` already replaced in place.
The pod comes back on a template whose annotation names the old checksum while the mount serves the new file, so `GET /api/config/status` reports a checksum that disagrees with the annotation — which is the drift the annotation exists to make visible.
Use it to stop the bleeding, then revert the source and re-apply so the two agree again.

If the bad config stops the daemon from booting at all, the pod will not become ready and the rollout will not complete; the previous pod is already gone, because a StatefulSet terminates before it re-creates.
Read the reason before undoing:

```bash
kubectl -n librefang logs statefulset/librefang --tail=100
```

An invalid `config.toml` is a boot-time parse failure with the offending key named, not a silent fallback to defaults.

## Support boundary

`replicas` must remain `1`.

The hard stop is `/data/daemon.lock`: `run_daemon` holds an exclusive flock, so a second pod sharing the volume cannot boot.
Giving each replica its own volume removes that error without fixing anything real, because the coordination problems are in the daemon, not the storage:

- Cron and trigger dispatch have no leader election, so every replica fires every job.
- `(agent_id, session_id)` execution ownership is process-local, so two replicas racing one session interleave writes to its history.
- Budget enforcement reads per-process metering state, so N replicas enforce roughly N× the configured cap.
- The audit hash chain has a single tip per process, so replicas diverge into unverifiable chains.

Those are architecture questions, not configuration ones. [docs/architecture/multi-replica-rfc.md](../../docs/architecture/multi-replica-rfc.md) enumerates every singleton subsystem and the coordination mechanism each would need.

## Migrating from Docker Compose

`deploy/docker-compose.yml` and these manifests run the same image with the same `/data` layout, so the migration is a volume copy:

```bash
# 1. Stop the Compose stack so nothing is mid-write.
docker compose -f deploy/docker-compose.yml down

# 2. Copy the volume out. `-a` preserves ownership; the tar is uid/gid 1001
#    inside, which is what the pod expects.
docker run --rm -v librefang-data:/data -v "$PWD":/backup alpine \
  tar -C /data -cf /backup/librefang-data.tar .

# 3. Create the PVC by applying the manifests, then copy the data in before
#    the daemon writes anything (scale to zero first).
kubectl -n librefang apply -k deploy/kubernetes/base
kubectl -n librefang scale statefulset/librefang --replicas=0
kubectl -n librefang run seed --rm -i --restart=Never \
  --image=alpine --overrides='{"spec":{"securityContext":{"runAsUser":1001,"runAsGroup":1001,"fsGroup":1001},"containers":[{"name":"seed","image":"alpine","command":["tar","-C","/data","-xf","-"],"stdin":true,"volumeMounts":[{"name":"data","mountPath":"/data"}]}],"volumes":[{"name":"data","persistentVolumeClaim":{"claimName":"data-librefang-0"}}]}}' \
  < librefang-data.tar
kubectl -n librefang scale statefulset/librefang --replicas=1
```

Compose deployments keep working unchanged — the entrypoint's root path is untouched.
Credentials move from `.env`/`environment:` entries to the two Secrets above; `LIBREFANG_API_KEY` is the env var the Kubernetes path uses to supply `api_key`.
Since #6695 `config.toml` no longer has to live inside the daemon's writable data dir — `LIBREFANG_CONFIG_PATH` will mount it anywhere — but it still must not carry credentials: the managed overlay puts it in a ConfigMap, which is unencrypted in etcd and in version control, so every secret keeps its env-var path.

## Observability

`deploy/docker-compose.observability.yml` (Prometheus / Tempo / Grafana / OTel collector) has no Kubernetes counterpart here yet.
The daemon exposes `/api/metrics` in Prometheus format on the same port, so a `ServiceMonitor` or a `prometheus.io/scrape` annotation is enough for metrics; tracing needs `OTEL_EXPORTER_OTLP_ENDPOINT` pointed at your collector.
