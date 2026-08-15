#!/usr/bin/env python3
"""Regression checks for release and repository automation safety."""

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = (
    ROOT / ".github" / "workflows" / "release-tag.yml",
    ROOT / ".github" / "workflows" / "release-cli.yml",
)
INPUTS_TOKEN = re.compile(r"\binputs\b")
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")


def contains_direct_input(script: str) -> bool:
    # Conservatively reject any shell source that contains both a GitHub
    # expression and the inputs context. This covers dot/bracket access,
    # github.event aliases, and wrappers whose string literals contain braces.
    return "${{" in script and INPUTS_TOKEN.search(script) is not None


def check_repository_automation() -> None:
    devcontainer = json.loads(
        (ROOT / ".devcontainer" / "devcontainer.json").read_text(encoding="utf-8")
    )
    post_create = devcontainer.get("postCreateCommand")
    if not isinstance(post_create, str):
        raise SystemExit("devcontainer has no string postCreateCommand")
    with tempfile.TemporaryDirectory() as temp_dir:
        fake_cargo = Path(temp_dir) / "cargo"
        fake_cargo.write_text(
            "#!/bin/sh\n"
            "if [ \"${FAKE_CARGO_FAIL:-}\" = 1 ]; then\n"
            "  echo simulated build failure >&2\n"
            "  exit 7\n"
            "fi\n"
            "echo simulated build success\n"
        )
        fake_cargo.chmod(0o755)
        environment = os.environ.copy()
        environment["PATH"] = f"{temp_dir}{os.pathsep}{environment['PATH']}"
        environment["FAKE_CARGO_FAIL"] = "1"
        failed = subprocess.run(
            ["sh", "-c", post_create],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        environment.pop("FAKE_CARGO_FAIL")
        succeeded = subprocess.run(
            ["sh", "-c", post_create],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
    if failed.returncode == 0 or "Build complete" in failed.stdout:
        raise SystemExit("devcontainer postCreateCommand masks cargo build failures")
    if succeeded.returncode != 0 or "Build complete" not in succeeded.stdout:
        raise SystemExit("devcontainer postCreateCommand does not report successful builds")

    ci_workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    )
    route_step = next(
        (
            step
            for step in ci_workflow.get("jobs", {}).get("changes", {}).get("steps", [])
            if step.get("name") == "Compute diff and route"
        ),
        None,
    )
    route_script = route_step.get("run", "") if isinstance(route_step, dict) else ""
    for routed_path in (
        r"\.devcontainer/",
        r"scripts/tests/test_release_tag_workflow_safety\.py",
    ):
        if routed_path not in route_script:
            raise SystemExit(f"CI routing does not cover {routed_path}")

    ignored_tests = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "kernel-ignored-tests.yml").read_text(
            encoding="utf-8"
        )
    )
    concurrency = ignored_tests.get("concurrency", {})
    group = concurrency.get("group") if isinstance(concurrency, dict) else None
    cancel = concurrency.get("cancel-in-progress") if isinstance(concurrency, dict) else None
    if not isinstance(group, str) or not all(
        token in group for token in ("github.event_name", "github.ref", "github.run_id")
    ):
        raise SystemExit("kernel ignored-tests runs do not isolate PR and non-PR concurrency")
    if cancel != "${{ github.event_name == 'pull_request' }}":
        raise SystemExit("kernel ignored-tests workflow does not cancel superseded PR runs")

    supply_chain = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "supply-chain-audit.yml").read_text(
            encoding="utf-8"
        )
    )
    expected_actions = {
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97",
    }
    jobs = supply_chain.get("jobs", {})
    for job_name in ("self-test", "audit"):
        job = jobs.get(job_name, {})
        actual_actions = {
            step.get("uses", "")
            for step in job.get("steps", [])
            if step.get("uses", "").startswith(
                ("actions/checkout@", "actions/setup-python@")
            )
        }
        if actual_actions != expected_actions:
            raise SystemExit(
                f"supply-chain-audit {job_name} actions are not pinned as expected: "
                f"{sorted(actual_actions)}"
            )
        if any(
            FULL_SHA.fullmatch(uses.rsplit("@", 1)[1]) is None
            for uses in actual_actions
        ):
            raise SystemExit(f"supply-chain-audit {job_name} has a non-SHA action pin")

    issue_pr_link = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "issue-pr-link.yml").read_text(
            encoding="utf-8"
        )
    )
    issue_link_triggers = issue_pr_link.get("on", issue_pr_link.get(True, {}))
    if set(issue_link_triggers.get("pull_request", {}).get("paths", [])) != {
        ".github/scripts/issue-pr-links.js",
        ".github/scripts/tests/issue-pr-links.test.js",
        ".github/workflows/issue-pr-link.yml",
    }:
        raise SystemExit("issue-link validation paths drifted")
    if issue_link_triggers.get("pull_request_target", {}).get("types") != [
        "opened",
        "edited",
        "closed",
        "reopened",
    ]:
        raise SystemExit("issue-link reconciliation privileged events drifted")
    if issue_pr_link.get("permissions") != {"contents": "read"}:
        raise SystemExit("issue-link workflow has broader default permissions than read-only")
    issue_link_jobs = issue_pr_link.get("jobs", {})
    test_job = issue_link_jobs.get("test", {})
    link_job = issue_link_jobs.get("link", {})
    if (
        test_job.get("if") != "github.event_name == 'pull_request'"
        or test_job.get("timeout-minutes") != 2
    ):
        raise SystemExit("issue-link unprivileged test gate drifted")
    if (
        link_job.get("if") != "github.event_name == 'pull_request_target'"
        or link_job.get("timeout-minutes") != 5
        or link_job.get("permissions")
        != {
            "contents": "read",
            "issues": "write",
            "pull-requests": "read",
        }
    ):
        raise SystemExit("issue-link privileged reconciliation gate drifted")
    if link_job.get("concurrency") != {
        "group": "issue-pr-link-${{ github.repository }}",
        "cancel-in-progress": False,
    }:
        raise SystemExit("issue-link mutations are not globally serialized")
    expected_checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    expected_github_script = (
        "actions/github-script@3a2844b7e9c422d3c10d287c895573f7108da1b3"
    )
    test_uses = [step.get("uses") for step in test_job.get("steps", []) if step.get("uses")]
    link_uses = [step.get("uses") for step in link_job.get("steps", []) if step.get("uses")]
    if test_uses != [expected_checkout] or link_uses != [
        expected_checkout,
        expected_github_script,
    ]:
        raise SystemExit("issue-link workflow action pins drifted")
    for uses in test_uses + link_uses:
        if FULL_SHA.fullmatch(uses.rsplit("@", 1)[1]) is None:
            raise SystemExit("issue-link workflow has a non-SHA action pin")
    privileged_checkout = link_job.get("steps", [])[0].get("with", {})
    if (
        privileged_checkout.get("ref")
        != "${{ github.event.repository.default_branch }}"
        or privileged_checkout.get("persist-credentials") is not False
    ):
        raise SystemExit("privileged issue-link job checks out untrusted code")
    github_script = link_job.get("steps", [])[1].get("with", {}).get("script", "")
    for contract_fragment in (
        "hasOpenPeerLink",
        "context.payload.changes?.body?.from",
        "await github.rest.pulls.get",
        "github.paginate(github.rest.pulls.list",
    ):
        if contract_fragment not in github_script:
            raise SystemExit(
                "issue-link reconciliation lost contract: " + contract_fragment
            )


def main() -> None:
    check_repository_automation()
    unsafe_forms = (
        "${{ inputs.version }}",
        "${{inputs['version']}}",
        '${{ inputs["version"] }}',
        "${{ github.event.inputs.version }}",
        "${{ github.event.inputs['version'] }}",
        "${{ github.event['inputs']['version'] }}",
        "${{ format('{0}', inputs.version) }}",
    )
    if not all(contains_direct_input(form) for form in unsafe_forms):
        raise SystemExit("release workflow input scanner does not cover every expression form")
    if contains_direct_input("${{ matrix.target }}"):
        raise SystemExit("release workflow input scanner rejected an unrelated expression")

    unsafe_steps: list[str] = []
    documents: dict[str, dict] = {}
    for workflow in WORKFLOWS:
        document = yaml.safe_load(workflow.read_text(encoding="utf-8"))
        documents[workflow.name] = document
        for job in document.get("jobs", {}).values():
            for step in job.get("steps", []):
                script = step.get("run")
                if isinstance(script, str) and contains_direct_input(script):
                    unsafe_steps.append(
                        f"{workflow.name}: {step.get('name', 'unnamed step')}"
                    )

    if unsafe_steps:
        raise SystemExit(
            "release workflows interpolate the inputs context directly into run blocks: "
            + ", ".join(unsafe_steps)
        )

    release_cli_jobs = documents["release-cli.yml"].get("jobs", {})
    validator = release_cli_jobs.get("validate_release_tag")
    if not isinstance(validator, dict):
        raise SystemExit("release-cli workflow has no validate_release_tag prerequisite")
    dashboard_needs = release_cli_jobs.get("build_dashboard", {}).get("needs", [])
    if isinstance(dashboard_needs, str):
        dashboard_needs = [dashboard_needs]
    if "validate_release_tag" not in dashboard_needs:
        raise SystemExit("release-cli build_dashboard bypasses release tag validation")

    format_step = next(
        (
            step
            for step in validator.get("steps", [])
            if step.get("name") == "Validate release tag format"
        ),
        None,
    )
    if not isinstance(format_step, dict) or not isinstance(format_step.get("run"), str):
        raise SystemExit("release-cli workflow has no executable release tag format gate")
    format_script = format_step["run"]
    for tag, expected_success in (
        ("v2026.8.9", True),
        ("v2026.8.9-rc.1", True),
        ("--help", False),
        ("--repo=attacker/repo", False),
        ("$(id)", False),
    ):
        environment = os.environ.copy()
        environment["RELEASE_TAG"] = tag
        result = subprocess.run(
            ["bash", "-eu", "-o", "pipefail", "-c", format_script],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        if (result.returncode == 0) != expected_success:
            raise SystemExit(f"release tag format gate misclassified {tag!r}")

    verify_step = next(
        (
            step
            for step in validator.get("steps", [])
            if step.get("name") == "Verify release exists"
        ),
        None,
    )
    verify_script = verify_step.get("run") if isinstance(verify_step, dict) else None
    verify_env = verify_step.get("env", {}) if isinstance(verify_step, dict) else {}
    if verify_env.get("GH_REPO") != "${{ github.repository }}":
        raise SystemExit("pre-checkout release validation does not set trusted GH_REPO")
    if not isinstance(verify_script, str) or not all(
        fragment in verify_script
        for fragment in (
            'gh release view "$RELEASE_TAG"',
            '[ "$ACTUAL_TAG" != "$RELEASE_TAG" ]',
        )
    ):
        raise SystemExit("release-cli workflow does not verify the exact release tag")

    print("release and repository automation safety checks passed")


if __name__ == "__main__":
    main()
