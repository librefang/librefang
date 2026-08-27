#!/usr/bin/env python3
"""Regression checks for release and repository automation safety."""

import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
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

    contributors_workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "update-contributors.yml").read_text(
            encoding="utf-8"
        )
    )
    if contributors_workflow.get("permissions") != {"contents": "read"}:
        raise SystemExit("contributor updater has broader default permissions than read-only")
    contributors_concurrency = contributors_workflow.get("concurrency", {})
    if (
        contributors_concurrency.get("group") != "${{ github.workflow }}"
        or contributors_concurrency.get("cancel-in-progress") is not False
    ):
        raise SystemExit("contributor updater does not serialize complete mutating runs")
    contributors_job = contributors_workflow.get("jobs", {}).get("update", {})
    if contributors_job.get("timeout-minutes") != 15:
        raise SystemExit("contributor updater does not retain its bounded runtime")
    contributor_steps = contributors_job.get("steps", [])
    expected_contributor_actions = {
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8",
        "Swatinem/rust-cache@a45951ff880207c249adf57334cf2e9bd81d6e1e",
        "peter-evans/create-pull-request@5f6978faf089d4d20b00c7766989d076bb2fc7f1",
    }
    actual_contributor_actions = {
        step["uses"] for step in contributor_steps if isinstance(step.get("uses"), str)
    }
    if actual_contributor_actions != expected_contributor_actions:
        raise SystemExit(
            "contributor updater action pins drifted: "
            f"{sorted(actual_contributor_actions)}"
        )
    if any(
        FULL_SHA.fullmatch(uses.rsplit("@", 1)[1]) is None
        for uses in actual_contributor_actions
    ):
        raise SystemExit("contributor updater has a non-SHA action pin")
    cache_step = next(
        (
            step
            for step in contributor_steps
            if step.get("uses", "").startswith("Swatinem/rust-cache@")
        ),
        None,
    )
    if not isinstance(cache_step, dict) or cache_step.get("with", {}).get(
        "shared-key"
    ) != "contributors":
        raise SystemExit("contributor updater lost its dedicated Rust cache key")

    auto_merge_step = next(
        (step for step in contributor_steps if step.get("name") == "Enable auto-merge"),
        None,
    )
    auto_merge_script = (
        auto_merge_step.get("run") if isinstance(auto_merge_step, dict) else None
    )
    auto_merge_env = (
        auto_merge_step.get("env", {}) if isinstance(auto_merge_step, dict) else {}
    )
    if (
        not isinstance(auto_merge_script, str)
        or "${{" in auto_merge_script
        or 'gh pr merge "$PR_NUMBER"' not in auto_merge_script
        or "for attempt in 1 2 3" not in auto_merge_script
        or auto_merge_env.get("PR_NUMBER")
        != "${{ steps.cpr.outputs.pull-request-number }}"
    ):
        raise SystemExit("contributor auto-merge does not use a bounded trusted PR number")

    with tempfile.TemporaryDirectory() as temp_dir:
        fake_bin = Path(temp_dir)
        fake_gh = fake_bin / "gh"
        fake_gh.write_text(
            "#!/bin/sh\n"
            "count=0\n"
            "if [ -f \"$FAKE_GH_COUNT\" ]; then count=$(cat \"$FAKE_GH_COUNT\"); fi\n"
            "count=$((count + 1))\n"
            "printf '%s' \"$count\" > \"$FAKE_GH_COUNT\"\n"
            "printf '%s\\n' \"$*\" >> \"$FAKE_GH_LOG\"\n"
            "[ \"$count\" -ge \"${FAKE_GH_SUCCEED_AT:-999}\" ]\n"
        )
        fake_gh.chmod(0o755)
        fake_sleep = fake_bin / "sleep"
        fake_sleep.write_text("#!/bin/sh\nexit 0\n")
        fake_sleep.chmod(0o755)
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{fake_bin}{os.pathsep}{environment['PATH']}",
                "GITHUB_REPOSITORY": "librefang/librefang",
                "PR_NUMBER": "123",
                "FAKE_GH_COUNT": str(fake_bin / "count"),
                "FAKE_GH_LOG": str(fake_bin / "calls"),
            }
        )
        environment["FAKE_GH_SUCCEED_AT"] = "3"
        transient = subprocess.run(
            ["bash", "-c", auto_merge_script],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        if transient.returncode != 0 or (fake_bin / "count").read_text() != "3":
            raise SystemExit("contributor auto-merge did not recover on its third attempt")
        expected_call = (
            "pr merge 123 --repo librefang/librefang "
            "--squash --delete-branch --auto"
        )
        calls = (fake_bin / "calls").read_text(encoding="utf-8").splitlines()
        if calls != [expected_call, expected_call, expected_call]:
            raise SystemExit(f"contributor auto-merge invoked gh unexpectedly: {calls}")

        (fake_bin / "count").unlink()
        (fake_bin / "calls").unlink()
        environment["FAKE_GH_SUCCEED_AT"] = "999"
        permanent = subprocess.run(
            ["bash", "-c", auto_merge_script],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        if (
            permanent.returncode == 0
            or (fake_bin / "count").read_text() != "3"
            or "PR #123 remains open" not in permanent.stderr
        ):
            raise SystemExit("contributor auto-merge masks a permanent API failure")

    secrets_documentation = (ROOT / ".github" / "SECRETS.md").read_text(
        encoding="utf-8"
    )
    for required_secret_contract in (
        "`WEBSITE_REPO_TOKEN`",
        "Fine-grained PAT limited to `librefang/librefang`",
        "Contents: read and write",
        "Pull requests: read and write",
    ):
        if required_secret_contract not in secrets_documentation:
            raise SystemExit(
                "WEBSITE_REPO_TOKEN scope is missing from the secret inventory: "
                + required_secret_contract
            )
    if "Fine-grained PAT or GitHub App token" in secrets_documentation:
        raise SystemExit("secret inventory treats an expiring App token as persistent")


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

    release_tag_job = documents["release-tag.yml"].get("jobs", {}).get("tag", {})
    if release_tag_job.get("if") != "github.ref == 'refs/heads/main'":
        raise SystemExit("release-tag job can run from a non-main ref")
    if release_tag_job.get("timeout-minutes") != 10:
        raise SystemExit("release-tag job does not have the expected timeout")
    release_tag_checkout = next(
        (
            step
            for step in release_tag_job.get("steps", [])
            if step.get("uses", "").startswith("actions/checkout@")
        ),
        {},
    )
    if release_tag_checkout.get("with", {}).get("ref") != "main":
        raise SystemExit("release-tag checkout is not pinned to main")
    workspace_step = next(
        (
            step
            for step in release_tag_job.get("steps", [])
            if step.get("name") == "Verify tag matches workspace version"
        ),
        None,
    )
    workspace_script = workspace_step.get("run") if isinstance(workspace_step, dict) else None
    if not isinstance(workspace_script, str) or "tomllib.load" not in workspace_script:
        raise SystemExit("release-tag workflow does not parse workspace version as TOML")
    create_tag_step = next(
        (
            step
            for step in release_tag_job.get("steps", [])
            if step.get("name") == "Create + push tag"
        ),
        None,
    )
    create_tag_script = (
        create_tag_step.get("run") if isinstance(create_tag_step, dict) else None
    )
    if not isinstance(create_tag_script, str) or not all(
        fragment in create_tag_script
        for fragment in (
            "git fetch --no-tags origin refs/heads/main",
            "CHECKED_OUT_SHA=$(git rev-parse HEAD)",
            "CURRENT_MAIN_SHA=$(git rev-parse FETCH_HEAD)",
            'if [ "$CHECKED_OUT_SHA" != "$CURRENT_MAIN_SHA" ]',
            'git push --atomic origin "HEAD:refs/heads/main" "refs/tags/$VERSION"',
        )
    ):
        raise SystemExit("release-tag workflow can tag a stale main checkout")
    manifest_text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    manifest = tomllib.loads(manifest_text)
    current_version = manifest["workspace"]["package"]["version"]
    workspace_environment = os.environ.copy()
    workspace_environment["PATH"] = (
        f"{Path(sys.executable).parent}{os.pathsep}{workspace_environment['PATH']}"
    )

    def run_workspace_gate(candidate_manifest: str, version: str) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as temp_dir:
            Path(temp_dir, "Cargo.toml").write_text(
                candidate_manifest,
                encoding="utf-8",
            )
            return subprocess.run(
                ["bash", "-eu", "-o", "pipefail", "-c", workspace_script],
                cwd=temp_dir,
                env={**workspace_environment, "VERSION": version},
                capture_output=True,
                text=True,
                check=False,
            )

    workspace_result = run_workspace_gate(manifest_text, f"v{current_version}")
    if workspace_result.returncode != 0:
        raise SystemExit(
            "release-tag workspace-version gate rejected the current manifest: "
            + workspace_result.stdout
            + workspace_result.stderr
        )
    for candidate_manifest, version, expected_error in (
        (manifest_text, "v0.0.0", "does not match"),
        ("[workspace.package\n", "v1.0.0", "could not parse"),
        ('[workspace.package]\nversion = ""\n', "v1.0.0", "is empty"),
    ):
        rejected = run_workspace_gate(candidate_manifest, version)
        output = rejected.stdout + rejected.stderr
        if rejected.returncode == 0 or expected_error not in output:
            raise SystemExit(
                f"release-tag workspace-version gate did not reject {expected_error!r} fixture"
            )

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

    for workflow_name in ("mobile-smoke.yml", "release.yml"):
        mobile_document = yaml.safe_load(
            (ROOT / ".github" / "workflows" / workflow_name).read_text(encoding="utf-8")
        )
        if mobile_document.get("env", {}).get("TAURI_CLI_VERSION") != "2.11.4":
            raise SystemExit(f"{workflow_name} does not pin Tauri CLI 2.11.4")
        mobile_steps = [
            step
            for job in mobile_document.get("jobs", {}).values()
            for step in job.get("steps", [])
        ]
        cache_keys = [
            step.get("with", {}).get("key", "")
            for step in mobile_steps
            if step.get("name") == "Cache tauri-cli binary"
        ]
        if len(cache_keys) != 2 or any(
            "env.TAURI_CLI_VERSION" not in key for key in cache_keys
        ):
            raise SystemExit(f"{workflow_name} Tauri CLI caches omit the pinned version")
        install_scripts = [
            step.get("run", "")
            for step in mobile_steps
            if step.get("name") == "Install Tauri CLI"
        ]
        if len(install_scripts) != 2 or any(
            'cargo install tauri-cli --version "$TAURI_CLI_VERSION" --locked'
            not in script
            for script in install_scripts
        ):
            raise SystemExit(f"{workflow_name} does not install the pinned Tauri CLI")
        if any(
            'test "$(cargo tauri --version)" = "tauri-cli $TAURI_CLI_VERSION"'
            not in script
            for script in install_scripts
        ):
            raise SystemExit(f"{workflow_name} does not verify the installed Tauri CLI")
        ios_init = next(
            (
                step.get("run", "")
                for step in mobile_steps
                if step.get("name") == "Initialise iOS project"
            ),
            "",
        )
        if "rm -rf gen/apple\ncargo tauri ios init" not in ios_init:
            raise SystemExit(f"{workflow_name} does not remove iOS placeholders before init")
        ndk_step = next(
            (
                step.get("run", "")
                for step in mobile_steps
                if step.get("name") == "Symlink legacy NDK binutils for openssl-src cross-compile"
            ),
            "",
        )
        if '[ -x "$NDK_BIN/llvm-$tool" ]' not in ndk_step:
            raise SystemExit(f"{workflow_name} does not validate NDK tools before linking")

    print("release and repository automation safety checks passed")


if __name__ == "__main__":
    main()
