#!/usr/bin/env python3
"""Regression checks for untrusted release-tag workflow inputs."""

import re
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release-tag.yml"
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

    document = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    unsafe_steps: list[str] = []
    for job in document.get("jobs", {}).values():
        for step in job.get("steps", []):
            script = step.get("run")
            if isinstance(script, str) and contains_direct_input(script):
                unsafe_steps.append(step.get("name", "unnamed step"))

    if unsafe_steps:
        raise SystemExit(
            "release-tag workflow interpolates the inputs context directly into run blocks: "
            + ", ".join(unsafe_steps)
        )

    print("release-tag workflow keeps the inputs context out of shell source")


if __name__ == "__main__":
    main()
