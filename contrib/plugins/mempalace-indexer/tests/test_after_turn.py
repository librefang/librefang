"""Tests for the after_turn hook (stdin/stdout interface)."""
import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

HOOK = Path(__file__).parent.parent / "hooks" / "after_turn.py"


def run_hook(payload: dict, env=None) -> dict:
    import os
    run_env = os.environ.copy()
    if env:
        run_env.update(env)
    result = subprocess.run(
        [sys.executable, str(HOOK)],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        env=run_env,
    )
    return json.loads(result.stdout.strip())


def _load_module(env_overrides=None):
    import os
    saved = {}
    if env_overrides:
        for k, v in env_overrides.items():
            saved[k] = os.environ.get(k)
            os.environ[k] = v
    spec = importlib.util.spec_from_file_location("after_turn", HOOK)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    for k, v in saved.items():
        if v is None:
            os.environ.pop(k, None)
        else:
            os.environ[k] = v
    return mod


_mod = _load_module()


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
    """User saying 'mcp_mempalace' should NOT trigger the dedup skip."""
    messages = [
        {"role": "user", "content": "Can you use mcp_mempalace to save my dentist appointment on April 15th?"},
        {"role": "assistant", "content": "I will remember your dentist appointment on April 15th."},
    ]
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": messages})
    assert out.get("reason") != "agent used mcp_mempalace"


# ---------------------------------------------------------------------------
# Code block stripping
# ---------------------------------------------------------------------------

def test_code_block_stripped_not_whole_message_skipped():
    """A message with a code block but meaningful surrounding text should not be skipped."""
    text = "My email is alice@example.com. Here is the script:\n```bash\necho hello\n```\nRun it daily."
    result = _mod._strip_code_blocks(text)
    assert "alice@example.com" in result
    assert "```" not in result
    assert "[code]" in result


def test_pure_code_block_skipped_after_strip():
    """A message that is only a code block becomes empty after stripping → too short → skip."""
    messages = [
        {"role": "user", "content": "here's the script"},
        {"role": "assistant", "content": "```python\nfor i in range(10):\n    print(i)\n```"},
    ]
    out = run_hook({"type": "after_turn", "agent_id": "a1", "messages": messages})
    # Should be skipped due to length or irrelevance after stripping
    assert out["status"] == "skip"


# ---------------------------------------------------------------------------
# Multi-room classification
# ---------------------------------------------------------------------------

def test_classify_single_room():
    assert _mod._classify_rooms("Her email is alice@example.com") == [("people", "contacts")]


def test_classify_multiple_rooms():
    # "meeting" → calendar, "email" → contacts — both should match
    rooms = _mod._classify_rooms("Schedule a meeting with alice@example.com next Tuesday")
    assert ("people", "contacts") in rooms
    assert ("time", "calendar") in rooms


def test_classify_finance():
    assert _mod._classify_rooms("Invoice for $500 is due next week") == [("finance", "transactions")]


def test_classify_logistics():
    assert _mod._classify_rooms("The shipment arrived today") == [("logistics", "orders")]


def test_classify_decisions():
    assert _mod._classify_rooms("We decided to use Postgres going forward") == [("knowledge", "decisions")]


def test_classify_default_fallback():
    assert _mod._classify_rooms("The sky is blue and the grass is green.") == [("default", "sessions")]


# ---------------------------------------------------------------------------
# Content deduplication
# ---------------------------------------------------------------------------

def test_duplicate_detection():
    with tempfile.TemporaryDirectory() as tmpdir:
        mod = _load_module({"MEMPALACE_PALACE_PATH": tmpdir})
        text = "I have a dentist appointment on April 15th with Dr. Smith."
        assert mod._is_duplicate(text) is False   # first time: not a duplicate
        assert mod._is_duplicate(text) is True    # second time: duplicate


def test_different_content_not_duplicate():
    with tempfile.TemporaryDirectory() as tmpdir:
        mod = _load_module({"MEMPALACE_PALACE_PATH": tmpdir})
        assert mod._is_duplicate("dentist appointment April 15th") is False
        assert mod._is_duplicate("meeting with Bob on Friday afternoon") is False


def test_dedup_rolling_max():
    with tempfile.TemporaryDirectory() as tmpdir:
        mod = _load_module({"MEMPALACE_PALACE_PATH": tmpdir, "MEMPALACE_DEDUP_MAX": "3"})
        # Fill past capacity
        for i in range(5):
            mod._is_duplicate(f"unique content number {i} with enough chars to matter")
        seen = json.loads((Path(tmpdir) / ".after_turn_seen.json").read_text())
        assert len(seen) == 3  # capped at DEDUP_MAX


# ---------------------------------------------------------------------------
# Configurable parameters
# ---------------------------------------------------------------------------

def test_custom_min_chars_env():
    mod = _load_module({"MEMPALACE_MIN_CHARS": "200"})
    assert mod.MIN_CONTENT_LENGTH == 200


def test_custom_window_size_env():
    mod = _load_module({"MEMPALACE_WINDOW_SIZE": "10"})
    assert mod.WINDOW_SIZE == 10
