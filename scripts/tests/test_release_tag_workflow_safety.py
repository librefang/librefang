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
DESKTOP_WORKFLOW = ROOT / ".github" / "workflows" / "release-desktop.yml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
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

    desktop_source = DESKTOP_WORKFLOW.read_text(encoding="utf-8")
    desktop = yaml.safe_load(desktop_source)
    desktop_job = desktop.get("jobs", {}).get("desktop", {})
    cask_job = desktop.get("jobs", {}).get("sync_homebrew_cask", {})
    if desktop_job.get("timeout-minutes") != 345:
        raise SystemExit("desktop release builds do not have the expected timeout")
    if desktop_job.get("strategy", {}).get("max-parallel") != 1:
        raise SystemExit("desktop updater manifest writes are not serialized")
    if cask_job.get("timeout-minutes") != 15:
        raise SystemExit("desktop cask sync does not have the expected timeout")
    if "*latest.json" in desktop_source:
        raise SystemExit("desktop release rebuilds delete the shared updater manifest")
    if desktop_source.count("uploadUpdaterJson: true") != 2:
        raise SystemExit("desktop release builds do not upload updater manifests")
    if desktop_source.count("retryAttempts: 3") != 2:
        raise SystemExit("desktop updater manifest uploads do not retry conflicts")
    for required in (
        'if [ "$LISTED" != true ]',
        'gh release delete-asset "$TAG" "$name" --yes',
        "Clean up macOS signing keychain",
        'security delete-keychain "$KEYCHAIN_PATH"',
        'TAURI_VERSION" != "$X86_TAURI_VERSION',
        "--connect-timeout 10 --max-time 60 --retry 3",
        'git rebase "origin/$BRANCH"',
    ):
        if required not in desktop_source:
            raise SystemExit(f"desktop release hardening is missing: {required}")

    openrouter_workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "update-openrouter-models.yml").read_text(
            encoding="utf-8"
        )
    )
    if openrouter_workflow.get("permissions") != {"contents": "read"}:
        raise SystemExit("OpenRouter updater has broader default permissions than read-only")
    if openrouter_workflow.get("concurrency", {}).get("cancel-in-progress") is not False:
        raise SystemExit("OpenRouter updater can cancel a run after it mutates PR state")
    openrouter_job = openrouter_workflow.get("jobs", {}).get("update", {})
    if openrouter_job.get("timeout-minutes") != 10:
        raise SystemExit("OpenRouter updater does not retain its bounded runtime")
    openrouter_steps = openrouter_job.get("steps", [])
    for step in openrouter_steps:
        uses = step.get("uses")
        if isinstance(uses, str) and (
            "@" not in uses
            or FULL_SHA.fullmatch(uses.rsplit("@", 1)[1]) is None
        ):
            raise SystemExit(f"OpenRouter updater has a non-SHA action pin: {uses}")

    download_step = next(
        (
            step
            for step in openrouter_steps
            if step.get("name") == "Download deterministic snapshot"
        ),
        None,
    )
    download_script = download_step.get("run") if isinstance(download_step, dict) else None
    if not isinstance(download_script, str) or not all(
        fragment in download_script
        for fragment in (
            'set -euo pipefail',
            'mktemp "${snapshot_path}.XXXXXX"',
            'mv -- "$snapshot" "$snapshot_path"',
        )
    ):
        raise SystemExit("OpenRouter download does not use fail-fast same-directory replacement")

    valid_model = {
        "data": [
            {
                "id": "acme/model",
                "name": "Acme Model",
                "context_length": 32768,
                "architecture": {"input_modalities": ["text"]},
                "supported_parameters": ["tools"],
                "top_provider": {"max_completion_tokens": 4096},
                "pricing": {"prompt": "0.1", "completion": "0.2"},
                "ignored": "not projected",
            }
        ]
    }
    invalid_catalogs = (
        {"data": []},
        {
            "data": [
                {"id": "duplicate", "name": "First", "context_length": 4096},
                {"id": "duplicate", "name": "Second", "context_length": 8192},
            ]
        },
        {"data": [{"id": " padded", "name": "Padded", "context_length": 4096}]},
        {"data": [{"id": "zero", "name": "Zero", "context_length": 0}]},
        {"data": [{"id": "float", "name": "Float", "context_length": 1.5}]},
    )
    with tempfile.TemporaryDirectory() as temp_dir:
        temp_root = Path(temp_dir)
        fake_bin = temp_root / "bin"
        fake_bin.mkdir()
        fake_curl = fake_bin / "curl"
        fake_curl.write_text(
            "#!/bin/sh\n"
            "if [ \"${FAKE_CURL_FAIL:-0}\" = 1 ]; then exit 22; fi\n"
            "printf '%s' \"$FAKE_OPENROUTER_BODY\"\n"
        )
        fake_curl.chmod(0o755)
        snapshot = (
            temp_root
            / "crates"
            / "librefang-runtime"
            / "openrouter-models.snapshot.json"
        )
        snapshot.parent.mkdir(parents=True)
        original_snapshot = b'{"data":[{"id":"preserve-me"}]}\n'
        environment = os.environ.copy()
        environment["PATH"] = f"{fake_bin}{os.pathsep}{environment['PATH']}"

        def run_download(body: dict, *, curl_fails: bool = False) -> subprocess.CompletedProcess:
            snapshot.write_bytes(original_snapshot)
            environment["FAKE_OPENROUTER_BODY"] = json.dumps(body)
            environment["FAKE_CURL_FAIL"] = "1" if curl_fails else "0"
            return subprocess.run(
                ["bash", "-c", download_script],
                cwd=temp_root,
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )

        valid_result = run_download(valid_model)
        if valid_result.returncode != 0:
            raise SystemExit(
                "OpenRouter updater rejected a valid catalog: " + valid_result.stderr
            )
        projected = json.loads(snapshot.read_text(encoding="utf-8"))
        if projected["data"][0].get("ignored") is not None:
            raise SystemExit("OpenRouter updater retained an unreviewed API field")

        for invalid_catalog in invalid_catalogs:
            invalid_result = run_download(invalid_catalog)
            if invalid_result.returncode == 0 or snapshot.read_bytes() != original_snapshot:
                raise SystemExit(
                    "OpenRouter updater replaced the snapshot with an invalid catalog"
                )
        curl_result = run_download(valid_model, curl_fails=True)
        if curl_result.returncode == 0 or snapshot.read_bytes() != original_snapshot:
            raise SystemExit("OpenRouter updater replaced the snapshot after curl failed")
        if list(snapshot.parent.glob("openrouter-models.snapshot.json.*")):
            raise SystemExit("OpenRouter updater leaked a temporary snapshot")

    auto_merge_step = next(
        (step for step in openrouter_steps if step.get("name") == "Enable auto-merge"),
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
        raise SystemExit("OpenRouter auto-merge does not use the bounded trusted PR number")

    pr_title = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "pr-title.yml").read_text(
            encoding="utf-8"
        )
    )
    pr_title_triggers = pr_title.get("on", pr_title.get(True, {}))
    if pr_title_triggers.get("pull_request", {}).get("types") != [
        "opened",
        "edited",
        "synchronize",
        "reopened",
    ]:
        raise SystemExit("PR-title validation trigger contract drifted")
    if pr_title.get("permissions") != {"pull-requests": "read"}:
        raise SystemExit("PR-title validation has broader permissions than read-only")
    if pr_title.get("concurrency") != {
        "group": "pr-title-${{ github.event.pull_request.number }}",
        "cancel-in-progress": True,
    }:
        raise SystemExit("PR-title validation does not cancel superseded runs per PR")
    title_job = pr_title.get("jobs", {}).get("check-title", {})
    if title_job.get("timeout-minutes") != 5:
        raise SystemExit("PR-title validation no longer has a five-minute bound")
    title_steps = title_job.get("steps", [])
    if len(title_steps) != 1:
        raise SystemExit("PR-title validation has an unexpected step inventory")
    title_step = title_steps[0]
    expected_title_action = (
        "amannn/action-semantic-pull-request@"
        "48f256284bd46cdaab1048c3721360e808335d50"
    )
    if title_step.get("uses") != expected_title_action:
        raise SystemExit("PR-title validation action pin drifted")
    if FULL_SHA.fullmatch(expected_title_action.rsplit("@", 1)[1]) is None:
        raise SystemExit("PR-title validation action is not pinned to a full SHA")
    title_inputs = title_step.get("with", {})
    if title_inputs.get("subjectPattern") != "^.+$":
        raise SystemExit("PR-title validation no longer accepts case-neutral subjects")
    expected_title_types = {
        "feat",
        "fix",
        "docs",
        "style",
        "refactor",
        "perf",
        "test",
        "build",
        "ci",
        "chore",
        "revert",
        "release",
    }
    actual_title_types = set(str(title_inputs.get("types", "")).split())
    if actual_title_types != expected_title_types:
        raise SystemExit("PR-title validation conventional types drifted")

    label_sync = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "label-sync.yml").read_text(
            encoding="utf-8"
        )
    )
    if label_sync.get("concurrency") != {
        "group": "label-sync",
        "cancel-in-progress": False,
    }:
        raise SystemExit("label reconciliation does not serialize complete runs")
    label_sync_job = label_sync.get("jobs", {}).get("sync", {})
    if label_sync_job.get("timeout-minutes") != 5:
        raise SystemExit("label reconciliation no longer has a five-minute bound")
    if label_sync_job.get("permissions") != {
        "contents": "read",
        "issues": "write",
    }:
        raise SystemExit("label reconciliation lacks minimal checkout/write permissions")
    expected_label_sync_actions = {
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "EndBug/label-sync@52074158190acb45f3077f9099fea818aa43f97a",
    }
    actual_label_sync_actions = {
        step.get("uses")
        for step in label_sync_job.get("steps", [])
        if isinstance(step.get("uses"), str)
    }
    if actual_label_sync_actions != expected_label_sync_actions:
        raise SystemExit("label reconciliation action pins drifted")
    if any(
        FULL_SHA.fullmatch(uses.rsplit("@", 1)[1]) is None
        for uses in actual_label_sync_actions
    ):
        raise SystemExit("label reconciliation has a non-SHA action pin")

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

    contributor_role = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "contributor-role.yml").read_text(
            encoding="utf-8"
        )
    )
    contributor_triggers = contributor_role.get("on", contributor_role.get(True, {}))
    if set(contributor_triggers.get("pull_request", {}).get("paths", [])) != {
        ".github/scripts/contributor-announcement.sh",
        ".github/scripts/tests/contributor-announcement.test.sh",
        ".github/workflows/contributor-role.yml",
    }:
        raise SystemExit("contributor announcement validation paths drifted")
    if contributor_triggers.get("pull_request_target", {}).get("types") != ["closed"]:
        raise SystemExit("contributor announcements run on unsupported privileged events")
    if contributor_role.get("permissions") != {
        "contents": "read",
        "pull-requests": "read",
    }:
        raise SystemExit("contributor announcements have broader than read-only permissions")
    contributor_jobs = contributor_role.get("jobs", {})
    validate_job = contributor_jobs.get("validate", {})
    announce_job = contributor_jobs.get("assign-role", {})
    if (
        validate_job.get("if") != "github.event_name == 'pull_request'"
        or validate_job.get("timeout-minutes") != 5
    ):
        raise SystemExit("contributor announcement PR validation gate drifted")
    if announce_job.get("if") != (
        "github.event_name == 'pull_request_target' && "
        "github.event.pull_request.merged == true"
    ) or announce_job.get("timeout-minutes") != 5:
        raise SystemExit("contributor announcement privileged gate drifted")
    expected_checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    for job_name, job in (("validate", validate_job), ("announce", announce_job)):
        uses = [step.get("uses") for step in job.get("steps", []) if step.get("uses")]
        if uses != [expected_checkout]:
            raise SystemExit(f"contributor {job_name} checkout pin drifted")
        if FULL_SHA.fullmatch(expected_checkout.rsplit("@", 1)[1]) is None:
            raise SystemExit(f"contributor {job_name} checkout is not a full SHA")
    announce_checkout = announce_job.get("steps", [])[0].get("with", {})
    if (
        announce_checkout.get("ref") != "${{ github.event.repository.default_branch }}"
        or announce_checkout.get("persist-credentials") is not False
    ):
        raise SystemExit("privileged contributor announcement checks out untrusted code")
    announce_step = next(
        (
            step
            for step in announce_job.get("steps", [])
            if step.get("name") == "Announce in Discord"
        ),
        None,
    )
    announce_script = announce_step.get("run") if isinstance(announce_step, dict) else None
    if (
        not isinstance(announce_script, str)
        or announce_script.strip() != "bash .github/scripts/contributor-announcement.sh"
    ):
        raise SystemExit("privileged contributor announcement bypasses its trusted helper")
    if "${{" in announce_script:
        raise SystemExit("contributor announcement interpolates event data into shell code")

    todo_workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "todo-to-issue.yml").read_text(
            encoding="utf-8"
        )
    )
    if todo_workflow.get("permissions") != {
        "contents": "read",
        "issues": "write",
    }:
        raise SystemExit("TODO issue workflow permissions are not minimal and complete")
    # Any concurrency group at all can drop an intermediate push scan, and nothing retriggers it.
    # A group keyed on `github.run_id` does not count as an escape hatch: the id is unique per run,
    # so the block reads as a policy while implementing none.
    if "concurrency" in todo_workflow:
        raise SystemExit("TODO issue workflow can discard an incremental push scan")
    scan_job = todo_workflow.get("jobs", {}).get("scan", {})
    if scan_job.get("timeout-minutes") != 10:
        raise SystemExit("TODO issue workflow has no bounded runtime")
    checkout = next(
        (
            step
            for step in scan_job.get("steps", [])
            if step.get("uses", "").startswith("actions/checkout@")
        ),
        {},
    )
    if checkout.get("with", {}).get("persist-credentials") is not False:
        raise SystemExit("TODO issue checkout persists credentials")
    todo_action = next(
        (
            step
            for step in scan_job.get("steps", [])
            if step.get("uses", "").startswith("alstr/todo-to-issue-action@")
        ),
        {},
    )
    identifiers = todo_action.get("with", {}).get("IDENTIFIERS")
    try:
        parsed_identifiers = json.loads(identifiers)
    except (TypeError, json.JSONDecodeError) as error:
        raise SystemExit("TODO issue identifiers are not valid JSON") from error
    if parsed_identifiers != [
        {"name": "TODO", "labels": ["good first issue", "help wanted"]}
    ]:
        raise SystemExit("TODO issue labels do not match the supported action contract")

    welcome_workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "welcome.yml").read_text(encoding="utf-8")
    )
    if welcome_workflow.get("permissions") != {
        "issues": "write",
        "pull-requests": "read",
    }:
        raise SystemExit("welcome workflow permissions are not minimal")
    welcome_job = welcome_workflow.get("jobs", {}).get("welcome", {})
    if welcome_job.get("timeout-minutes") != 5:
        raise SystemExit("welcome workflow has no bounded runtime")
    welcome_step = next(
        (
            step
            for step in welcome_job.get("steps", [])
            if step.get("name") == "Welcome first-time contributors"
        ),
        {},
    )
    welcome_script = welcome_step.get("run", "")
    if not isinstance(welcome_script, str) or "${{" in welcome_script:
        raise SystemExit("welcome shell source interpolates GitHub event expressions")
    if (
        welcome_script.count('select(.number != $current)') != 2
        or welcome_script.count('--argjson current "$NUMBER"') != 2
    ):
        raise SystemExit("welcome workflow does not exclude the event item from history")
    if "cargo test --workspace" in welcome_script or (
        "cargo test -p <affected-crate>" not in welcome_script
    ):
        raise SystemExit("welcome workflow recommends a forbidden workspace-wide test")
    expected_event_env = {
        "REPO": "${{ github.repository }}",
        "AUTHOR": "${{ github.event.sender.login }}",
        "EVENT_NAME": "${{ github.event_name }}",
        "ISSUE_NUMBER": "${{ github.event.issue.number || '' }}",
        "PR_NUMBER": "${{ github.event.pull_request.number || '' }}",
    }
    welcome_env = welcome_step.get("env", {})
    if any(welcome_env.get(name) != value for name, value in expected_event_env.items()):
        raise SystemExit("welcome workflow does not pass event metadata through env")
    shell_check = subprocess.run(
        ["bash", "-n"],
        input=welcome_script,
        capture_output=True,
        text=True,
        check=False,
    )
    if shell_check.returncode != 0:
        raise SystemExit("welcome workflow shell is invalid: " + shell_check.stderr)

    dashboard_build = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "dashboard-build.yml").read_text(
            encoding="utf-8"
        )
    )
    if dashboard_build.get("permissions") != {"contents": "read"}:
        raise SystemExit("dashboard workflow does not default to read-only contents")
    dashboard_jobs = dashboard_build.get("jobs", {})
    build_job = dashboard_jobs.get("build", {})
    upload_job = dashboard_jobs.get("upload", {})
    if "permissions" in build_job:
        raise SystemExit("dashboard build job overrides its read-only token")
    if (
        upload_job.get("if") != "github.event_name != 'pull_request'"
        or upload_job.get("needs") != "build"
        or upload_job.get("permissions") != {"contents": "write"}
        or upload_job.get("timeout-minutes") != 10
    ):
        raise SystemExit("dashboard release writes are not isolated behind build")
    expected_transfer_actions = {
        "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
    }
    actual_transfer_actions = {
        step.get("uses")
        for job in (build_job, upload_job)
        for step in job.get("steps", [])
        if str(step.get("uses", "")).startswith(
            ("actions/upload-artifact@", "actions/download-artifact@")
        )
    }
    if actual_transfer_actions != expected_transfer_actions:
        raise SystemExit("dashboard release artifact action pins drifted")
    upload_checkout = upload_job.get("steps", [])[0].get("with", {})
    if (
        upload_checkout.get("ref")
        != "${{ github.event.repository.default_branch }}"
        or upload_checkout.get("persist-credentials") is not False
    ):
        raise SystemExit("dashboard release upload executes an untrusted helper")
    upload_step = next(
        (
            step
            for step in upload_job.get("steps", [])
            if step.get("name") == "Upload to release"
        ),
        {},
    )
    upload_script = upload_step.get("run", "")
    if (
        'gh release upload --repo "$REPOSITORY" --clobber --' not in upload_script
        or '"$TAG" /tmp/dashboard-dist.tar.gz' not in upload_script
    ):
        raise SystemExit("dashboard release tag is not separated from gh options")

    auto_update = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "auto-update-branches.yml").read_text(
            encoding="utf-8"
        )
    )
    if auto_update.get("permissions") != {"contents": "read"}:
        raise SystemExit("branch reconciliation does not default to read-only")
    update_job = auto_update.get("jobs", {}).get("update-branches", {})
    if "permissions" in update_job:
        raise SystemExit("branch reconciliation grants the default token write access")
    if (
        update_job.get("if") != "github.event_name != 'pull_request'"
        or update_job.get("needs") != "test-helper"
    ):
        raise SystemExit("branch reconciliation mutations are not gated behind tests")
    update_steps = update_job.get("steps", [])
    privileged_checkout = update_steps[0].get("with", {}) if update_steps else {}
    if (
        privileged_checkout.get("ref")
        != "${{ github.event.repository.default_branch }}"
        or privileged_checkout.get("persist-credentials") is not False
    ):
        raise SystemExit("branch reconciliation executes an untrusted helper")
    github_script = next(
        (
            step
            for step in update_steps
            if str(step.get("uses", "")).startswith("actions/github-script@")
        ),
        {},
    )
    if github_script.get("with", {}).get("github-token") != (
        "${{ secrets.WEBSITE_REPO_TOKEN }}"
    ):
        raise SystemExit("branch reconciliation does not use its explicit PAT")
    reconciliation_script = github_script.get("with", {}).get("script", "")
    if "expected_head_sha: ref" not in reconciliation_script:
        raise SystemExit("branch reconciliation has no head-movement guard")

    dependabot = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "auto-merge-dependabot.yml").read_text(
            encoding="utf-8"
        )
    )
    if dependabot.get("permissions") != {"contents": "read"}:
        raise SystemExit("Dependabot auto-merge does not default to read-only")
    dependabot_jobs = dependabot.get("jobs", {})
    merge_job = dependabot_jobs.get("auto-merge", {})
    if merge_job.get("needs") != "test-selector":
        raise SystemExit("Dependabot auto-merge does not require its helper tests")
    if merge_job.get("permissions") != {
        "actions": "read",
        "contents": "read",
        "pull-requests": "write",
    }:
        raise SystemExit("Dependabot auto-merge permissions drifted")
    merge_steps = merge_job.get("steps", [])
    privileged_checkout = merge_steps[0].get("with", {}) if merge_steps else {}
    if (
        privileged_checkout.get("ref")
        != "${{ github.event.repository.default_branch }}"
        or privileged_checkout.get("persist-credentials") is not False
    ):
        raise SystemExit("Dependabot auto-merge executes an untrusted helper")
    classify_step = next(
        (
            step
            for step in merge_steps
            if step.get("name") == "Classify update-type from PR title"
        ),
        {},
    )
    if "classify-dependabot-title.sh" not in classify_step.get("run", ""):
        raise SystemExit("Dependabot auto-merge bypasses its tested classifier")
    merge_step = next(
        (
            step
            for step in merge_steps
            if step.get("name") == "Enable auto-merge for safe Dependabot bumps"
        ),
        {},
    )
    if "--match-head-commit \"$expected_head\"" not in merge_step.get("run", ""):
        raise SystemExit("Dependabot auto-merge has no final head guard")

    auto_close = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "auto-close-resolved-issues.yml").read_text(
            encoding="utf-8"
        )
    )
    reconcile = auto_close.get("jobs", {}).get("reconcile", {})
    if (
        auto_close.get("permissions") != {"contents": "read"}
        or reconcile.get("needs") != "test-helper"
        or reconcile.get("if") != "github.event_name != 'pull_request'"
        or reconcile.get("permissions")
        != {"contents": "read", "issues": "write"}
    ):
        raise SystemExit("resolved-issue mutations are not isolated behind tests")
    reconcile_steps = reconcile.get("steps", [])
    checkout = reconcile_steps[0].get("with", {}) if reconcile_steps else {}
    if (
        checkout.get("ref") != "${{ github.event.repository.default_branch }}"
        or checkout.get("persist-credentials") is not False
        or checkout.get("fetch-depth") != 0
    ):
        raise SystemExit("resolved-issue reconciliation checks out untrusted history")
    reconcile_script = next(
        (
            step.get("with", {}).get("script", "")
            for step in reconcile_steps
            if str(step.get("uses", "")).startswith("actions/github-script@")
        ),
        "",
    )
    for contract in (
        "execFileSync",
        "--format=%H%x00%s%x00%B",
        "sanitizeInlineCode",
    ):
        if contract not in reconcile_script:
            raise SystemExit("resolved-issue reconciliation lost contract: " + contract)

    pr_labels = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "pr-labels.yml").read_text(
            encoding="utf-8"
        )
    )
    pr_label_triggers = pr_labels.get("on", pr_labels.get(True, {}))
    target_trigger = pr_label_triggers.get("pull_request_target", {})
    if target_trigger.get("types") != ["opened", "reopened", "synchronize"]:
        raise SystemExit("PR area labels schedule unsupported or no-op event types")
    if pr_labels.get("concurrency") != {
        "group": "pr-labels-${{ github.event.pull_request.number }}",
        "cancel-in-progress": True,
    }:
        raise SystemExit("PR area labels do not cancel superseded runs per PR")
    if pr_labels.get("permissions") != {
        "contents": "read",
        "pull-requests": "write",
    }:
        raise SystemExit("PR area labels have incorrect token permissions")
    area_job = pr_labels.get("jobs", {}).get("area", {})
    if "if" in area_job:
        raise SystemExit("PR area labels retain a redundant event condition")
    if area_job.get("timeout-minutes") != 5:
        raise SystemExit("PR area labels no longer have a five-minute bound")
    area_steps = area_job.get("steps", [])
    expected_labeler = (
        "actions/labeler@bf12e9b00b37c5c0ca2b87b79b2daf7891dbda13"
    )
    if [step.get("uses") for step in area_steps] != [expected_labeler]:
        raise SystemExit("PR area labels do not use the reviewed labeler action pin")
    if FULL_SHA.fullmatch(expected_labeler.rsplit("@", 1)[1]) is None:
        raise SystemExit("PR area labeler action is not pinned to a full SHA")

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
        "collectReconciliationState",
        "await github.rest.pulls.get",
        "github.paginate(github.rest.pulls.list",
        "github.paginate(github.rest.issues.listForRepo",
    ):
        if contract_fragment not in github_script:
            raise SystemExit(
                "issue-link reconciliation lost contract: " + contract_fragment
            )

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

    # A published GitHub Release is what every user and mirror reads, so it must not exist until
    # the artifacts do. release.yml used to create it published before a single job had run, which
    # is how v2026.8.30 stayed permanently public with 23 of 48 assets and no SHA256SUMS — and why
    # `sign_release_artifacts` refusing to sign changed nothing that anyone could observe.
    release_document = yaml.safe_load(RELEASE_WORKFLOW.read_text(encoding="utf-8"))
    release_jobs = release_document.get("jobs", {})
    create_steps = release_jobs.get("create_release", {}).get("steps", [])
    create_script = "".join(
        step.get("run", "") for step in create_steps if isinstance(step, dict)
    )
    if "gh release create" not in create_script:
        raise SystemExit("release.yml no longer creates the release")
    for command in ("gh release create", "gh release edit"):
        for line in create_script.splitlines():
            if command in line and "--draft" not in line:
                raise SystemExit(f"release.yml runs `{command}` without --draft")

    publish_job = release_jobs.get("publish_release")
    if publish_job is None:
        raise SystemExit("release.yml has no publish_release job to leave draft state")
    publish_needs = publish_job.get("needs") or []
    if isinstance(publish_needs, str):
        publish_needs = [publish_needs]
    # The gate is the signature plus the desktop bundles. `sign_release_artifacts` already waits on
    # every CLI job and rejects an incomplete platform set, so requiring it requires them too.
    for required in ("sign_release_artifacts", "desktop"):
        if required not in publish_needs:
            raise SystemExit(f"publish_release does not wait for {required}")
    # Best-effort jobs must never hold a finished release back.
    for optional in ("mobile_ios", "mobile_android"):
        if optional in publish_needs:
            raise SystemExit(f"publish_release gates on best-effort job {optional}")
    publish_script = "".join(
        step.get("run", "") for step in publish_job.get("steps", []) if isinstance(step, dict)
    )
    if "--draft=false" not in publish_script:
        raise SystemExit("publish_release never leaves draft state")

    # The two formulae that bake a public releases/download URL cannot run while it 404s.
    for consumer in ("sync_homebrew", "sync_homebrew_cask"):
        consumer_needs = release_jobs.get(consumer, {}).get("needs") or []
        if isinstance(consumer_needs, str):
            consumer_needs = [consumer_needs]
        if "publish_release" not in consumer_needs:
            raise SystemExit(f"{consumer} writes a download URL before the release is published")

    sign_job = release_cli_jobs.get("sign_release_artifacts", {})
    sign_steps = sign_job.get("steps", [])
    manifest_step = next(
        (
            step
            for step in sign_steps
            if step.get("name") == "Build SHA256SUMS manifest from release assets"
        ),
        None,
    )
    manifest_script = (
        manifest_step.get("run") if isinstance(manifest_step, dict) else None
    )
    sign_env = sign_job.get("env", {})
    expected_platforms = sign_env.get("EXPECTED_CHECKSUM_ASSETS")
    allowed_symbols = sign_env.get("ALLOWED_SYMBOL_CHECKSUM_ASSETS")
    if not all(
        isinstance(value, str)
        for value in (manifest_script, expected_platforms, allowed_symbols)
    ):
        raise SystemExit("release-cli manifest builder is missing its asset contract")

    def run_manifest_builder(assets: list[str]) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            fake_bin = temp_root / "bin"
            fake_bin.mkdir()
            fake_gh = fake_bin / "gh"
            fake_gh.write_text(
                "#!/bin/sh\n"
                "if [ \"$1 $2\" = \"release view\" ]; then\n"
                "  printf '%s\\n' \"$FAKE_ASSETS\"\n"
                "  exit 0\n"
                "fi\n"
                "if [ \"$1 $2\" = \"release download\" ]; then\n"
                "  while [ \"$#\" -gt 0 ]; do\n"
                "    if [ \"$1\" = --pattern ]; then shift; name=$1; fi\n"
                "    shift\n"
                "  done\n"
                "  printf 'deadbeef  %s\\n' \"${name%.sha256}\" > \"$name\"\n"
                "  exit 0\n"
                "fi\n"
                "exit 1\n",
                encoding="utf-8",
            )
            fake_gh.chmod(0o755)
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}{os.pathsep}{environment['PATH']}",
                    "FAKE_ASSETS": "\n".join(assets),
                    "RELEASE_TAG": "v2026.8.19",
                    "EXPECTED_CHECKSUM_ASSETS": expected_platforms,
                    "ALLOWED_SYMBOL_CHECKSUM_ASSETS": allowed_symbols,
                }
            )
            return subprocess.run(
                ["bash", "-eu", "-o", "pipefail", "-c", manifest_script],
                cwd=temp_root,
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )

    platform_assets = expected_platforms.splitlines()
    symbol_assets = allowed_symbols.splitlines()

    # release.yml gates the same set by count and release-cli.yml by exact name, so the two drift
    # apart the moment a matrix target is added on one side only. Tie them together here: a new
    # target that reaches release.yml's constant without reaching this list, or the reverse, stops
    # the build instead of shipping a manifest that is missing a platform's hash.
    release_text = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    expected_platform_count = re.search(r"^\s*EXPECTED_PLATFORMS=(\d+)$", release_text, re.M)
    if expected_platform_count is None:
        raise SystemExit("release.yml no longer declares EXPECTED_PLATFORMS")
    if len(platform_assets) != int(expected_platform_count.group(1)):
        raise SystemExit(
            "release-cli EXPECTED_CHECKSUM_ASSETS "
            f"({len(platform_assets)}) does not match release.yml EXPECTED_PLATFORMS "
            f"({expected_platform_count.group(1)})"
        )
    required_symbols = [
        asset for asset in symbol_assets if "apple-darwin" in asset
    ]
    if run_manifest_builder(platform_assets + required_symbols).returncode != 0:
        raise SystemExit("release-cli rejected the required platform and macOS symbol assets")
    if run_manifest_builder(platform_assets + symbol_assets).returncode != 0:
        raise SystemExit("release-cli rejected the complete known symbol asset set")
    if run_manifest_builder(platform_assets + required_symbols[:1]).returncode == 0:
        raise SystemExit("release-cli accepted a missing required macOS symbol asset")
    unexpected_symbol = "librefang-unknown-debug-symbols.tar.gz.sha256"
    if (
        run_manifest_builder(platform_assets + required_symbols + [unexpected_symbol]).returncode
        == 0
    ):
        raise SystemExit("release-cli accepted an unknown debug-symbol asset")

    verify_signature_step = next(
        (
            step
            for step in sign_steps
            if step.get("name") == "Verify signature locally before upload"
        ),
        None,
    )
    verify_signature_script = (
        verify_signature_step.get("run")
        if isinstance(verify_signature_step, dict)
        else None
    )
    if (
        not isinstance(verify_signature_script, str)
        or '--certificate-identity "https://github.com/${GITHUB_WORKFLOW_REF}"'
        not in verify_signature_script
    ):
        raise SystemExit("release-cli does not verify the exact signing workflow identity")

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
