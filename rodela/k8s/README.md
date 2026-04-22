# LibreFang — Kubernetes Operator Runbook

## Prerequisites

- `kubectl` 1.27+ with access to the target cluster
- `kustomize` 5+ (or `kubectl` with built-in kustomize via `kubectl apply -k`)
- `docker buildx` for building multi-arch images
- Push access to `ghcr.io/rodela-ai/librefang`
- A k3s or k8s cluster with the **local-path-provisioner** StorageClass available

---

## Directory layout

```
deploy/k8s/
├── kustomization.yaml          # Root kustomize entry point
├── namespace.yaml              # librefang namespace
├── statefulset.yaml            # Single-replica StatefulSet
├── service.yaml                # ClusterIP service (port 4545)
├── ingress.yaml                # Ingress (edit host/TLS as needed)
├── pvc-backup.yaml             # Optional backup PVC
├── rbac-backup.yaml            # RBAC for backup jobs
├── secrets.env.example         # Template — copy to secrets.env (gitignored)
├── secrets.env                 # GITIGNORED — real secret values
├── librefang.service.reference # Systemd unit kept for entrypoint parity
├── scripts/
│   └── k8s-migrate-from-rpi.sh # One-shot RPi → k8s migration helper
└── config/
    ├── config.toml             # Main daemon config (ConfigMap)
    ├── aliases.toml            # Agent aliases (ConfigMap)
    ├── channels/               # Per-channel config files (ConfigMap)
    ├── providers/              # Per-provider config files (ConfigMap)
    └── integrations/           # Integration config files (ConfigMap)
```

---

## First-time setup (migrate from RPi)

### 1. Build and push the image

```bash
docker buildx build \
  --platform linux/amd64 \
  -f deploy/Dockerfile.k8s \
  --build-arg REPO_REF=main \
  -t ghcr.io/rodela-ai/librefang:k8s-latest \
  --push .
```

### 2. Run the migration script

```bash
bash deploy/k8s/scripts/k8s-migrate-from-rpi.sh
```

This script:
- SSH-pulls `config.toml`, `aliases.toml`, `channels/`, `providers/`, and `integrations/` from the RPi
- Strips secret values from `config.toml` (api keys, tokens, passwords)
- Writes the sanitized files into `deploy/k8s/config/`
- Creates `deploy/k8s/secrets.env` with the extracted secret values

### 3. Review what was extracted

```bash
git diff --stat deploy/k8s/config/
```

Check that no real secret values appear in the config files before committing.

### 4. Apply to the cluster

```bash
kubectl apply -k deploy/k8s
```

### 5. Wait for the pod to become ready

```bash
kubectl -n librefang wait --for=condition=ready pod/librefang-0 --timeout=180s
```

### 6. Verify

```bash
kubectl -n librefang port-forward svc/librefang 4545:4545 &
curl http://127.0.0.1:4545/api/health
```

---

## Editing config (day-to-day)

kustomize hashes ConfigMap contents — any edit triggers a rolling restart automatically.

```bash
# Edit a config file:
$EDITOR deploy/k8s/config/config.toml
git commit -am "config: adjust <setting>"
kubectl apply -k deploy/k8s   # kustomize hash changes → rolling restart
```

To edit secrets, update `deploy/k8s/secrets.env` (never commit it) and re-apply:

```bash
$EDITOR deploy/k8s/secrets.env
kubectl apply -k deploy/k8s
```

---

## Viewing logs

```bash
kubectl -n librefang logs statefulset/librefang -f
```

For historical logs with timestamps:

```bash
kubectl -n librefang logs statefulset/librefang --since=1h --timestamps
```

---

## Restoring from backup

1. Scale down the StatefulSet:
   ```bash
   kubectl -n librefang scale statefulset librefang --replicas=0
   ```
2. Find the backup:
   ```bash
   kubectl -n librefang exec -it <backup-pod> -- ls /backups
   ```
3. Restore the tarball into the PVC (seed pod or `kubectl cp`).
4. Scale back up:
   ```bash
   kubectl -n librefang scale statefulset librefang --replicas=1
   ```

---

## Rollback to RPi

If you need to fail back to the RPi immediately:

1. Scale down the k8s pod:
   ```bash
   kubectl -n librefang scale statefulset librefang --replicas=0
   ```
2. On the RPi, restart the systemd service:
   ```bash
   systemctl --user start librefang
   ```

Data written to the PVC while running in k8s will not automatically sync back
to the RPi — do a manual export first if needed.

---

## Known limitations

- **Single-replica only** — LibreFang uses SQLite with a single writer; do not
  increase `replicas` beyond 1.
- **`secrets.env` must stay gitignored** — never commit it; rotate secrets by
  editing the file and re-running `kubectl apply -k deploy/k8s`.
- **StorageClass `local-path`** — assumes k3s or a cluster with the
  [local-path-provisioner](https://github.com/rancher/local-path-provisioner)
  installed. Change `storageClassName` in `statefulset.yaml` for other clusters.
