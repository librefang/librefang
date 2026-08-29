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

Exits 0 when every check passes, 1 for manifest policy failures, and 2 for
usage or unreadable/invalid input.
"""

from __future__ import annotations

import hashlib
import posixpath
import re
import sys
import tomllib
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

# Managed configuration (#6695).
# `LIBREFANG_CONFIG_PATH` relocates the file and `LIBREFANG_CONFIG_MODE=managed` locks it; the two are independent, so the checks below only fire once the manifest has actually opted into the mode.
CONFIG_MODE_ENV = "LIBREFANG_CONFIG_MODE"
CONFIG_PATH_ENV = "LIBREFANG_CONFIG_PATH"
MANAGED_MODE = "managed"

# The pod-template annotation whose change is what rolls the StatefulSet when the config changes.
# `checksum/config` is the Helm convention for exactly this, so an operator reading the template already knows what it is for.
CHECKSUM_ANNOTATION = "checksum/config"

# Config keys that carry a credential *value* rather than a reference to one.
# A ConfigMap is stored unencrypted in etcd and this copy is in git besides, so a hit here is a leaked secret, not a style problem.
# Keys ending `_env` name an environment variable and are deliberately absent: that is the supported way to point at a Secret from the config file.
SECRET_VALUE_KEYS = (
    "api_key",
    "dashboard_pass",
    "vault_key",
    "state_secret",
    "client_secret",
    "password",
)
SECRET_ASSIGNMENT = re.compile(
    r"^\s*(" + "|".join(SECRET_VALUE_KEYS) + r")\s*=\s*(\"[^\"]*\"|'[^']*')",
    re.MULTILINE,
)
# `vault:` and `env:` values are indirections, which is the pattern this check exists to steer people towards rather than away from.
SECRET_INDIRECTIONS = ("vault:", "env:")

# `include = [...]` pulls further TOML files into the effective configuration.
# Since #6695 the checksum on `GET /api/config/status` covers the whole include closure rather than the primary file alone, so an included file can be part of a managed deployment — but only when the manifest actually renders it, which is what `check_include_targets` below verifies.
INCLUDE_ARRAY = re.compile(r"^[ \t]*include[ \t]*=[ \t]*\[(.*?)\]", re.MULTILINE | re.DOTALL)
INCLUDE_ITEM = re.compile(r"""["']([^"']+)["']""")

# Mirrors `MAX_INCLUDE_DEPTH` in crates/librefang-kernel/src/config.rs.
MAX_INCLUDE_DEPTH = 10

# Declarative resource provisioning (#6695).
# `LIBREFANG_PROVISIONING_PATH` switches the feature on; unset means off, so the checks below stay silent for every manifest that has not opted in.
# A ConfigMap mounts its keys flat in one directory, so the mount has to supply `<root>/agents` rather than the root itself.
PROVISIONING_PATH_ENV = "LIBREFANG_PROVISIONING_PATH"
PROVISIONING_PRUNE_ENV = "LIBREFANG_PROVISIONING_PRUNE"
PROVISIONING_AGENTS_SUBDIR = "agents"
PROVISIONING_CHECKSUM_ANNOTATION = "checksum/provisioning"


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

    # An init container copying config.toml into /data is the workaround
    # managed mode exists to remove (#6695): the daemon resolves
    # LIBREFANG_CONFIG_PATH itself, so a read-only mount is bootable directly.
    # A copy would also defeat the lock, because the daemon would then own a
    # writable duplicate the manifest no longer controls.
    init_containers = pod_spec.get("initContainers", [])
    failures.check(
        not init_containers,
        "the pod template declares initContainers "
        f"({[c.get('name') for c in init_containers]!r}). A config.toml copy "
        "step is not needed — LIBREFANG_CONFIG_PATH boots straight off a "
        "read-only mount — and a writable duplicate under /data would be a "
        "second source of truth the deployment does not control.",
    )

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
        if not isinstance(probe, dict):
            failures.fail(f"{probe_name} must be a mapping, got {probe!r}.")
            continue
        http_get = probe.get("httpGet", {})
        if not isinstance(http_get, dict):
            failures.fail(f"{probe_name}.httpGet must be a mapping, got {http_get!r}.")
            continue
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
    if not isinstance(startup, dict):
        return
    period = startup.get("periodSeconds", 10)
    threshold = startup.get("failureThreshold", 3)
    if (
        not isinstance(period, int)
        or isinstance(period, bool)
        or not isinstance(threshold, int)
        or isinstance(threshold, bool)
    ):
        failures.fail(
            "startupProbe periodSeconds and failureThreshold must be integers, "
            f"got {period!r} and {threshold!r}."
        )
        return
    failures.check(
        period * threshold >= 60,
        f"startupProbe budget is only {period * threshold}s "
        f"(periodSeconds {period} x failureThreshold {threshold}). First boot runs "
        "`librefang init`, which writes config.toml and seeds the registry — give "
        "it at least 60s or slow storage will crash-loop a healthy install.",
    )


def check_managed_config(
    docs: list[dict[str, Any]], sts: dict[str, Any], failures: Failures
) -> None:
    """Assert the managed-configuration contract (#6695), when opted into.

    Silent unless the manifest sets `LIBREFANG_CONFIG_MODE`, so the base kustomization — which is deliberately mutable — passes unchanged.
    """
    template = sts.get("spec", {}).get("template", {})
    pod_spec = template.get("spec", {})
    containers = pod_spec.get("containers", [])
    if len(containers) != 1:
        return  # already reported by check_statefulset
    container = containers[0]
    env = {e.get("name"): e for e in container.get("env", []) if isinstance(e, dict)}

    if CONFIG_MODE_ENV not in env:
        return

    mode_entry = env[CONFIG_MODE_ENV]
    if mode_entry.get("valueFrom") is not None:
        failures.fail(
            f"{CONFIG_MODE_ENV} must be a literal value in the manifest, not a "
            "valueFrom reference. The lock is what stops the API from editing "
            "the deployment's own config, so which mode is in force has to be "
            "readable from the manifest rather than from another object."
        )
        return
    mode = mode_entry.get("value")
    if mode != MANAGED_MODE:
        failures.fail(
            f"{CONFIG_MODE_ENV} is {mode!r}. Any value other than "
            f"{MANAGED_MODE!r} — including a typo, an empty string, and a "
            "capitalised variant — resolves to mutable, silently and by "
            "design. Set it exactly or remove it."
        )
        return

    path_entry = env.get(CONFIG_PATH_ENV, {})
    if path_entry.get("valueFrom") is not None:
        failures.fail(
            f"{CONFIG_PATH_ENV} must be a literal value in the manifest, not a "
            "valueFrom reference — it names the file this checker has to find "
            "a mount for."
        )
        return
    config_path = path_entry.get("value")
    if not config_path:
        failures.fail(
            f"{CONFIG_MODE_ENV}={MANAGED_MODE} without {CONFIG_PATH_ENV}. The "
            f"daemon would lock {DATA_DIR}/config.toml, a file on the PVC that "
            "the deployment does not supply, so first boot has nothing to read "
            "and no way to write it."
        )
        return

    if not posixpath.isabs(config_path):
        failures.fail(f"{CONFIG_PATH_ENV} must be absolute, got {config_path!r}.")
        return

    if config_path == DATA_DIR or config_path.startswith(DATA_DIR + "/"):
        failures.fail(
            f"{CONFIG_PATH_ENV} {config_path!r} is inside {DATA_DIR}, which is "
            "the PVC. A ConfigMap mounted there would shadow the daemon's own "
            "state directory; a path outside it is the point of relocating."
        )
        return

    config_dir, config_key = posixpath.split(config_path)
    mount = _find_config_mount(container, config_path, config_dir)
    if mount is None:
        failures.fail(
            f"no volumeMount supplies {config_path!r}. Mount the ConfigMap at "
            f"{config_dir!r}, or at {config_path!r} with a matching subPath."
        )
        return

    failures.check(
        mount.get("readOnly") is True,
        f"the volumeMount supplying {config_path!r} must set readOnly: true. "
        "It is defence in depth rather than the enforcement mechanism — the "
        "423 is — but a manifest that declares a managed file and mounts it "
        "writable is stating two different intentions.",
    )
    if mount.get("subPath"):
        config_key = mount["subPath"]

    volume = next(
        (v for v in pod_spec.get("volumes", []) if v.get("name") == mount.get("name")),
        None,
    )
    if volume is None or "configMap" not in volume:
        failures.fail(
            f"volume {mount.get('name')!r} must be a configMap volume — a "
            "managed config comes from the manifest, and this checker verifies "
            "the checksum annotation against its rendered contents."
        )
        return

    cm_name = volume["configMap"].get("name")
    config_map = next(
        (
            d
            for d in docs
            if d.get("kind") == "ConfigMap" and d.get("metadata", {}).get("name") == cm_name
        ),
        None,
    )
    if config_map is None:
        failures.fail(
            f"ConfigMap {cm_name!r} is referenced but not rendered by this "
            "kustomization. An out-of-band ConfigMap cannot be checked here, "
            "and the checksum annotation could not be verified against it."
        )
        return

    contents = config_map.get("data", {}).get(config_key)
    if contents is None:
        failures.fail(
            f"ConfigMap {cm_name!r} has no data key {config_key!r}; it has "
            f"{sorted(config_map.get('data', {}))!r}. The key name is the "
            f"filename the mount exposes, so it must match {config_path!r}."
        )
        return

    data = config_map.get("data", {})
    check_no_secret_values(cm_name, config_key, contents, failures)
    chain = check_include_targets(
        cm_name, config_key, data, bool(mount.get("subPath")), failures
    )
    if chain is None:
        return
    check_checksum_annotation(template, data, chain, failures)


def _find_config_mount(
    container: dict[str, Any], config_path: str, config_dir: str
) -> dict[str, Any] | None:
    """The volumeMount that puts `config_path` in the container's filesystem.

    Either the whole directory is mounted, or the single file is mounted with a subPath.
    Both are valid; a subPath mount additionally never picks up a ConfigMap update in place, which is consistent with rollout-only updates.
    """
    for mount in container.get("volumeMounts", []):
        if mount.get("subPath") and mount.get("mountPath") == config_path:
            return mount
        if not mount.get("subPath") and mount.get("mountPath") == config_dir:
            return mount
    return None


def check_no_secret_values(
    cm_name: str, key: str, contents: str, failures: Failures
) -> None:
    """No credential may be spelled out in a ConfigMap."""
    for match in SECRET_ASSIGNMENT.finditer(contents):
        field, raw = match.group(1), match.group(2)
        value = raw[1:-1]
        if not value or value.startswith(SECRET_INDIRECTIONS):
            continue
        failures.fail(
            f"ConfigMap {cm_name!r} key {key!r} assigns {field} a literal "
            "value. A ConfigMap is unencrypted in etcd and readable by anyone "
            "with `get configmaps` in the namespace, and this copy is in "
            "version control. Supply it from a Secret through the pod "
            "environment instead — see deploy/kubernetes/secrets.example.yaml."
        )


def check_include_targets(
    cm_name: str,
    config_key: str,
    data: dict[str, str],
    mount_is_subpath: bool,
    failures: Failures,
) -> list[str] | None:
    """Resolve `include = [...]` against the ConfigMap's own keys and return the source chain.

    The daemon resolves an include relative to the primary file's directory, and a directory-mounted ConfigMap puts every data key in that one directory — so `include = ["extra.toml"]` works exactly when `extra.toml` is another key of the same ConfigMap.
    Anything else the daemon would silently skip or fail to read, which is worth catching in CI rather than at boot.

    Returns the ordered, deduplicated list of contributing keys (primary first), or `None` when the chain is unusable and the caller should stop.
    """
    if mount_is_subpath and INCLUDE_ARRAY.search(data.get(config_key, "")):
        failures.fail(
            f"ConfigMap {cm_name!r} key {config_key!r} uses `include = [...]` "
            "but is mounted with a subPath, which exposes this one file and "
            "nothing else. The included files would not exist in the "
            "container. Mount the whole directory instead."
        )
        return None

    chain: list[str] = []
    seen: set[str] = set()

    def walk(key: str, depth: int) -> bool:
        if key in seen:
            return True
        seen.add(key)
        chain.append(key)
        if depth >= MAX_INCLUDE_DEPTH:
            failures.fail(
                f"ConfigMap {cm_name!r} `include` nesting exceeds the daemon's "
                f"maximum depth of {MAX_INCLUDE_DEPTH}."
            )
            return False
        match = INCLUDE_ARRAY.search(data.get(key, ""))
        if match is None:
            return True
        for target in INCLUDE_ITEM.findall(match.group(1)):
            if target.startswith("/") or ".." in target.split("/"):
                failures.fail(
                    f"ConfigMap {cm_name!r} key {key!r} includes {target!r}. "
                    "The daemon rejects absolute paths and `..` components, so "
                    "this include is silently skipped and the file it names "
                    "never reaches the effective configuration."
                )
                return False
            if "/" in target:
                failures.fail(
                    f"ConfigMap {cm_name!r} key {key!r} includes {target!r}, "
                    "but a ConfigMap key cannot contain '/' and every key is "
                    "mounted flat in one directory. A subdirectory include can "
                    "never resolve here."
                )
                return False
            if target not in data:
                failures.fail(
                    f"ConfigMap {cm_name!r} key {key!r} includes {target!r}, "
                    f"which this kustomization does not render — it has "
                    f"{sorted(data)!r}. Add it to the configMapGenerator, or "
                    "fold its contents into the primary file."
                )
                return False
            if not walk(target, depth + 1):
                return False
        return True

    if not walk(config_key, 0):
        return None
    return chain


def check_checksum_annotation(
    template: dict[str, Any],
    data: dict[str, str],
    chain: list[str],
    failures: Failures,
) -> None:
    """The rollout trigger and the rollout *proof* must be the same string.

    `GET /api/config/status` reports `sha256:<hex>` over everything that contributes to the effective configuration, and the ConfigMap's data values are those bytes.
    Pinning the annotation to the same digest means one comparison answers both "will editing the config roll the pod?" and "is the running daemon on the file I edited?" — and a stale annotation, which would silently skip the rollout, fails here instead of in production.

    With no `include` the digest is over the primary file's raw bytes, unchanged from before the include closure was folded in, so an existing annotation keeps matching.
    With includes it is the digest of `sha256sum` output over the chain, matching `config_provenance` in crates/librefang-kernel/src/config.rs.
    """
    annotations = template.get("metadata", {}).get("annotations", {})
    actual = annotations.get(CHECKSUM_ANNOTATION)
    expected = f"sha256:{expected_config_digest(data, chain)}"

    if actual is None:
        failures.fail(
            f"the pod template has no {CHECKSUM_ANNOTATION!r} annotation. "
            "Without it, editing the ConfigMap leaves the pod template "
            "identical, so `kubectl apply -k` reports no change and the "
            f"running daemon keeps the old file. Expected {expected!r}."
        )
        return

    failures.check(
        actual == expected,
        f"{CHECKSUM_ANNOTATION} is {actual!r} but the rendered config hashes "
        f"to {expected!r}. The config changed and the annotation did not, so "
        "applying this would not roll the StatefulSet and the annotation would "
        "no longer match the checksum GET /api/config/status reports.",
    )


def expected_config_digest(data: dict[str, str], chain: list[str]) -> str:
    """The hex digest `GET /api/config/status` will report for this ConfigMap."""
    if len(chain) <= 1:
        return hashlib.sha256(data[chain[0]].encode()).hexdigest()
    manifest = "".join(
        f"{hashlib.sha256(data[key].encode()).hexdigest()}  {key}\n" for key in chain
    )
    return hashlib.sha256(manifest.encode()).hexdigest()


def check_provisioning(
    docs: list[dict[str, Any]], sts: dict[str, Any], failures: Failures
) -> None:
    """The declarative provisioning tree, when the manifest opts into one (#6695).

    Silent unless the container sets `LIBREFANG_PROVISIONING_PATH`, so the base kustomization and every existing deployment pass unchanged.

    The daemon reconciles this tree at boot and then locks each declared agent individually, which makes the manifest the only place those agents can be changed — so the same two things have to hold as for the managed config: the files have to actually reach the container, and editing them has to roll the pod.
    """
    template = sts.get("spec", {}).get("template", {})
    pod_spec = template.get("spec", {})
    containers = pod_spec.get("containers", [])
    if len(containers) != 1:
        return
    container = containers[0]
    env = {e.get("name"): e for e in container.get("env", []) if isinstance(e, dict)}

    if PROVISIONING_PATH_ENV not in env:
        return

    entry = env[PROVISIONING_PATH_ENV]
    if "value" not in entry:
        failures.fail(
            f"{PROVISIONING_PATH_ENV} must be set as a literal `value`, not "
            "valueFrom. Which resources the deployment owns has to be readable "
            "from the manifest rather than from another object."
        )
        return

    root = entry["value"].strip()
    if not root:
        failures.fail(
            f"{PROVISIONING_PATH_ENV} is empty, which the daemon reads as "
            "provisioning being switched off. Remove the variable or give it a "
            "path."
        )
        return
    if not posixpath.isabs(root):
        failures.fail(
            f"{PROVISIONING_PATH_ENV} must be an absolute path, got {root!r}."
        )
        return
    if root == DATA_DIR or root.startswith(DATA_DIR + "/"):
        failures.fail(
            f"{PROVISIONING_PATH_ENV} is {root!r}, inside {DATA_DIR}. The "
            "provisioning tree is deployment-owned and the PVC is runtime "
            "state; putting one inside the other gives the same file two "
            "owners."
        )
        return

    prune = env.get(PROVISIONING_PRUNE_ENV, {}).get("value")
    if prune is not None and prune.strip() and prune.strip().lower() != "delete":
        failures.fail(
            f"{PROVISIONING_PRUNE_ENV} is {prune!r}, which the daemon reads as "
            "`keep` — only the exact word `delete` prunes. Set it to `delete` "
            "or remove it, rather than leaving a value that reads as intent it "
            "does not carry."
        )

    agents_dir = posixpath.join(root, PROVISIONING_AGENTS_SUBDIR)
    mount = next(
        (
            m
            for m in container.get("volumeMounts", [])
            if not m.get("subPath") and m.get("mountPath") == agents_dir
        ),
        None,
    )
    if mount is None:
        failures.fail(
            f"no volumeMount supplies {agents_dir!r}. A ConfigMap mounts its "
            "keys flat in one directory, so the agent declarations have to be "
            f"mounted at the `{PROVISIONING_AGENTS_SUBDIR}` subdirectory of the "
            "provisioning root, not at the root."
        )
        return

    failures.check(
        mount.get("readOnly") is True,
        f"the volumeMount supplying {agents_dir!r} must set readOnly: true. "
        "The daemon never writes into the provisioning tree — a provisioned "
        "agent's manifest is materialised into its own workspace instead — so "
        "a writable mount states an intention the daemon does not have.",
    )

    volume = next(
        (v for v in pod_spec.get("volumes", []) if v.get("name") == mount.get("name")),
        None,
    )
    if volume is None or "configMap" not in volume:
        failures.fail(
            f"volume {mount.get('name')!r} must be a configMap volume — "
            "provisioned resources come from the manifest, and this checker "
            "verifies the checksum annotation against its rendered contents."
        )
        return

    cm_name = volume["configMap"].get("name")
    config_map = next(
        (
            d
            for d in docs
            if d.get("kind") == "ConfigMap" and d.get("metadata", {}).get("name") == cm_name
        ),
        None,
    )
    if config_map is None:
        failures.fail(
            f"ConfigMap {cm_name!r} is referenced but not rendered by this "
            "kustomization. An out-of-band ConfigMap cannot be checked here, "
            "and the checksum annotation could not be verified against it."
        )
        return

    data = config_map.get("data", {})
    if not data:
        failures.fail(
            f"ConfigMap {cm_name!r} renders no data, so the provisioning tree "
            "is empty and the feature does nothing. Remove "
            f"{PROVISIONING_PATH_ENV} or declare a resource."
        )
        return

    for key in sorted(data):
        check_provisioned_agent(cm_name, key, data[key], failures)
        check_no_secret_values(cm_name, key, data[key], failures)

    check_provisioning_checksum(template, data, failures)


def check_provisioned_agent(
    cm_name: str, key: str, contents: str, failures: Failures
) -> None:
    """One declaration the daemon has to be able to use.

    The reconcile records a bad file as a failure and carries on rather than refusing to boot, which is the right behaviour at runtime and the wrong place to discover a typo — the agent is simply missing, and only `GET /api/provisioning/status` says why.
    Catching it here means the manifest does not merge in the first place.
    """
    if not key.endswith(".toml"):
        failures.fail(
            f"ConfigMap {cm_name!r} key {key!r} is not a `.toml` file. The "
            "reconcile only reads `*.toml` from the agents directory, so this "
            "key is mounted and ignored."
        )
        return

    try:
        parsed = tomllib.loads(contents)
    except tomllib.TOMLDecodeError as exc:
        failures.fail(
            f"ConfigMap {cm_name!r} key {key!r} is not valid TOML: {exc}. The "
            "daemon would record this as a provisioning failure and start "
            "without the agent."
        )
        return

    name = parsed.get("name")
    if not isinstance(name, str) or not name.strip():
        failures.fail(
            f"ConfigMap {cm_name!r} key {key!r} declares no `name`. The "
            "resource identifier is the manifest's `name`, not the file name, "
            "and the reconcile refuses a manifest without one rather than "
            "provisioning an agent called `unnamed`."
        )


def check_provisioning_checksum(
    template: dict[str, Any], data: dict[str, str], failures: Failures
) -> None:
    """Editing a declaration has to roll the pod, exactly as editing the config does.

    The reconcile runs at boot and nowhere else, so a ConfigMap edit that does not roll the StatefulSet changes nothing at all — and `GET /api/provisioning/status` would report the resource as `drifted` indefinitely with no indication that a rollout was ever expected.
    """
    annotations = template.get("metadata", {}).get("annotations", {})
    actual = annotations.get(PROVISIONING_CHECKSUM_ANNOTATION)
    manifest = "".join(
        f"{hashlib.sha256(data[key].encode()).hexdigest()}  {key}\n"
        for key in sorted(data)
    )
    expected = f"sha256:{hashlib.sha256(manifest.encode()).hexdigest()}"

    if actual is None:
        failures.fail(
            f"the pod template has no {PROVISIONING_CHECKSUM_ANNOTATION!r} "
            "annotation. The provisioning tree is reconciled at boot only, so "
            "without it an edited declaration leaves the pod template "
            "identical and never reaches a running daemon. Expected "
            f"{expected!r}."
        )
        return

    failures.check(
        actual == expected,
        f"{PROVISIONING_CHECKSUM_ANNOTATION} is {actual!r} but the rendered "
        f"declarations hash to {expected!r}. A declaration changed and the "
        "annotation did not, so applying this would not roll the StatefulSet "
        "and the daemon would keep provisioning the old manifest.",
    )


def check_services(
    docs: list[dict[str, Any]],
    statefulset: dict[str, Any] | None,
    failures: Failures,
) -> None:
    services = [d for d in docs if d.get("kind") == "Service"]
    if not failures.check(
        len(services) >= 1, "expected at least one Service exposing the daemon."
    ):
        return

    governing = (statefulset or {}).get("spec", {}).get("serviceName")
    by_name = {s.get("metadata", {}).get("name"): s for s in services}

    failures.check(
        governing in by_name,
        f"StatefulSet.spec.serviceName {governing!r} does not name a Service in "
        f"this kustomization (have {sorted(by_name, key=repr)!r}).",
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
        sys.stderr.write(f"usage: {argv[0]} [rendered.yaml]\n")
        return 2

    input_label = repr(argv[1]) if len(argv) == 2 else "from stdin"
    try:
        source = (
            Path(argv[1]).read_text(encoding="utf-8")
            if len(argv) == 2
            else sys.stdin.read()
        )
    except (OSError, UnicodeError) as error:
        sys.stderr.write(f"error: cannot read manifest input {input_label}: {error}\n")
        return 2
    if not source.strip():
        sys.stderr.write("error: no manifest input (stdin was empty)\n")
        return 2

    try:
        docs = load_documents(source)
    except yaml.YAMLError as error:
        sys.stderr.write(f"error: invalid YAML manifest input: {error}\n")
        return 2
    failures = Failures()

    try:
        statefulsets = [doc for doc in docs if doc.get("kind") == "StatefulSet"]
        sts = statefulsets[0] if len(statefulsets) == 1 else None
        if len(statefulsets) != 1:
            kinds = sorted({d.get("kind") for d in docs}, key=repr)
            failures.fail(
                f"expected exactly one StatefulSet, found {len(statefulsets)}; "
                f"rendered kinds: {kinds!r}"
            )
        else:
            check_statefulset(sts, failures)
            check_managed_config(docs, sts, failures)
            check_provisioning(docs, sts, failures)

        check_services(docs, sts, failures)
        check_no_inline_secrets(docs, failures)
    except (AttributeError, TypeError) as error:
        sys.stderr.write(f"error: invalid Kubernetes manifest structure: {error}\n")
        return 2

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
