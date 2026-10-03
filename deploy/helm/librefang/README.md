# LibreFang Helm chart (community)

A Helm packaging of the Kustomize manifests in [`deploy/kubernetes/`](../../kubernetes/README.md).
**The Kustomize tree is the reference; this chart mirrors it.**
Every safety property, and the reason for it, is documented there — this page covers only what is specific to the chart.
When the two disagree, the Kustomize manifests are right and the chart has a bug.

## What the chart enforces

| Rule | Where it is enforced |
| --- | --- |
| Single replica: `replicas` must be `1` | `values.schema.json` (`const: 1`) **and** a `fail` in the templates, so `--skip-schema-validation` does not bypass it |
| Secrets only via `existingSecret` — the chart renders no `Secret` | `auth.existingSecret` is required; nothing in `templates/` creates a Secret |
| Nothing is exposed unless you ask | `ingress.enabled` and `httpRoute.enabled` default to `false`; when on, hostnames are required and an Ingress must have TLS (see [Exposing the API](#exposing-the-api)) |
| `ClusterIP` only | schema (`service.type` is `const: ClusterIP`); external reach is `ingress` / `httpRoute`, never a NodePort or LoadBalancer Service |
| Pod Security `restricted` | `podSecurityContext` / `securityContext` defaults; the schema pins `runAsNonRoot: true`, `allowPrivilegeEscalation: false`, `drop: [ALL]` |
| `ReadWriteOnce` only | hard-coded in `volumeClaimTemplates`; not a value |
| No `latest` tag | schema; the default tag is `Chart.appVersion`, a concrete release |
| No credential in a ConfigMap | render-time `fail` on literal `api_key`, `dashboard_pass`, `vault_key`, `state_secret`, `client_secret`, `password` in `managedConfig.*` (same list as `scripts/check-k8s-manifests.py`) |

Each rule fails at `helm template` / `helm install` time with a message naming the value to fix.

## Install

The Secret is created out of band, exactly as in the Kustomize quick start, so no credential enters a values file, Helm release storage or git.
[`secrets.example.yaml`](secrets.example.yaml) lists every Secret and key name the default values reference, with commented-out extras (more providers, channel tokens, OAuth) to copy from. It is a reference, never rendered and never to be committed filled in.

The one-liner form:

```bash
kubectl create namespace librefang
kubectl label namespace librefang \
  pod-security.kubernetes.io/enforce=restricted \
  pod-security.kubernetes.io/enforce-version=latest

kubectl -n librefang create secret generic librefang-auth \
  --from-literal=api-key="$(openssl rand -hex 32)" \
  --from-literal=vault-key="$(openssl rand -base64 32)" \
  --from-literal=dashboard-user=admin \
  --from-literal=dashboard-pass="$(openssl rand -hex 24)"

# Optional — skip for a local-model-only cluster. Key names: see values.yaml.
kubectl -n librefang create secret generic librefang-providers \
  --from-literal=anthropic-api-key="$ANTHROPIC_API_KEY" \
  --from-literal=openrouter-api-key="$OPENROUTER_API_KEY"

helm install librefang deploy/helm/librefang -n librefang \
  --set auth.existingSecret=librefang-auth
```

`vault-key` (and `state-secret`) must base64-decode to exactly 32 bytes; `openssl rand -base64 32` does.
If you enable `[external_auth]`, add `state-secret` to the same Secret — the daemon refuses to boot without it.

## Managed configuration

`managedConfig.enabled=true` is the chart's equivalent of `overlays/managed-config`:
`config.toml` goes into a ConfigMap, is mounted read-only at `/etc/librefang`, and is locked server-side (`LIBREFANG_CONFIG_MODE=managed`).
It needs an image **≥ 2026.8.30**; the chart refuses an older `image.tag`, because that image would run `librefang init` against the read-only mount and crash-loop.

```bash
helm upgrade --install librefang deploy/helm/librefang -n librefang \
  --set auth.existingSecret=librefang-auth \
  --set managedConfig.enabled=true \
  -f my-config.yaml   # managedConfig.config / managedConfig.agents
```

- **Switching an existing install from mutable to managed** leaves the old `/data/config.toml` on the PVC. The daemon ignores it (it reads `/etc/librefang/config.toml`; `GET /api/config/status` reports `source`), but it is a stale second copy. Delete it once managed mode is confirmed. A fresh install never creates it.
- **Changing `config.toml` on a running release** is a values edit plus `helm upgrade`: the ConfigMap is re-rendered, the `checksum/config` annotation changes, and the StatefulSet replaces the pod, which reads the new file at boot. Verified on a live cluster: after an upgrade `GET /api/config/status` reports the same `checksum` as the pod annotation, and `GET /api/config` shows the new values. Nothing reloads in place (`[reload] mode = "off"`).
- **A bad config is caught before the running pod is touched.** Two layers:
  1. At render time the chart parses the TOML, so a syntax error fails `helm upgrade` immediately.
  2. A `pre-install`/`pre-upgrade` hook Job (`managedConfig.validate.enabled`, default `true`) boots the real daemon, from the image being deployed, against the new `config.toml` and requires `/api/health`. The daemon rejects a wrongly typed value, a bad enum or an unresolvable `include` in about a second (`Config file cannot be deserialized … expected usize`), the Job fails, and `helm upgrade` aborts with the daemon's own error in `kubectl logs job/<release>-librefang-config-validate`. This matters because a StatefulSet stops the old pod *before* starting the new one: without the hook, a config the daemon refuses leaves you with a crash-looping instance and no working one. Verified on a live cluster: the old pod kept serving, the ConfigMap and StatefulSet kept the old config, and the next good upgrade went through.

  Limits: the hook has no credentials (least privilege), so it cannot tell that a provider key or channel token is wrong; and unknown keys are only rejected if your file sets `strict_config = true`, the daemon's own rule. Set `managedConfig.validate.enabled=false` to skip it, for example where Jobs are not allowed. Note that Helm records a failed hook as a `failed` release revision; the next successful `helm upgrade` supersedes it.
- `managedConfig.config` replaces the whole file; empty uses `files/config.toml`, which is the overlay's `config.toml` byte for byte (comments aside).
- `managedConfig.agents` is a map of `<file>.toml` to its content, rendered into a second ConfigMap and mounted at `/etc/librefang/provisioning/agents`. Empty leaves provisioning off.
- The `checksum/config` and `checksum/provisioning` pod annotations are **computed**, so a values change rolls the StatefulSet without anyone hashing by hand. They are the same digests `GET /api/config/status` and `GET /api/provisioning/status` report. Confirm a rollout landed by comparing them, not by counting restarts.
- Nothing secret belongs in these values either: Helm stores values in release Secrets and the rendered ConfigMap is unencrypted in etcd. Name the variable (`api_key_env = "ANTHROPIC_API_KEY"`) and supply it through `providers.env` / `extraEnv`.

## Providers and extra environment

`providers.env` maps environment variable names to keys of `providers.existingSecret`; every entry is `optional: true`.
Add a variable, or source one from a different Secret, with `secretName`.
`extraEnv` is passed through verbatim, but names the chart manages (`LIBREFANG_LISTEN`, `LIBREFANG_HOME`, `LIBREFANG_*_KEY`, `LIBREFANG_CONFIG_*`, …) are rejected.

## Exposing the API

By default nothing leaves the cluster: the Service is `ClusterIP`, and `kubectl port-forward svc/librefang 4545:4545` is the way in.
The API can run shell commands, use the credential vault and spend your provider keys, so exposure is opt-in and refuses to render half-configured.

**What protects it.** The chart always requires a credential (`auth.existingSecret`), so an exposed daemon is never anonymous. Checked against 2026.9.19 with a credential set: `/api/agents`, `/api/config`, `/api/sessions`, `/api/providers`, `/api/metrics` and `/api/health/detail` answer `401` without a token.
**What stays open**, by design: `/api/health`, `/api/ready`, `/api/version`, `/api/config/schema`, the dashboard's static assets, the login endpoints, and the A2A agent card (`/.well-known/agent.json`, `/a2a/agents`), which lists agent names and descriptions. Keep that in mind before publishing an agent's description.

### Gateway API (`httpRoute`)

```yaml
httpRoute:
  enabled: true
  parentRefs:
    - name: shared-gateway
      namespace: default
      sectionName: https-librefang     # an HTTPS listener: TLS ends at the Gateway
  hostnames: [librefang.example.com]
  rules:
    - matches: [{path: {type: PathPrefix, value: /}}]
      timeouts: {request: 3600s, backendRequest: 3600s}   # streaming chat / SSE
  httpRedirect:                        # optional 301 http -> https
    enabled: true
    parentRefs:
      - {name: shared-gateway, namespace: default, sectionName: http-librefang}
```

The chart adds the backend (this release's Service) to every rule; do not set `backendRefs`. It needs the Gateway API CRDs (`gateway.networking.k8s.io/v1`) and a Gateway whose listener admits routes from this namespace (`allowedRoutes`). Certificates are the Gateway's business (for example cert-manager on the Gateway); the chart does not create any.
Give the HTTPS route and the redirect route different listeners (`sectionName`), or they compete for one.

### Ingress (`ingress`)

```yaml
ingress:
  enabled: true
  className: nginx
  annotations: {cert-manager.io/cluster-issuer: letsencrypt-prod}
  hosts:
    - host: librefang.example.com
  tls:
    - secretName: librefang-tls
      hosts: [librefang.example.com]
```

`hosts` and `tls` are required, and every host must appear under a `tls` entry, otherwise it would be served in clear text and the bearer token and dashboard password would cross the network unencrypted. `allowWithoutTLS: true` overrides this only for setups where TLS is terminated in front of the Ingress by something the chart cannot see.

### Behind a proxy: set `trusted_proxies`

Behind a Gateway or Ingress the daemon sees the proxy's address, not the client's. Its per-IP login rate limit (a brute-force guard on the dashboard login and token endpoints) then counts every visitor as one client, so a single failed-login burst can lock everyone out, and no client is limited individually.
Tell the daemon which peers to believe with `managedConfig.config` (or your `config.toml`):

```toml
trusted_proxies = ["10.108.0.0/14"]   # your pod CIDR, or the Gateway's pod IPs
trust_forwarded_for = true
```

Both are required; an empty `trusted_proxies` disables header trust whatever `trust_forwarded_for` says, which is the safe default. Do not trust a wider range than your proxy: any peer in it can forge the client address.
`cors_origin` and `trusted_hosts` (top-level keys) are also worth setting to your public hostname if you use the dashboard or MCP OAuth flows from it.

### Network policy

`networkPolicy.enabled=true` denies ingress except from the peers in `networkPolicy.ingressFrom`. When you expose the API, add your Gateway or ingress controller there; without it the policy blocks the very traffic you just enabled. Egress is intentionally open.

## Storage and lifecycle

- `persistence.retentionPolicy` defaults to `Retain/Retain`, so `helm uninstall` leaves the PVC (your agents' state) in place. Delete it yourself to discard.
- Volume ownership relies on `fsGroup: 1001`; drivers with `fsGroupPolicy: None` (most NFS/CIFS) ignore it — see [Volume ownership](../../kubernetes/README.md#volume-ownership).
- Resources default to the Kustomize values: they are starting points, not tuned recommendations.

## Differences from the Kustomize output

Deliberate, all additive; the StatefulSet's `securityContext`, probes, env, resources, volumes and update strategy are identical.

| Chart adds | Why |
| --- | --- |
| `app.kubernetes.io/instance` selector label, `helm.sh/chart`, `managed-by`, `version` labels | Helm convention; lets two releases share a namespace |
| ServiceAccount with `automountServiceAccountToken: false` | The daemon never calls the Kubernetes API, so it gets no token |
| `persistentVolumeClaimRetentionPolicy: Retain` | Makes the "PVC outlives the StatefulSet" behaviour explicit |
| Release-prefixed names (`<release>-headless`, …) | `helm install librefang …` reproduces the Kustomize names |

The `app.kubernetes.io/instance` selector label means a cluster that already runs the **Kustomize** StatefulSet cannot be adopted by `helm upgrade`: a StatefulSet selector is immutable. Install into a fresh namespace or release, and migrate state with the volume-copy procedure in the Kustomize README.

## Development

CI (`.github/workflows/helm.yml`) runs `helm lint`, `helm template` and `scripts/check-k8s-manifests.py` over the output for every values file in [`ci/`](ci/). A new scenario is a new file there, not a workflow change. That is all it checks; it needs no cluster.

`Chart.appVersion` is the default image tag. ghcr.io publishes tags **without** a leading `v` (`2026.9.19`); the schema rejects `v`-prefixed and `latest` tags. Bump `appVersion` with each release you have tested the chart against.

Not in this chart yet: more than one replica (unsupported by the daemon), chart-created Secrets, and OCI publishing to `ghcr.io` with `appVersion` tracking releases.
