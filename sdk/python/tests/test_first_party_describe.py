"""First-party adapters expose stable SCHEMA shapes."""
import subprocess
import sys
import json


def _describe(module):
    out = subprocess.check_output(
        [sys.executable, "-m", module, "--describe"],
        stderr=subprocess.PIPE,
    )
    return json.loads(out)


def test_telegram_describe_contract():
    s = _describe("librefang.sidecar.adapters.telegram")
    assert s["name"] == "telegram"
    keys = {f["key"]: f for f in s["fields"]}
    assert keys["TELEGRAM_BOT_TOKEN"]["type"] == "secret"
    assert keys["TELEGRAM_BOT_TOKEN"]["required"] is True
    assert keys["ALLOWED_USERS"]["type"] == "list"
    assert keys["TELEGRAM_CLEAR_DONE_REACTION"]["type"] == "bool"


def test_ntfy_describe_contract():
    s = _describe("librefang.sidecar.adapters.ntfy")
    assert s["name"] == "ntfy"
    keys = {f["key"]: f for f in s["fields"]}
    assert keys["NTFY_TOPIC"]["required"] is True
    assert keys["NTFY_TOKEN"]["type"] == "secret"
