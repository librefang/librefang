"""Tests for the after_turn hook (stdin/stdout interface)."""
import json
import subprocess
import sys
from pathlib import Path

HOOK = Path(__file__).parent.parent / "hooks" / "after_turn.py"


def run_hook(payload: dict) -> dict:
    result = subprocess.run(
        [sys.executable, str(HOOK)],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout.strip())


# ---------------------------------------------------------------------------
# Skip cases
# ---------------------------------------------------------------------------

def test_empty_messages_skipped():
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": []})
    assert out["status"] == "skip"
    assert out["reason"] == "no messages"


def test_bad_json_skipped():
    result = subprocess.run(
        [sys.executable, str(HOOK)],
        input="not json",
        capture_output=True,
        text=True,
    )
    out = json.loads(result.stdout.strip())
    assert out["status"] == "skip"
    assert out["reason"] == "bad input"


def test_short_exchange_skipped():
    messages = [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "hello"},
    ]
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": messages})
    assert out["status"] == "skip"
    assert out["reason"] == "too short"


def test_irrelevant_exchange_skipped():
    messages = [
        {"role": "user", "content": "What is the capital of France and why is it historically significant?"},
        {"role": "assistant", "content": "Paris has been the capital since the 10th century and is the cultural hub."},
    ]
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": messages})
    assert out["status"] == "skip"
    assert out["reason"] == "not relevant"


def test_agent_mempalace_tool_call_skipped():
    messages = [
        {"role": "user", "content": "Save this to my memory please."},
        {"role": "tool", "content": '{"tool": "mcp_mempalace_add_drawer", "result": "ok"}'},
        {"role": "assistant", "content": "Done, I saved it using mcp_mempalace."},
    ]
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": messages})
    assert out["status"] == "skip"
    assert out["reason"] == "agent used mcp_mempalace"


def test_user_mentioning_mempalace_not_skipped():
    """User saying 'mcp_mempalace' in a message should NOT trigger the dedup skip."""
    messages = [
        {"role": "user", "content": "Can you use mcp_mempalace to save my dentist appointment on April 15th?"},
        {"role": "assistant", "content": "I will remember your dentist appointment on April 15th, it's in my calendar."},
    ]
    # Will fail at import (no mempalace installed in test env) → status=error, not skip
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": messages})
    assert out["status"] != "skip" or out.get("reason") != "agent used mcp_mempalace"


# ---------------------------------------------------------------------------
# Room classification
# ---------------------------------------------------------------------------

import importlib.util, types

def _load_hook_module():
    spec = importlib.util.spec_from_file_location("after_turn", HOOK)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_mod = _load_hook_module()


def test_classify_contacts():
    assert _mod._classify_room("Her email is alice@example.com and phone 555-1234") == ("people", "contacts")


def test_classify_calendar():
    assert _mod._classify_room("I have a dentist appointment on Friday") == ("time", "calendar")


def test_classify_finance():
    assert _mod._classify_room("The invoice for $500 is due next week") == ("finance", "transactions")


def test_classify_logistics():
    assert _mod._classify_room("The package shipment arrived today") == ("logistics", "orders")


def test_classify_decisions():
    assert _mod._classify_room("We decided to use Postgres going forward") == ("knowledge", "decisions")


def test_classify_default_fallback():
    assert _mod._classify_room("The sky is blue and the grass is green today outside.") == ("default", "sessions")


