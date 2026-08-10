#!/usr/bin/env python3
"""Regression checks for untrusted release workflow inputs."""

import os
import re
import subprocess
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = (
    ROOT / ".github" / "workflows" / "release-tag.yml",
    ROOT / ".github" / "workflows" / "release-cli.yml",
)
INPUTS_TOKEN = re.compile(r"\binputs\b")


def contains_direct_input(script: str) -> bool:
    # Conservatively reject any shell source that contains both a GitHub
    # expression and the inputs context. This covers dot/bracket access,
    # github.event aliases, and wrappers whose string literals contain braces.
    return "${{" in script and INPUTS_TOKEN.search(script) is not None


def main() -> None:
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

    print("release workflows keep the inputs context out of shell source")


if __name__ == "__main__":
    main()
