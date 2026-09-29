#!/usr/bin/env bash
# Gate for deploy/helm/librefang/. Run locally or from .github/workflows/kubernetes.yml.
#
#   1. helm lint --strict for every ci/*-values.yaml
#   2. helm template each of them and feed the output through
#      scripts/check-k8s-manifests.py — the same assertions the Kustomize
#      output must satisfy
#   3. parity with the Kustomize reference, which the chart mirrors:
#        - Chart.appVersion == the image tag pinned in kubernetes/base
#        - files/config.toml == the overlay's config.toml (comments aside)
#   4. the constraints the chart promises, each proven by a render that must FAIL
#
# Requires: helm >= 3.13, python3 with PyYAML. kubeconform is used when present.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="$root/deploy/helm/librefang"
checker="$root/scripts/check-k8s-manifests.py"
kdir="$root/deploy/kubernetes"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo "── $*"; }

step "lint + render + check-k8s-manifests.py"
for values in "$chart"/ci/*-values.yaml; do
  name="$(basename "$values" -values.yaml)"
  helm lint "$chart" --strict -f "$values" >/dev/null || { helm lint "$chart" --strict -f "$values"; fail "helm lint ($name)"; }
  helm template librefang "$chart" -f "$values" > "$tmp/$name.yaml"
  python3 "$checker" "$tmp/$name.yaml" || fail "check-k8s-manifests.py rejected the '$name' render"
  if command -v kubeconform >/dev/null; then
    kubeconform -strict -summary -kubernetes-version 1.31.0 "$tmp/$name.yaml" || fail "kubeconform ($name)"
  fi
done

step "parity with deploy/kubernetes"
kustomize_tag="$(sed -n 's/^[[:space:]]*newTag:[[:space:]]*//p' "$kdir/base/kustomization.yaml" | tr -d '"' | head -n1)"
app_version="$(sed -n 's/^appVersion:[[:space:]]*//p' "$chart/Chart.yaml" | tr -d '"' | head -n1)"
[ -n "$kustomize_tag" ] && [ "$kustomize_tag" = "$app_version" ] \
  || fail "Chart.appVersion '$app_version' != kubernetes/base image tag '$kustomize_tag'"
diff <(grep -v '^[[:space:]]*#' "$kdir/overlays/managed-config/config.toml") \
     <(grep -v '^[[:space:]]*#' "$chart/files/config.toml") >/dev/null \
  || fail "files/config.toml drifted from overlays/managed-config/config.toml"

step "secrets.example.yaml documents every Secret key the chart reads by default"
python3 - "$chart" <<'PY' || fail "secrets.example.yaml is out of sync with values.yaml"
import sys, yaml
chart = sys.argv[1]
values = yaml.safe_load(open(f"{chart}/values.yaml"))
docs = {d["metadata"]["name"]: set(d.get("stringData", {})) for d in yaml.safe_load_all(open(f"{chart}/secrets.example.yaml")) if d}
missing = []
auth = docs.get(values["auth"]["existingSecret"] or "librefang-auth", set())
for name, key in values["auth"]["keys"].items():
    # stateSecret is optional (only needed with [external_auth]); it is documented commented-out.
    if name != "stateSecret" and key not in auth:
        missing.append(f"librefang-auth: {key}")
prov = docs.get(values["providers"]["existingSecret"], set())
for e in values["providers"]["env"]:
    if "secretName" not in e and e["key"] not in prov:
        missing.append(f"{values['providers']['existingSecret']}: {e['key']}")
for m in missing:
    print("missing from secrets.example.yaml ->", m, file=sys.stderr)
sys.exit(1 if missing else 0)
PY

step "promised constraints are enforced (each render below must fail)"
must_fail() {
  local desc="$1" want="$2"; shift 2
  local out
  if out="$(helm template librefang "$chart" -f "$chart/ci/default-values.yaml" "$@" 2>&1)"; then
    fail "accepted: $desc"
  fi
  grep -qiE "$want" <<<"$out" || fail "rejected '$desc' for the wrong reason: $out"
}
managed=(--set managedConfig.enabled=true)
must_fail "replicas=2"                        "replicas"           --set replicas=2
must_fail "replicas=2, schema bypassed"       "exactly 1"          --set replicas=2 --skip-schema-validation
must_fail "no auth.existingSecret"            "existingSecret"     --set auth.existingSecret=
must_fail "no auth.existingSecret, no schema" "existingSecret"     --set auth.existingSecret= --skip-schema-validation
must_fail "service.type=LoadBalancer"         "service"            --set service.type=LoadBalancer
must_fail "image.tag=latest"                  "image"              --set image.tag=latest
must_fail "runAsNonRoot=false"                "runAsNonRoot"       --set podSecurityContext.runAsNonRoot=false
must_fail "an ingress value (none in v1)"     "ingress"            --set ingress.enabled=true
must_fail "managed config on a pre-8.30 tag"  "2026.8.30"          --set managedConfig.enabled=true --set image.tag=2026.7.31
must_fail "v-prefixed tag (never pushed)"     "image"              --set image.tag=v2026.9.19
must_fail "literal api_key in config.toml"    "api_key"            "${managed[@]}" \
  --set-json 'managedConfig.config="[default_model]\napi_key = \"sk-live\"\n"'
must_fail "literal password in an agent"     "password"           "${managed[@]}" \
  --set-json 'managedConfig.agents={"a.toml":"name = \"a\"\npassword = \"hunter2\"\n"}'
must_fail "unparseable config.toml"           "not valid TOML"     "${managed[@]}" \
  --set-json 'managedConfig.config="api_listen = \"0.0.0.0:4545\n[default_model\n"'
must_fail "unparseable agent TOML"            "not valid TOML"     "${managed[@]}" \
  --set-json 'managedConfig.agents={"a.toml":"name = \"a"}'
must_fail "extraEnv shadowing a chart var"    "collides"           --set-json 'extraEnv=[{"name":"LIBREFANG_API_KEY","value":"x"}]'

step "config pre-flight hook is wired correctly"
helm template librefang "$chart" -f "$chart/ci/managed-values.yaml" > "$tmp/hook.yaml"
python3 - "$tmp/hook.yaml" <<'PY' || fail "config-validate hook is missing or mis-wired"
import json, sys, yaml
docs = [d for d in yaml.safe_load_all(open(sys.argv[1])) if d]
job = next((d for d in docs if d["kind"] == "Job"), None)
assert job, "no Job rendered when managedConfig.enabled"
hooks = job["metadata"]["annotations"]["helm.sh/hook"].split(",")
assert set(hooks) == {"pre-install", "pre-upgrade"}, hooks
assert job["spec"]["backoffLimit"] == 0, "a failed validation must not be retried"
pod = job["spec"]["template"]["spec"]
sc = pod["containers"][0]["securityContext"]
assert sc["allowPrivilegeEscalation"] is False and sc["capabilities"]["drop"] == ["ALL"]
assert pod["securityContext"]["runAsNonRoot"] is True
env = {e["name"]: e.get("value") for e in pod["containers"][0]["env"]}
assert env["LIBREFANG_CONFIG_PATH"] == "/candidate/config.toml"
assert env["LIBREFANG_LISTEN"].startswith("127.0.0.1:"), "the hook must not bind a routable address"
assert "secretKeyRef" not in json.dumps(pod), "the hook must not mount credentials"
# The hook validates the candidate ConfigMap; it must carry the same bytes as the real one.
cms = {d["metadata"]["name"]: d for d in docs if d["kind"] == "ConfigMap"}
real = next(v for k, v in cms.items() if k.endswith("-config"))
cand = next(v for k, v in cms.items() if k.endswith("-config-validate"))
assert real["data"]["config.toml"] == cand["data"]["config.toml"], "hook validates different bytes than are deployed"
PY
[ "$(helm template librefang "$chart" -f "$chart/ci/default-values.yaml" | grep -c 'config-validate')" = 0 ] \
  || fail "the hook rendered although managedConfig is disabled"
[ "$(helm template librefang "$chart" -f "$chart/ci/managed-values.yaml" --set managedConfig.validate.enabled=false | grep -c 'config-validate')" = 0 ] \
  || fail "the hook rendered although managedConfig.validate.enabled=false"

step "the chart renders no Secret and no Ingress"
if grep -Eq '^kind: (Secret|Ingress)$' "$tmp"/*.yaml; then fail "a Secret or Ingress was rendered"; fi

echo "OK: helm chart gate passed"
