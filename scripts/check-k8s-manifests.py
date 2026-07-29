#!/usr/bin/env python3
"""Validate the rendered Kubernetes manifests under `deploy/kubernetes/`.

The manifests carry safety properties that are easy to break silently, because
nothing fails loudly when they regress: a `replicas` bump boots a second daemon
that duplicates every cron fire, a dropped `fsGroup` makes a fresh PVC
unwritable only on the operator's first install, and a `livenessProbe` pointed
at `/api/ready` turns a recoverable storage incident into a restart loop. None
of those show up in `kubectl apply` output.

This script asserts each property against the *rendered* output of
`kubectl kustomize` (or `kustomize build`), so overlays and transformers are
included rather than assumed away.

Usage:
    kubectl kustomize deploy/kubernetes/base | scripts/check-k8s-manifests.py
    scripts/check-k8s-manifests.py rendered.yaml

Exits 0 when every check passes, 1 with one line per failure otherwise.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:  # pragma: no cover - dependency reported, not worked around
    sys.exit(
        "error: PyYAML is required (pip install pyyaml). "
        "CI installs it in the same step that runs this script."
    )

# The daemon's data dir. `LIBREFANG_HOME` must agree with the volume mount, or
# the pod runs with an empty ephemeral state dir and loses everything on
# restart while looking perfectly healthy.
DATA_DIR = "/data"
CONTAINER_PORT = 4545

# Secret keys the StatefulSet must consume from a `secretKeyRef` rather than a
# literal value. A non-loopback bind refuses to boot without authentication
# (#3572), so these are load-bearing, not hardening.
REQUIRED_SECRET_ENV = {
    "LIBREFANG_API_KEY",
    "LIBREFANG_VAULT_KEY",
    "LIBREFANG_DASHBOARD_USER",
    "LIBREFANG_DASHBOARD_PASS",
}

# Probe → path. The split is the whole point of #6633: liveness must not fail
# for a dependency outage, readiness must.
EXPECTED_PROBE_PATHS = {
    "startupProbe": "/api/ready",
    "livenessProbe": "/api/health",
    "readinessProbe": "/api/ready",
}


class Failures:
    """Collects every failure so one run reports all of them."""

    def __init__(self) -> None:
        self.items: list[str] = []

    def check(self, condition: bool, message: str) -> bool:
        if not condition:
            self.items.append(message)
        return condition

    def fail(self, message: str) -> None:
        self.items.append(message)


def load_documents(source: str) -> list[dict[str, Any]]:
    return [doc for doc in yaml.safe_load_all(source) if isinstance(doc, dict)]


def find_one(docs: list[dict[str, Any]], kind: str) -> dict[str, Any] | None:
    matches = [d for d in docs if d.get("kind") == kind]
    return matches[0] if len(matches) == 1 else None


def check_statefulset(sts: dict[str, Any], failures: Failures) -> None:
    spec = sts.get("spec", {})

    failures.check(
        spec.get("replicas") == 1,
        f"StatefulSet.spec.replicas must be exactly 1, got {spec.get('replicas')!r}. "
        "Cron, trigger dispatch, session ownership, budget enforcement, and the "
        "audit hash chain are all process-local — see "
        "docs/architecture/multi-replica-rfc.md.",
    )

    selector_labels = spec.get("selector", {}).get("matchLabels", {})
    template = spec.get("template", {})
    template_labels = template.get("metadata", {}).get("labels", {})
    failures.check(
        bool(selector_labels)
        and all(template_labels.get(k) == v for k, v in selector_labels.items()),
        f"StatefulSet selector {selector_labels!r} does not match pod template "
        f"labels {template_labels!r} — the Service would select nothing.",
    )

    pod_spec = template.get("spec", {})

    grace = pod_spec.get("terminationGracePeriodSeconds")
    failures.check(
        isinstance(grace, int) and grace >= 30,
        f"terminationGracePeriodSeconds must be >= 30 to let the SQLite WAL "
        f"checkpoint and any in-flight agent turn finish, got {grace!r}.",
    )

    check_pod_security(pod_spec, failures)
    check_volume_claims(spec, pod_spec, failures)

    containers = pod_spec.get("containers", [])
    if not failures.check(
        len(containers) == 1,
        f"expected exactly 1 container in the pod template, got {len(containers)}.",
    ):
        return
    check_container(containers[0], failures)


def check_pod_security(pod_spec: dict[str, Any], failures: Failures) -> None:
    """Assert the pod-level half of Pod Security Standard `restricted` (#6632)."""
    ctx = pod_spec.get("securityContext", {})

    failures.check(
        ctx.get("runAsNonRoot") is True,
        "pod securityContext.runAsNonRoot must be true — `restricted` rejects a "
        "root-starting main container.",
    )
    for field in ("runAsUser", "runAsGroup", "fsGroup"):
        failures.check(
            ctx.get(field) == 1001,
            f"pod securityContext.{field} must be 1001 (the uid/gid the image "
            f"creates), got {ctx.get(field)!r}.",
        )
    failures.check(
        ctx.get("seccompProfile", {}).get("type") == "RuntimeDefault",
        "pod securityContext.seccompProfile.type must be RuntimeDefault.",
    )
    # Not a `restricted` requirement, but dropping it makes every pod start
    # re-chgrp the whole data dir.
    failures.check(
        ctx.get("fsGroupChangePolicy") == "OnRootMismatch",
        "pod securityContext.fsGroupChangePolicy should be OnRootMismatch so a "
        "growing /data is not relabelled on every start.",
    )


def check_volume_claims(
    spec: dict[str, Any], pod_spec: dict[str, Any], failures: Failures
) -> None:
    claims = spec.get("volumeClaimTemplates", [])
    if not failures.check(
        len(claims) == 1,
        f"expected exactly 1 volumeClaimTemplate for {DATA_DIR}, got {len(claims)}.",
    ):
        return

    claim = claims[0]
    claim_name = claim.get("metadata", {}).get("name")
    access_modes = claim.get("spec", {}).get("accessModes", [])
    failures.check(
        access_modes == ["ReadWriteOnce"],
        f"volumeClaimTemplate accessModes must be exactly ['ReadWriteOnce'], got "
        f"{access_modes!r}. SQLite WAL and the daemon.lock flock depend on POSIX "
        "locking that shared filesystems frequently do not provide.",
    )
    failures.check(
        bool(claim.get("spec", {}).get("resources", {}).get("requests", {}).get("storage")),
        "volumeClaimTemplate must request a storage size.",
    )

    # A claim nothing mounts is the failure mode where the pod looks healthy
    # and silently keeps its state on an ephemeral overlay.
    mounts = [
        m
        for c in pod_spec.get("containers", [])
        for m in c.get("volumeMounts", [])
        if m.get("name") == claim_name
    ]
    failures.check(
        any(m.get("mountPath") == DATA_DIR for m in mounts),
        f"volumeClaimTemplate {claim_name!r} must be mounted at {DATA_DIR}; "
        f"found mounts {[m.get('mountPath') for m in mounts]!r}.",
    )


def check_container(container: dict[str, Any], failures: Failures) -> None:
    ctx = container.get("securityContext", {})
    failures.check(
        ctx.get("allowPrivilegeEscalation") is False,
        "container securityContext.allowPrivilegeEscalation must be false.",
    )
    failures.check(
        ctx.get("capabilities", {}).get("drop") == ["ALL"],
        "container securityContext.capabilities.drop must be ['ALL'].",
    )

    ports = container.get("ports", [])
    named_http = [p for p in ports if p.get("name") == "http"]
    failures.check(
        any(p.get("containerPort") == CONTAINER_PORT for p in named_http),
        f"container must expose containerPort {CONTAINER_PORT} named 'http' "
        f"(the probes and Service target it by name), got {ports!r}.",
    )

    check_env(container, failures)
    check_probes(container, failures)

    resources = container.get("resources", {})
    failures.check(
        bool(resources.get("requests", {}).get("cpu"))
        and bool(resources.get("requests", {}).get("memory")),
        "container must declare cpu and memory requests so the scheduler has "
        "something to place against.",
    )
    failures.check(
        bool(resources.get("limits", {}).get("memory")),
        "container must declare a memory limit — an unbounded agent workload "
        "otherwise takes the node down instead of just itself.",
    )


def check_env(container: dict[str, Any], failures: Failures) -> None:
    env = {e.get("name"): e for e in container.get("env", []) if isinstance(e, dict)}

    listen = env.get("LIBREFANG_LISTEN", {}).get("value")
    failures.check(
        listen == f"0.0.0.0:{CONTAINER_PORT}",
        f"LIBREFANG_LISTEN must be '0.0.0.0:{CONTAINER_PORT}' — the container's "
        f"loopback is unreachable from the kubelet, got {listen!r}.",
    )
    home = env.get("LIBREFANG_HOME", {}).get("value")
    failures.check(
        home == DATA_DIR,
        f"LIBREFANG_HOME must be {DATA_DIR} to match the volume mount, got {home!r}.",
    )

    for name in sorted(REQUIRED_SECRET_ENV):
        entry = env.get(name)
        if entry is None:
            failures.fail(
                f"{name} is missing. Without configured authentication the daemon "
                "refuses to start on a non-loopback bind (#3572)."
            )
            continue
        ref = entry.get("valueFrom", {}).get("secretKeyRef")
        if ref is None:
            failures.fail(
                f"{name} must come from a secretKeyRef, not a literal "
                f"'value' in the manifest (got {entry!r})."
            )
            continue
        failures.check(
            ref.get("optional") is not True,
            f"{name} must not be marked optional — a missing value here means "
            "the daemon boots unauthenticated or not at all, and failing at the "
            "kubelet is the safer of the two.",
        )

    # Anything else pulled from a Secret must be explicitly optional, so a
    # cluster that runs only local models is not blocked on provider keys.
    for name, entry in sorted(env.items()):
        if name in REQUIRED_SECRET_ENV:
            continue
        ref = entry.get("valueFrom", {}).get("secretKeyRef")
        if ref is not None:
            failures.check(
                ref.get("optional") is True,
                f"{name} reads a Secret key that is not required for boot, so it "
                "must be marked optional: true.",
            )


def check_probes(container: dict[str, Any], failures: Failures) -> None:
    for probe_name, expected_path in EXPECTED_PROBE_PATHS.items():
        probe = container.get(probe_name)
        if probe is None:
            failures.fail(f"{probe_name} is missing.")
            continue
        http_get = probe.get("httpGet", {})
        actual = http_get.get("path")
        if probe_name == "livenessProbe" and actual == "/api/ready":
            failures.fail(
                "livenessProbe must NOT target /api/ready. Readiness returns 503 "
                "for a recoverable dependency outage; as a liveness signal that "
                "restart-loops the pod through the same incident. Use "
                "/api/health."
            )
            continue
        failures.check(
            actual == expected_path,
            f"{probe_name}.httpGet.path must be {expected_path!r}, got {actual!r}.",
        )
        failures.check(
            http_get.get("port") == "http",
            f"{probe_name} should target the named port 'http' so a port change "
            f"cannot desynchronise, got {http_get.get('port')!r}.",
        )

    startup = container.get("startupProbe", {})
    period = startup.get("periodSeconds") or 10
    threshold = startup.get("failureThreshold") or 3
    failures.check(
        period * threshold >= 60,
        f"startupProbe budget is only {period * threshold}s "
        f"(periodSeconds {period} x failureThreshold {threshold}). First boot runs "
        "`librefang init`, which writes config.toml and seeds the registry — give "
        "it at least 60s or slow storage will crash-loop a healthy install.",
    )


def check_services(docs: list[dict[str, Any]], failures: Failures) -> None:
    services = [d for d in docs if d.get("kind") == "Service"]
    if not failures.check(
        len(services) >= 1, "expected at least one Service exposing the daemon."
    ):
        return

    sts = find_one(docs, "StatefulSet")
    governing = (sts or {}).get("spec", {}).get("serviceName")
    by_name = {s.get("metadata", {}).get("name"): s for s in services}

    failures.check(
        governing in by_name,
        f"StatefulSet.spec.serviceName {governing!r} does not name a Service in "
        f"this kustomization (have {sorted(by_name)!r}).",
    )
    if governing in by_name:
        failures.check(
            by_name[governing].get("spec", {}).get("clusterIP") == "None",
            f"the governing Service {governing!r} must be headless "
            "(clusterIP: None) for the StatefulSet's per-pod DNS to resolve.",
        )

    client_services = [
        s for s in services if s.get("spec", {}).get("clusterIP") != "None"
    ]
    failures.check(
        len(client_services) >= 1,
        "expected a non-headless ClusterIP Service for clients to reach.",
    )
    for service in client_services:
        spec = service.get("spec", {})
        name = service.get("metadata", {}).get("name")
        failures.check(
            spec.get("type") == "ClusterIP",
            f"Service {name!r} must be type ClusterIP — the API exposes shell "
            f"exec, the vault, and provider keys, so cluster-external reach "
            f"should be a deliberate Ingress, got {spec.get('type')!r}.",
        )
        ports = spec.get("ports", [])
        failures.check(
            any(p.get("port") == CONTAINER_PORT for p in ports),
            f"Service {name!r} must expose port {CONTAINER_PORT}, got {ports!r}.",
        )


def check_no_inline_secrets(docs: list[dict[str, Any]], failures: Failures) -> None:
    """A Secret rendered by the kustomization would land its value in git."""
    for doc in docs:
        if doc.get("kind") == "Secret":
            name = doc.get("metadata", {}).get("name")
            failures.fail(
                f"the kustomization renders a Secret ({name!r}). Credentials must "
                "be created out of band with `kubectl create secret` so they never "
                "enter version control — see deploy/kubernetes/README.md."
            )


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        return int(bool(sys.stderr.write(f"usage: {argv[0]} [rendered.yaml]\n")) or 2)

    source = Path(argv[1]).read_text() if len(argv) == 2 else sys.stdin.read()
    if not source.strip():
        sys.stderr.write("error: no manifest input (stdin was empty)\n")
        return 2

    docs = load_documents(source)
    failures = Failures()

    sts = find_one(docs, "StatefulSet")
    if sts is None:
        kinds = sorted({d.get("kind") for d in docs})
        failures.fail(f"expected exactly one StatefulSet; rendered kinds: {kinds!r}")
    else:
        check_statefulset(sts, failures)

    check_services(docs, failures)
    check_no_inline_secrets(docs, failures)

    if failures.items:
        sys.stderr.write(
            f"{len(failures.items)} manifest check(s) failed:\n\n"
            + "".join(f"  - {item}\n" for item in failures.items)
        )
        return 1

    print(f"OK: {len(docs)} rendered manifest(s) satisfy every check.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
