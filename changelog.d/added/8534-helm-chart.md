Add a community Helm chart at `deploy/helm/librefang/` that mirrors the Kustomize manifests in `deploy/kubernetes/`, for operators who deploy through Helm.
It keeps the reference's guarantees: a single-replica StatefulSet (`values.schema.json` rejects any other `replicas`, and the templates re-check it so `--skip-schema-validation` cannot bypass it), credentials only from an `existingSecret` the chart never creates, `ClusterIP` only, and Pod Security `restricted`.
An `Ingress` and a Gateway API `HTTPRoute` are available but off by default, and refuse to render without hostnames (and, for an Ingress, TLS).
A `pre-install`/`pre-upgrade` hook validates a managed `config.toml` against the target image before the running pod is replaced.
CI runs `helm lint`, `helm template` and `scripts/check-k8s-manifests.py` over the chart's rendered output (#8534) (@sFritsch09)
