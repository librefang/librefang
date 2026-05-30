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

# 8. Smoke (the public route is behind Envoy Basic Auth — see "Edge auth")
kubectl -n bossfang port-forward svc/bossfang 4545:4545 &
curl -fsS http://127.0.0.1:4545/api/health                       # in-cluster: no auth
curl -fsS -u "$BASIC_AUTH_USER:$BASIC_AUTH_PASS" \
     https://bossfang.prometheusags.ai/api/health                # public: 401 without creds
```

## Edge auth (Envoy Gateway Basic Auth)

`bossfang.prometheusags.ai` is intentionally public-facing (the WebChat
dashboard is browser-accessible), so the daemon runs without an in-tree
`api_key` (`LIBREFANG_ALLOW_NO_AUTH=1`). Authentication is enforced ONE
LAYER OUT, at the Envoy Gateway edge, via an HTTP Basic Auth
`SecurityPolicy` ([`base/security-policy.yaml`](base/security-policy.yaml)).
The daemon's `config.toml` sets `external_auth_proxy = true` to record that
posture.

**Prerequisite — SecurityPolicy CRD.** Basic auth needs the Envoy Gateway
`securitypolicies.gateway.envoyproxy.io` CRD. A complete Envoy Gateway
install ships it, but if the cluster has a partial CRD set, install the
matching-version CRD and restart the controller so it watches the resource:

```bash
EG_VER=$(kubectl get deploy envoy-gateway -n envoy-gateway-system \
  -o jsonpath='{.spec.template.spec.containers[0].image}' | sed 's/.*://')
curl -fsSL "https://github.com/envoyproxy/gateway/releases/download/${EG_VER}/install.yaml" \
  | python3 -c "import sys,yaml; [print('---'); print(yaml.safe_dump(d)) for d in yaml.safe_load_all(sys.stdin) if d and d.get('kind')=='CustomResourceDefinition' and d['metadata']['name']=='securitypolicies.gateway.envoyproxy.io']" \
  | kubectl apply -f -
kubectl rollout restart deployment/envoy-gateway -n envoy-gateway-system
```

**Create the credential Secret** (out-of-band, never committed — see
`base/secrets.template.yaml`):

```bash
HASH=$(printf '%s' "$BASIC_AUTH_PASS" | openssl sha1 -binary | openssl base64)
printf '%s:{SHA}%s\n' "$BASIC_AUTH_USER" "$HASH" > /tmp/bossfang.htpasswd
kubectl create secret generic bossfang-basic-auth -n bossfang \
  --from-file=.htpasswd=/tmp/bossfang.htpasswd \
  --dry-run=client -o yaml | kubectl apply -f -
rm -f /tmp/bossfang.htpasswd
```

Verify: `curl -i https://bossfang.prometheusags.ai/api/health` → **401**;
with `-u user:pass` → **200**.

## Secrets you must populate

See [`base/secrets.template.yaml`](base/secrets.template.yaml) for the
canonical list. At minimum:

- `SURREAL_USER`, `SURREAL_PASS` — credentials for the in-cluster SurrealDB
- `BOSSFANG_VAULT_KEY` — must base64-decode to **exactly 32 bytes**.
  Generate with `openssl rand -base64 32` (yields 44 chars). The legacy
  alias `LIBREFANG_VAULT_KEY` also works as a fallback.
- Provider keys: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GROQ_API_KEY`,
  `GEMINI_API_KEY`, etc. Only set the ones you actually need.
- `bossfang-basic-auth` Secret (`.htpasswd` key) — the Envoy Gateway edge
  Basic Auth credentials. Created separately from `bossfang-secrets`; see
  the "Edge auth" section above for the generation recipe.

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
