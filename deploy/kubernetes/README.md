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
Credentials move from `.env`/`environment:` entries to the two Secrets above; `LIBREFANG_API_KEY` is the env var the Kubernetes path uses to supply `api_key`, since `config.toml` lives inside the daemon's own writable data dir and cannot be mounted from a Secret.

## Observability

`deploy/docker-compose.observability.yml` (Prometheus / Tempo / Grafana / OTel collector) has no Kubernetes counterpart here yet.
The daemon exposes `/api/metrics` in Prometheus format on the same port, so a `ServiceMonitor` or a `prometheus.io/scrape` annotation is enough for metrics; tracing needs `OTEL_EXPORTER_OTLP_ENDPOINT` pointed at your collector.
