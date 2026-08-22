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

    devto_workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "devto-publish.yml").read_text(
            encoding="utf-8"
        )
    )
    devto_triggers = devto_workflow.get("on", devto_workflow.get(True, {}))
    expected_devto_pr_paths = {
        "articles/*.md",
        ".github/scripts/publish-devto.rb",
        ".github/scripts/test-publish-devto.rb",
        ".github/workflows/devto-publish.yml",
    }
    if set(devto_triggers.get("pull_request", {}).get("paths", [])) != (
        expected_devto_pr_paths
    ):
        raise SystemExit("Dev.to publisher validation paths drifted")
    if devto_workflow.get("permissions") != {"contents": "read"}:
        raise SystemExit("Dev.to publisher has broader repository permissions than read-only")
    if devto_workflow.get("concurrency") != {
        "group": (
            "devto-publish-${{ github.event_name == 'pull_request' && "
            "github.event.pull_request.number || 'production' }}"
        ),
        "cancel-in-progress": "${{ github.event_name == 'pull_request' }}",
    }:
        raise SystemExit("Dev.to publisher does not isolate PR validation from production")
    devto_jobs = devto_workflow.get("jobs", {})
    validate_job = devto_jobs.get("validate", {})
    publish_job = devto_jobs.get("publish", {})
    if validate_job.get("timeout-minutes") != 5:
        raise SystemExit("Dev.to publisher validation is not bounded to five minutes")
    if (
        publish_job.get("timeout-minutes") != 10
        or publish_job.get("needs") != "validate"
        or publish_job.get("if") != "github.event_name != 'pull_request'"
    ):
        raise SystemExit("Dev.to production publication bypasses its bounded validation gate")
    validate_steps = validate_job.get("steps", [])
    publish_steps = publish_job.get("steps", [])
    expected_checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    for job_name, steps in (("validate", validate_steps), ("publish", publish_steps)):
        checkouts = [step.get("uses") for step in steps if step.get("uses")]
        if checkouts != [expected_checkout]:
            raise SystemExit(f"Dev.to {job_name} checkout action pin drifted")
        if FULL_SHA.fullmatch(expected_checkout.rsplit("@", 1)[1]) is None:
            raise SystemExit(f"Dev.to {job_name} checkout is not pinned to a full SHA")
    if [step.get("run") for step in validate_steps if step.get("run")] != [
        "ruby .github/scripts/test-publish-devto.rb"
    ]:
        raise SystemExit("Dev.to validation does not run the publisher test suite")
    if [step.get("run") for step in publish_steps if step.get("run")] != [
        "ruby .github/scripts/publish-devto.rb articles/*.md"
    ]:
        raise SystemExit("Dev.to production job does not run the reviewed publisher")


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
