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
| No Ingress | none is shipped, and the schema rejects unknown top-level keys such as `ingress` |
| `ClusterIP` only | schema (`service.type` is `const: ClusterIP`) |
| Pod Security `restricted` | `podSecurityContext` / `securityContext` defaults; the schema pins `runAsNonRoot: true`, `allowPrivilegeEscalation: false`, `drop: [ALL]` |
| `ReadWriteOnce` only | hard-coded in `volumeClaimTemplates`; not a value |
| No `latest` tag | schema; the default tag is `Chart.appVersion`, a concrete release |
| No credential in a ConfigMap | render-time `fail` on literal `api_key`, `dashboard_pass`, `vault_key`, `state_secret`, `client_secret`, `password` in `managedConfig.*` (same list as `scripts/check-k8s-manifests.py`) |

`scripts/check-helm-chart.sh` proves each of these by rendering a violating input and requiring the render to fail.

## Install

The Secret is created out of band, exactly as in the Kustomize quick start, so no credential enters a values file, Helm release storage or git.
[`secrets.example.yaml`](secrets.example.yaml) lists every Secret and key name the default values reference, with commented-out extras (more providers, channel tokens, OAuth) to copy from. It is a reference, never rendered and never to be committed filled in. `scripts/check-helm-chart.sh` fails if the defaults in `values.yaml` and that file drift apart.

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

## Reaching the API

The Service is `ClusterIP` and the chart ships no Ingress: the API exposes shell exec, the credential vault and provider keys behind one bearer token.
Use `kubectl port-forward svc/librefang 4545:4545`, or put your own TLS-terminating Ingress/Gateway in front.
`networkPolicy.enabled=true` adds an ingress-deny policy with an explicit allow list (`networkPolicy.ingressFrom`); egress is intentionally open.

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

```bash
./scripts/check-helm-chart.sh   # lint --strict, render, check-k8s-manifests.py, parity, negative cases
```

`Chart.appVersion` must equal the tag in `deploy/kubernetes/base/kustomization.yaml`; bump them together.

Not in v1 (per the maintainers' scoping): Ingress, more than one replica, chart-created Secrets, and OCI publishing to `ghcr.io` with `appVersion` tracking releases.
