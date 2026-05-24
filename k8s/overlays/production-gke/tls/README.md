# Wildcard TLS wiring for `bossfang.prometheusags.ai`

The wildcard cert for `*.prometheusags.ai` was provisioned for candle-vllm
and lives on the cluster, NOT in this repo. This README explains how to
identify and wire it during deploy.

## Step 1 — Discover where the cert lives

Run these read-only kubectl commands first:

```bash
# What Gateways exist?
kubectl get gateway -A

# Inspect the Envoy Gateway's listeners — does it already have an HTTPS
# listener with the wildcard cert bound, or do we need to add one?
kubectl get gateway -n envoy-gateway-system envoy-gateway -o yaml

# Hunt for the actual cert Secret
kubectl get secret -A | grep -iE "wildcard|prometheusags|tls"

# Or, if it's cert-manager managed:
kubectl get certificate -A 2>/dev/null
kubectl get clusterissuer 2>/dev/null
```

## Step 2 — Pick the branch and wire it

Three possible states:

### Branch (a): The Gateway already has a wildcard HTTPS listener

Most likely case — `envoy-gateway` already terminates `*.prometheusags.ai`
TLS and routes all hostnames through. Then **no extra resources needed**;
the HTTPRoute in `k8s/base/httproute.yaml` will work as-is. The
`bossfang.prometheusags.ai` hostname matches the wildcard and the Gateway
serves it.

Verify by checking the listener block in the Gateway YAML for a `tls`
section with `certificateRefs`. If you see the wildcard cert Secret name
referenced, you're done — skip ahead to Step 3.

### Branch (b): Cert lives as a raw Secret in another namespace, Gateway needs explicit TLS binding

Use a `ReferenceGrant` to let the Gateway reference the Secret across
namespaces. Copy `reference-grant.yaml.template` to `reference-grant.yaml`,
fill in the actual Secret name and source namespace, and add it to
`../kustomization.yaml` under `resources:`.

You'll also need to add a TLS listener to the Gateway itself — but that's
a Gateway-level edit (in the `envoy-gateway-system` namespace), not in
this `bossfang/` kustomize tree. Coordinate with the cluster admin.

### Branch (c): Cert is cert-manager-managed and we need a per-host cert

Skip the wildcard reuse and create a new `Certificate` for
`bossfang.prometheusags.ai` via cert-manager. This is the heaviest path
but cleanest. Add a `Certificate` resource pointing at the
`ClusterIssuer` you discover in Step 1 and reference it in the Gateway
listener.

## Step 3 — Verify

After applying:

```bash
# Cert should be Ready
kubectl get certificate -A | grep bossfang || echo "(skipped - using existing wildcard)"

# Gateway should show the route attached and no TLS errors
kubectl describe gateway -n envoy-gateway-system envoy-gateway

# Public smoke
curl -fsS https://bossfang.prometheusags.ai/api/health
```

If the curl fails with a TLS handshake error, you're in branch (b) or (c)
without proper Gateway listener config. If it fails with a 503/404, the
HTTPRoute didn't attach — check Gateway events for the cause.
