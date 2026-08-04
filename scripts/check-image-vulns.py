#!/usr/bin/env python3
"""Summarise a Trivy container-image report and apply the release vulnerability gate.

Consumes the JSON report produced by `trivy image --format json` and does three things:

1. Writes a human-readable job summary to `$GITHUB_STEP_SUMMARY` naming the platform digest, the scanner and vulnerability-database versions, and every CRITICAL / HIGH finding with its package, installed version, fixed version, CVE id, and severity.
2. Writes a machine-readable `--summary-json` next to the raw report so the retained artifact carries the verdict and its inputs, not just the raw findings.
3. Applies the enforcement threshold and exits accordingly.

Exit codes are the contract the workflow depends on, and they deliberately separate "the image is dirty" from "the scanner did not work":

* `0` — the report was produced and the gate passed (or the gate is off).
* `1` — the gate failed: findings at or above the configured threshold.
* `2` — SCANNER INFRASTRUCTURE FAILURE: the report is missing, truncated, or structurally implausible.
  A vulnerability-database download failure, an aborted scan, or a zero-byte report must never be indistinguishable from a clean image, so this case is loud and separate.

The gate counts only **fixable** vulnerabilities.
That is not `--ignore-unfixed`: the report, the artifacts, and the job summary contain every finding at every severity including the unfixed ones.
Only the pass/fail decision is narrowed, because an unfixed CVE gives a maintainer nothing to act on and would make the gate permanently red for reasons outside the project's control.

Detected secrets fail whenever the gate is armed at all, regardless of the severity threshold.
Issue #6694's Trivy 0.57.0 scan found no embedded secrets in the image, so there is no known backlog to grandfather and no reason to let one through at a lower threshold.
That measurement predates the pinned 0.72.0 and its newer secret rules, so it is a prior rather than a verified baseline.

Run `--self-test` to execute the embedded fixture corpus, which is what proves the gate actually fails at the configured threshold without needing a live vulnerable image.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any

# Ascending order of seriousness.
# Index position is the comparison key, so a threshold of HIGH matches HIGH and CRITICAL and nothing below it.
SEVERITY_ORDER = ["UNKNOWN", "LOW", "MEDIUM", "HIGH", "CRITICAL"]

# `off` is the report-only setting and the shipped default.
# See the `fail-on` input of .github/actions/trivy-image-scan/action.yml for the single place a maintainer flips it.
THRESHOLDS = ["off", "unknown", "low", "medium", "high", "critical"]

EXIT_OK = 0
EXIT_GATE_FAILED = 1
EXIT_INFRA_FAILURE = 2

# Trivy marks a vulnerability it has a fix for as `fixed`; `affected`, `will_not_fix`, `fix_deferred`, and `end_of_life` all mean no fix is available.
# A non-empty FixedVersion is the older, more portable signal and some ecosystems populate it without setting Status, so either one counts.
FIXABLE_STATUSES = {"fixed"}


def severity_rank(severity: str) -> int:
    """Return the comparison rank of a Trivy severity string, defaulting to UNKNOWN."""
    try:
        return SEVERITY_ORDER.index((severity or "UNKNOWN").upper())
    except ValueError:
        return 0


class InfraFailure(Exception):
    """The report is absent or structurally implausible, so the scan cannot be trusted."""


def load_report(path: Path) -> dict[str, Any]:
    """Read a Trivy JSON report, rejecting anything that cannot be a completed scan."""
    if not path.is_file():
        raise InfraFailure(f"report file {path} does not exist — the scan step did not produce output")
    raw = path.read_text(encoding="utf-8", errors="replace").strip()
    if not raw:
        raise InfraFailure(f"report file {path} is empty — the scan step was interrupted before writing results")
    try:
        report = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise InfraFailure(f"report file {path} is not valid JSON ({exc}) — the scan step wrote a truncated report") from exc
    if not isinstance(report, dict):
        raise InfraFailure(f"report file {path} does not contain a Trivy report object")
    if "SchemaVersion" not in report:
        raise InfraFailure(f"report file {path} has no SchemaVersion — it is not a Trivy report")
    metadata = report.get("Metadata")
    if not isinstance(metadata, dict):
        raise InfraFailure(f"report file {path} has no Metadata block — the scanner did not finish inspecting the image")
    results = report.get("Results") or []
    if not metadata.get("OS") and not results:
        # The runtime image is Debian-based (node:*-bookworm-slim), so Trivy always identifies an OS for it.
        # A report with neither an OS nor a single result means the vulnerability database was unusable, not that the image is clean.
        raise InfraFailure(
            f"report file {path} identifies no operating system and contains no results — "
            "treating as a scanner failure rather than a clean image"
        )
    return report


def collect_findings(report: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Flatten a Trivy report into a vulnerability list and a secret list."""
    vulnerabilities: list[dict[str, Any]] = []
    secrets: list[dict[str, Any]] = []
    for result in report.get("Results") or []:
        if not isinstance(result, dict):
            continue
        target = result.get("Target", "")
        result_class = result.get("Class", "")
        for vuln in result.get("Vulnerabilities") or []:
            if not isinstance(vuln, dict):
                continue
            fixed_version = (vuln.get("FixedVersion") or "").strip()
            status = (vuln.get("Status") or "").strip().lower()
            vulnerabilities.append(
                {
                    "target": target,
                    "class": result_class,
                    "id": vuln.get("VulnerabilityID", ""),
                    "package": vuln.get("PkgName", ""),
                    "installed_version": vuln.get("InstalledVersion", ""),
                    "fixed_version": fixed_version,
                    "severity": (vuln.get("Severity") or "UNKNOWN").upper(),
                    "status": status,
                    "fixable": bool(fixed_version) or status in FIXABLE_STATUSES,
                    "url": vuln.get("PrimaryURL", ""),
                }
            )
        for secret in result.get("Secrets") or []:
            if not isinstance(secret, dict):
                continue
            secrets.append(
                {
                    "target": target,
                    "rule_id": secret.get("RuleID", ""),
                    "category": secret.get("Category", ""),
                    "title": secret.get("Title", ""),
                    "severity": (secret.get("Severity") or "UNKNOWN").upper(),
                    "start_line": secret.get("StartLine", 0),
                }
            )
    return vulnerabilities, secrets


def summarise(vulnerabilities: list[dict[str, Any]]) -> dict[str, dict[str, int]]:
    """Return per-severity total / fixable counts and unique-CVE counts."""
    counts: dict[str, dict[str, int]] = {
        severity: {"total": 0, "fixable": 0, "unique": 0} for severity in SEVERITY_ORDER
    }
    unique: dict[str, set[str]] = {severity: set() for severity in SEVERITY_ORDER}
    for vuln in vulnerabilities:
        severity = vuln["severity"] if vuln["severity"] in counts else "UNKNOWN"
        counts[severity]["total"] += 1
        if vuln["fixable"]:
            counts[severity]["fixable"] += 1
        if vuln["id"]:
            unique[severity].add(vuln["id"])
    for severity, ids in unique.items():
        counts[severity]["unique"] = len(ids)
    return counts


def image_digest(report: dict[str, Any], override: str) -> str:
    """Resolve the digest identifying the exact image that was scanned."""
    if override:
        return override
    metadata = report.get("Metadata") or {}
    repo_digests = metadata.get("RepoDigests") or []
    if repo_digests:
        return str(repo_digests[0])
    return str(metadata.get("ImageID") or "unknown")


def gate(
    vulnerabilities: list[dict[str, Any]],
    secrets: list[dict[str, Any]],
    fail_on: str,
) -> tuple[str, list[str]]:
    """Apply the enforcement threshold, returning the verdict and the reasons behind it."""
    if fail_on == "off":
        return "report-only", []
    threshold = severity_rank(fail_on.upper())
    blocking = [v for v in vulnerabilities if v["fixable"] and severity_rank(v["severity"]) >= threshold]
    reasons: list[str] = []
    if blocking:
        by_severity: dict[str, int] = {}
        for vuln in blocking:
            by_severity[vuln["severity"]] = by_severity.get(vuln["severity"], 0) + 1
        breakdown = ", ".join(
            f"{by_severity[s]} {s}" for s in reversed(SEVERITY_ORDER) if s in by_severity
        )
        reasons.append(f"{len(blocking)} fixable finding(s) at or above {fail_on.upper()} ({breakdown})")
    if secrets:
        reasons.append(f"{len(secrets)} embedded secret(s) detected")
    return ("fail" if reasons else "pass"), reasons


def render_summary(
    *,
    image_ref: str,
    platform: str,
    digest: str,
    trivy_version: str,
    db_updated_at: str,
    scanners: str,
    pkg_types: str,
    fail_on: str,
    verdict: str,
    reasons: list[str],
    counts: dict[str, dict[str, int]],
    vulnerabilities: list[dict[str, Any]],
    secrets: list[dict[str, Any]],
    top_n: int,
) -> str:
    """Build the markdown block written to $GITHUB_STEP_SUMMARY."""
    icon = {"pass": "✅", "report-only": "📋", "fail": "❌"}.get(verdict, "❓")
    lines: list[str] = []
    lines.append(f"## {icon} Container image vulnerability scan — `{platform}`")
    lines.append("")
    lines.append("| Field | Value |")
    lines.append("| --- | --- |")
    lines.append(f"| Image | `{image_ref}` |")
    lines.append(f"| Platform | `{platform}` |")
    lines.append(f"| Digest | `{digest}` |")
    lines.append(f"| Scanner | Trivy `{trivy_version}` |")
    lines.append(f"| Vulnerability DB updated | `{db_updated_at}` |")
    lines.append(f"| Scanners | `{scanners}` |")
    lines.append(f"| Package types | `{pkg_types}` |")
    lines.append(f"| Enforcement threshold | `{fail_on}` |")
    lines.append(f"| Verdict | **{verdict}** |")
    lines.append("")

    if verdict == "report-only":
        lines.append(
            "> **This scan is report-only.** The image carries a pre-existing vulnerability backlog, so the threshold ships as `off`: findings are reported and retained but never fail the job. "
            "Flip the `fail-on` input of `.github/actions/trivy-image-scan` to `critical` (then `high`) once the backlog is remediated. "
            "Until then, nothing below blocks a release."
        )
        lines.append("")
    elif verdict == "fail":
        lines.append(f"> **Gate failed at threshold `{fail_on}`.**")
        for reason in reasons:
            lines.append(f"> - {reason}")
        lines.append("")

    lines.append("### Findings by severity")
    lines.append("")
    lines.append("| Severity | Findings | Unique CVEs | Fixable |")
    lines.append("| --- | ---: | ---: | ---: |")
    for severity in reversed(SEVERITY_ORDER):
        row = counts[severity]
        lines.append(f"| {severity} | {row['total']} | {row['unique']} | {row['fixable']} |")
    total = sum(row["total"] for row in counts.values())
    total_fixable = sum(row["fixable"] for row in counts.values())
    lines.append(f"| **Total** | **{total}** | | **{total_fixable}** |")
    lines.append("")
    lines.append(
        "Every severity is reported, fixed and unfixed alike — no `--ignore-unfixed` and no severity suppression. "
        "Only the pass/fail decision is narrowed to fixable findings."
    )
    lines.append("")

    lines.append(f"### Embedded secrets: {len(secrets)}")
    lines.append("")
    if secrets:
        lines.append("| Target | Rule | Severity |")
        lines.append("| --- | --- | --- |")
        for secret in secrets[:top_n]:
            lines.append(f"| `{secret['target']}` | {secret['rule_id']} | {secret['severity']} |")
        lines.append("")

    actionable = sorted(
        (v for v in vulnerabilities if severity_rank(v["severity"]) >= severity_rank("HIGH")),
        key=lambda v: (-severity_rank(v["severity"]), not v["fixable"], v["package"], v["id"]),
    )
    lines.append(f"### CRITICAL / HIGH findings ({len(actionable)})")
    lines.append("")
    if actionable:
        lines.append("| Severity | CVE | Package | Installed | Fixed in | Target |")
        lines.append("| --- | --- | --- | --- | --- | --- |")
        for vuln in actionable[:top_n]:
            fixed = f"`{vuln['fixed_version']}`" if vuln["fixed_version"] else "_no fix available_"
            lines.append(
                f"| {vuln['severity']} | {vuln['id']} | `{vuln['package']}` | "
                f"`{vuln['installed_version']}` | {fixed} | `{vuln['target']}` |"
            )
        if len(actionable) > top_n:
            lines.append("")
            lines.append(f"_{len(actionable) - top_n} further CRITICAL/HIGH findings omitted — see the uploaded JSON and SARIF artifacts for the complete list._")
    else:
        lines.append("_None._")
    lines.append("")
    return "\n".join(lines) + "\n"


def write_github_output(path: str, values: dict[str, str]) -> None:
    """Append key=value pairs to the GitHub Actions step output file."""
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def run(args: argparse.Namespace) -> int:
    """Produce the summary artifacts and return the process exit code."""
    report_path = Path(args.report)
    try:
        report = load_report(report_path)
    except InfraFailure as exc:
        print(f"::error title=Scanner infrastructure failure::{exc}", file=sys.stderr)
        message = (
            "## ❌ Container image vulnerability scan — scanner infrastructure failure\n\n"
            f"`{args.image_ref}` (`{args.platform}`) was **not** scanned successfully: {exc}\n\n"
            "This is not a clean result. Re-run the job; if it persists, the Trivy vulnerability database or its mirror is unavailable.\n"
        )
        if args.step_summary:
            with open(args.step_summary, "a", encoding="utf-8") as handle:
                handle.write(message)
        write_github_output(args.github_output, {"verdict": "infra-failure", "total": "0", "fixable": "0"})
        return EXIT_INFRA_FAILURE

    vulnerabilities, secrets = collect_findings(report)
    counts = summarise(vulnerabilities)
    digest = image_digest(report, args.digest)
    verdict, reasons = gate(vulnerabilities, secrets, args.fail_on)

    summary_markdown = render_summary(
        image_ref=args.image_ref,
        platform=args.platform,
        digest=digest,
        trivy_version=args.trivy_version,
        db_updated_at=args.db_updated_at,
        scanners=args.scanners,
        pkg_types=args.pkg_types,
        fail_on=args.fail_on,
        verdict=verdict,
        reasons=reasons,
        counts=counts,
        vulnerabilities=vulnerabilities,
        secrets=secrets,
        top_n=args.max_rows,
    )
    if args.step_summary:
        with open(args.step_summary, "a", encoding="utf-8") as handle:
            handle.write(summary_markdown)
    else:
        sys.stdout.write(summary_markdown)

    if args.summary_json:
        payload = {
            "image_ref": args.image_ref,
            "platform": args.platform,
            "digest": digest,
            "trivy_version": args.trivy_version,
            "db_updated_at": args.db_updated_at,
            "scanners": args.scanners,
            "pkg_types": args.pkg_types,
            "fail_on": args.fail_on,
            "verdict": verdict,
            "reasons": reasons,
            "counts": counts,
            "secrets": len(secrets),
            "total": sum(row["total"] for row in counts.values()),
            "fixable": sum(row["fixable"] for row in counts.values()),
        }
        Path(args.summary_json).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    write_github_output(
        args.github_output,
        {
            "verdict": verdict,
            "digest": digest,
            "total": str(sum(row["total"] for row in counts.values())),
            "fixable": str(sum(row["fixable"] for row in counts.values())),
            "critical": str(counts["CRITICAL"]["total"]),
            "high": str(counts["HIGH"]["total"]),
            "secrets": str(len(secrets)),
        },
    )

    if verdict == "fail":
        print(
            f"::error title=Container vulnerability gate failed::{args.image_ref} ({args.platform}): "
            + "; ".join(reasons),
            file=sys.stderr,
        )
        return EXIT_GATE_FAILED
    return EXIT_OK


# ─────────────────────────────────────────────────────────────────────────────
# Self-test — the controlled fixture corpus that proves the gate fires.
#
# Issue #6694 asks for "a vulnerable test image or controlled fixture" that demonstrates the gate fails at the configured threshold.
# Building a deliberately vulnerable image in CI is slow and its finding set drifts with every database update; a fixture report is deterministic and exercises the exact decision function the workflow calls.
# ─────────────────────────────────────────────────────────────────────────────


def _fixture(results: list[dict[str, Any]], *, with_os: bool = True) -> dict[str, Any]:
    """Build a minimal but structurally faithful Trivy report."""
    metadata: dict[str, Any] = {
        "ImageID": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "RepoDigests": ["ghcr.io/librefang/librefang@sha256:2222222222222222222222222222222222222222222222222222222222222222"],
    }
    if with_os:
        metadata["OS"] = {"Family": "debian", "Name": "12.11"}
    return {"SchemaVersion": 2, "ArtifactName": "ghcr.io/librefang/librefang", "Metadata": metadata, "Results": results}


def _vuln(cve: str, severity: str, fixed: str, status: str = "fixed") -> dict[str, Any]:
    """Build one vulnerability entry; an empty `fixed` with status `affected` means unfixable."""
    return {
        "VulnerabilityID": cve,
        "PkgName": "libgnutls30",
        "InstalledVersion": "3.7.9-2+deb12u3",
        "FixedVersion": fixed,
        "Severity": severity,
        "Status": status,
        "PrimaryURL": f"https://avd.aquasec.com/nvd/{cve.lower()}",
    }


def _decide(report: dict[str, Any] | str, fail_on: str) -> int:
    """Run the full pipeline against an in-memory fixture and return the exit code.

    stderr is swallowed for the duration: the failure paths emit `::error::` workflow commands, and the runner would turn those into red annotations on a self-test job that is in fact passing.
    """
    with tempfile.TemporaryDirectory() as tmp:
        report_path = Path(tmp) / "report.json"
        if isinstance(report, str):
            report_path.write_text(report, encoding="utf-8")
        else:
            report_path.write_text(json.dumps(report), encoding="utf-8")
        args = argparse.Namespace(
            report=str(report_path),
            image_ref="ghcr.io/librefang/librefang",
            platform="linux/amd64",
            digest="",
            trivy_version="0.72.0",
            db_updated_at="1970-01-01T00:00:00Z",
            scanners="vuln,secret",
            pkg_types="os,library",
            fail_on=fail_on,
            summary_json=str(Path(tmp) / "summary.json"),
            step_summary=str(Path(tmp) / "step-summary.md"),
            github_output=str(Path(tmp) / "output.txt"),
            max_rows=50,
        )
        original_stderr = sys.stderr
        try:
            with open(os.devnull, "w", encoding="utf-8") as devnull:
                sys.stderr = devnull
                return run(args)
        finally:
            sys.stderr = original_stderr


def self_test() -> int:
    """Execute the fixture corpus, printing one line per case."""
    fixable_critical = _fixture([
        {"Target": "image (debian 12.11)", "Class": "os-pkgs", "Type": "debian",
         "Vulnerabilities": [_vuln("CVE-2026-33845", "CRITICAL", "3.7.9-2+deb12u4")]},
    ])
    unfixed_critical = _fixture([
        {"Target": "image (debian 12.11)", "Class": "os-pkgs", "Type": "debian",
         "Vulnerabilities": [_vuln("CVE-2026-00001", "CRITICAL", "", status="will_not_fix")]},
    ])
    fixable_high = _fixture([
        {"Target": "image (debian 12.11)", "Class": "os-pkgs", "Type": "debian",
         "Vulnerabilities": [_vuln("CVE-2026-00002", "HIGH", "3.7.9-2+deb12u4")]},
    ])
    language_critical = _fixture([
        {"Target": "usr/local/lib/node_modules/npm/package-lock.json", "Class": "lang-pkgs", "Type": "node-pkg",
         "Vulnerabilities": [{"VulnerabilityID": "CVE-2026-59873", "PkgName": "tar", "InstalledVersion": "6.1.11",
                              "FixedVersion": "6.2.1", "Severity": "CRITICAL", "Status": "fixed"}]},
    ])
    clean = _fixture([
        {"Target": "image (debian 12.11)", "Class": "os-pkgs", "Type": "debian", "Vulnerabilities": []},
    ])
    with_secret = _fixture([
        {"Target": "image (debian 12.11)", "Class": "os-pkgs", "Type": "debian", "Vulnerabilities": []},
        {"Target": "/app/.env", "Class": "secret",
         "Secrets": [{"RuleID": "aws-access-key-id", "Category": "AWS", "Title": "AWS Access Key ID",
                      "Severity": "CRITICAL", "StartLine": 1}]},
    ])
    no_os_no_results = _fixture([], with_os=False)

    cases: list[tuple[str, dict[str, Any] | str, str, int]] = [
        ("clean image, threshold critical -> pass", clean, "critical", EXIT_OK),
        ("clean image, threshold off -> pass", clean, "off", EXIT_OK),
        ("fixable CRITICAL, threshold critical -> GATE FAILS", fixable_critical, "critical", EXIT_GATE_FAILED),
        ("fixable CRITICAL, threshold high -> GATE FAILS", fixable_critical, "high", EXIT_GATE_FAILED),
        ("fixable CRITICAL, threshold off -> report-only, passes", fixable_critical, "off", EXIT_OK),
        ("language-package CRITICAL, threshold critical -> GATE FAILS", language_critical, "critical", EXIT_GATE_FAILED),
        ("fixable HIGH, threshold critical -> passes", fixable_high, "critical", EXIT_OK),
        ("fixable HIGH, threshold high -> GATE FAILS", fixable_high, "high", EXIT_GATE_FAILED),
        ("unfixed CRITICAL, threshold critical -> passes (gate counts fixable only)", unfixed_critical, "critical", EXIT_OK),
        ("embedded secret, threshold critical -> GATE FAILS", with_secret, "critical", EXIT_GATE_FAILED),
        ("embedded secret, threshold off -> report-only, passes", with_secret, "off", EXIT_OK),
        ("empty report file -> INFRA FAILURE", "", "critical", EXIT_INFRA_FAILURE),
        ("truncated JSON -> INFRA FAILURE", '{"SchemaVersion": 2, "Results": [', "critical", EXIT_INFRA_FAILURE),
        ("JSON without Metadata -> INFRA FAILURE", '{"SchemaVersion": 2, "Results": []}', "critical", EXIT_INFRA_FAILURE),
        ("no OS and no results -> INFRA FAILURE", no_os_no_results, "critical", EXIT_INFRA_FAILURE),
        ("infra failure is not masked by threshold off", "", "off", EXIT_INFRA_FAILURE),
    ]

    failures = 0
    for name, report, fail_on, expected in cases:
        actual = _decide(report, fail_on)
        if actual == expected:
            print(f"  ok   {name}")
        else:
            failures += 1
            print(f"  FAIL {name} (expected exit {expected}, got {actual})")

    # The unfixed CRITICAL must still be visible in the report even though it does not gate — that is the difference between "narrow the decision" and "--ignore-unfixed hides the finding".
    vulns, _ = collect_findings(unfixed_critical)
    if len(vulns) != 1 or vulns[0]["fixable"]:
        failures += 1
        print("  FAIL unfixed CRITICAL is still reported and marked unfixable")
    else:
        print("  ok   unfixed CRITICAL is still reported and marked unfixable")

    print()
    if failures:
        print(f"{failures} self-test case(s) failed")
        return 1
    print(f"all {len(cases) + 1} self-test cases passed")
    return 0


def build_parser() -> argparse.ArgumentParser:
    """Construct the CLI parser."""
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--report", help="Path to the Trivy JSON report")
    parser.add_argument("--image-ref", default="", help="Image reference that was scanned")
    parser.add_argument("--platform", default="", help="Platform label, e.g. linux/amd64")
    parser.add_argument("--digest", default="", help="Image digest; falls back to the report's RepoDigests / ImageID")
    parser.add_argument("--trivy-version", default="", help="Scanner version string for the report header")
    parser.add_argument("--db-updated-at", default="", help="Vulnerability database timestamp for the report header")
    parser.add_argument("--scanners", default="vuln,secret", help="Trivy scanners that were enabled")
    parser.add_argument("--pkg-types", default="os,library", help="Trivy package types that were scanned")
    parser.add_argument(
        "--fail-on",
        default="off",
        choices=THRESHOLDS,
        help="Enforcement threshold; `off` reports without failing",
    )
    parser.add_argument("--summary-json", default="", help="Where to write the machine-readable verdict")
    parser.add_argument(
        "--step-summary",
        default=os.environ.get("GITHUB_STEP_SUMMARY", ""),
        help="Where to append the markdown job summary (default: $GITHUB_STEP_SUMMARY)",
    )
    parser.add_argument(
        "--github-output",
        default=os.environ.get("GITHUB_OUTPUT", ""),
        help="Where to append step outputs (default: $GITHUB_OUTPUT)",
    )
    parser.add_argument("--max-rows", type=int, default=50, help="Maximum finding rows rendered in the job summary")
    parser.add_argument("--self-test", action="store_true", help="Run the embedded fixture corpus and exit")
    return parser


def main(argv: list[str] | None = None) -> int:
    """Entry point."""
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.report:
        parser.error("--report is required unless --self-test is given")
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
