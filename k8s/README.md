# BossFang Kubernetes Deployment

Kustomize-based deployment of the BossFang fork to GKE behind an Envoy
Gateway. Targets cluster `prometheus-461323/client-cluster/us-central1` at
hostname `bossfang.prometheusags.ai`.

## Layout

```
k8s/
├── gateway/                    # bossfang-gateway in envoy-gateway-system (applied first)
├── base/                       # Generic, reusable manifests (no env-specifics)
└── overlays/
    └── production-gke/         # Cluster-specific tweaks
```

The `gateway/` layer is applied once as a separate step because the
Gateway resource must live in `envoy-gateway-system` alongside
`prometheusags-wildcard-tls`, not in the `bossfang` namespace.
The `base/` layer is portable. The `production-gke/` overlay supplies the
namespace, resource sizing, and GKE StorageClass mapping.

## Topology

- **Namespace**: `bossfang`
- **SurrealDB**: dedicated `surrealdb/surrealdb:v3.0.5` StatefulSet, one
  replica, RocksDB on a 20 GiB PVC. Service on port `8000` (WebSocket +
  HTTP). Both `librefang-storage` (database `main`) and `surreal-memory`
  (database `memory`) connect remotely to this single instance — they
  share `StorageConfig` so they automatically route to different databases
  in the same namespace. No second SurrealDB or sidecar required.
- **BossFang**: one `Deployment` replica running the daemon
  (`librefang start --foreground`). Port `4545`. Persistent volume at
  `/data` for `BOSSFANG_HOME` (config.toml, vault, logs, daemon.lock,
  registry cache). Probes `/api/health`.
- **Gateway**: `bossfang-gateway` in `envoy-gateway-system`, using the
  existing `prometheusags-wildcard-tls` wildcard cert. Matches the cluster
  pattern (docuseal-gateway, document-designer-gateway, etc.). Two listeners:
  HTTP on 80 (redirect only) and HTTPS on 443 with TLS termination.
- **HTTPRoutes**: two routes in the `bossfang` namespace. HTTP→HTTPS redirect
  + HTTPS backend proxy to `bossfang:4545`. Both reference `bossfang-gateway`
  cross-namespace (`allowedRoutes.namespaces.from: All` permits this).
- **TLS**: terminates at the Gateway listener via `prometheusags-wildcard-tls`
  already present in `envoy-gateway-system`. No cert provisioning needed.

## Quick start (assuming kubectl is wired to the right cluster)

```bash
# 0. Make sure you're on the right cluster
gcloud container clusters get-credentials client-cluster \
    --region us-central1 --project prometheus-461323
kubectl config current-context   # should show the GKE cluster

# 1. Validate manifests locally (no cluster touch)
kubectl kustomize k8s/overlays/production-gke > /tmp/bossfang-manifests.yaml
head -20 /tmp/bossfang-manifests.yaml
wc -l /tmp/bossfang-manifests.yaml

# 2. Discover the wildcard cert location (read-only)
kubectl get gateway -A
kubectl get secret -A | grep -iE "wildcard|prometheusags"
kubectl get certificate -A 2>/dev/null

# 3. Populate the real secrets (NOT checked in)
#    See base/secrets.template.yaml for the keys you need.
cp k8s/base/secrets.template.yaml k8s/overlays/production-gke/secrets.yaml
$EDITOR k8s/overlays/production-gke/secrets.yaml
# Then add `- secrets.yaml` under resources: in overlay kustomization.yaml.
# NOTE: secrets.yaml is gitignored (see k8s/.gitignore).

# 4. Build the BossFang image and push to Artifact Registry
#    (separate CI workflow lands later — for the first deploy, build manually:)
docker build -t gcr.io/prometheus-461323/bossfang:$(git rev-parse --short HEAD) .
docker push gcr.io/prometheus-461323/bossfang:$(git rev-parse --short HEAD)

# 5. Pin the image tag in the overlay
$EDITOR k8s/overlays/production-gke/kustomization.yaml   # under `images:`

# 6. Server-side dry-run (no actual creates)
kubectl apply -k k8s/gateway --dry-run=server
kubectl apply -k k8s/overlays/production-gke --dry-run=server

# 7. Apply — two steps: gateway first, then the main overlay
kubectl apply -k k8s/gateway
kubectl apply -k k8s/overlays/production-gke
kubectl -n bossfang rollout status statefulset/surrealdb --timeout=120s
kubectl -n bossfang rollout status deployment/bossfang   --timeout=180s

# 8. Smoke
kubectl -n bossfang port-forward svc/bossfang 4545:4545 &
curl -fsS http://127.0.0.1:4545/api/health
curl -fsS https://bossfang.prometheusags.ai/api/health
```

## Secrets you must populate

See [`base/secrets.template.yaml`](base/secrets.template.yaml) for the
canonical list. At minimum:

- `SURREAL_USER`, `SURREAL_PASS` — credentials for the in-cluster SurrealDB
- `BOSSFANG_VAULT_KEY` — must base64-decode to **exactly 32 bytes**.
  Generate with `openssl rand -base64 32` (yields 44 chars). The legacy
  alias `LIBREFANG_VAULT_KEY` also works as a fallback.
- Provider keys: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GROQ_API_KEY`,
  `GEMINI_API_KEY`, etc. Only set the ones you actually need.

## What this does NOT cover

- **CI/CD image build & push pipeline.** Follow-up; for now build & push
  manually before each deploy.
- **Sealed Secrets / External Secrets Operator / Secret Manager wiring.**
  This ships a raw `Secret` template; secrets management migration is a
  separate concern.
- **HA / multi-replica.** The daemon's `daemon.lock` makes multi-replica
  unsafe. Single-replica only until a leader-election strategy lands.
- **Backup & restore** for the SurrealDB PVC.
- **Monitoring / observability**. No Prometheus scrape endpoint exists on
  the binary yet (it serves everything on port 4545).
