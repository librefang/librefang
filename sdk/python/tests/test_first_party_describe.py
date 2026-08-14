"""First-party adapters expose stable SCHEMA shapes."""
import subprocess
import sys
import json

import pytest


DESCRIBE_TIMEOUT_SECS = 10


def _describe(module):
    command = [sys.executable, "-m", module, "--describe"]
    try:
        proc = subprocess.run(
            command,
            capture_output=True,
            timeout=DESCRIBE_TIMEOUT_SECS,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        stderr = (error.stderr or b"").decode("utf-8", "replace")
        raise AssertionError(
            f"{module} --describe timed out after {DESCRIBE_TIMEOUT_SECS}s: "
            f"{stderr}"
        ) from error
    if proc.returncode != 0:
        stderr = proc.stderr.decode("utf-8", "replace")
        raise AssertionError(
            f"{module} --describe failed (rc={proc.returncode}): {stderr}"
        )
    return json.loads(proc.stdout)


def test_describe_failure_surfaces_module_and_stderr(monkeypatch):
    def fail(*_args, **kwargs):
        assert kwargs["timeout"] == DESCRIBE_TIMEOUT_SECS
        return subprocess.CompletedProcess([], 3, b"", b"import traceback")

    monkeypatch.setattr(subprocess, "run", fail)

    with pytest.raises(AssertionError, match="broken.adapter.*import traceback"):
        _describe("broken.adapter")


def test_describe_timeout_surfaces_module_and_stderr(monkeypatch):
    def timeout(*_args, **_kwargs):
        raise subprocess.TimeoutExpired([], DESCRIBE_TIMEOUT_SECS, stderr=b"stuck import")

    monkeypatch.setattr(subprocess, "run", timeout)

    with pytest.raises(AssertionError, match="broken.adapter.*stuck import"):
        _describe("broken.adapter")


def test_telegram_describe_contract():
    s = _describe("librefang.sidecar.adapters.telegram")
    assert s["name"] == "telegram"
    keys = {f["key"]: f for f in s["fields"]}
    assert keys["TELEGRAM_BOT_TOKEN"]["type"] == "secret"
    assert keys["TELEGRAM_BOT_TOKEN"]["required"] is True
    assert keys["ALLOWED_USERS"]["type"] == "list"
    placeholder = keys["ALLOWED_USERS"]["placeholder"]
    assert "empty" in placeholder
    assert "ALL users" in placeholder
    assert "insecure" in placeholder
    assert keys["TELEGRAM_CLEAR_DONE_REACTION"]["type"] == "bool"


def test_ntfy_describe_contract():
    s = _describe("librefang.sidecar.adapters.ntfy")
    assert s["name"] == "ntfy"
    keys = {f["key"]: f for f in s["fields"]}
    assert keys["NTFY_TOPIC"]["required"] is True
    assert keys["NTFY_TOKEN"]["type"] == "secret"
