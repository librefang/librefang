"""Tests for the ingest hook (stdin/stdout interface)."""
import json
import subprocess
import sys
from pathlib import Path

HOOK = Path(__file__).parent.parent / "hooks" / "ingest.py"


def run_hook(payload: dict) -> dict:
    result = subprocess.run(
        [sys.executable, str(HOOK)],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout.strip())


def test_bad_json_returns_empty():
    result = subprocess.run(
        [sys.executable, str(HOOK)],
        input="not json",
        capture_output=True,
        text=True,
    )
    out = json.loads(result.stdout.strip())
    assert out == {"memories": []}


def test_empty_message_returns_empty():
    out = run_hook({"type": "ingest", "agent_id": "a1", "message": ""})
    assert out == {"memories": []}


def test_short_message_returns_empty():
    out = run_hook({"type": "ingest", "agent_id": "a1", "message": "hi"})
    assert out == {"memories": []}


def test_no_mempalace_returns_error_not_crash():
    """Without mempalace installed, ingest returns error field but doesn't crash."""
    out = run_hook({"type": "ingest", "agent_id": "a1", "message": "What are my upcoming meetings?"})
    assert "memories" in out
    assert isinstance(out["memories"], list)
    # error field should be present since mempalace isn't installed in test env
    assert "error" in out


# ---------------------------------------------------------------------------
# Distance filtering (unit-level, no mempalace needed)
# ---------------------------------------------------------------------------

import importlib.util, os

def _load_ingest_module(max_distance="1.2"):
    env_before = os.environ.get("MEMPALACE_MAX_DISTANCE")
    os.environ["MEMPALACE_MAX_DISTANCE"] = max_distance
    spec = importlib.util.spec_from_file_location("ingest", HOOK)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    if env_before is None:
        os.environ.pop("MEMPALACE_MAX_DISTANCE", None)
    else:
        os.environ["MEMPALACE_MAX_DISTANCE"] = env_before
    return mod


def test_truncate_at_word_boundary():
    mod = _load_ingest_module()
    text = "one two three four five six seven"
    result = mod.truncate_at_word(text, 15)
    assert result.endswith("...")
    assert len(result) <= 18


def test_truncate_short_unchanged():
    mod = _load_ingest_module()
    assert mod.truncate_at_word("hello", 100) == "hello"


def test_distance_threshold_filters_results():
    """Simulate the distance filtering logic directly."""
    mod = _load_ingest_module(max_distance="1.0")

    results = [
        {"text": "good match", "source_file": "s1", "wing": "w1", "distance": 0.5},
        {"text": "bad match", "source_file": "s2", "wing": "w1", "distance": 1.5},
        {"text": "no distance", "source_file": "s3", "wing": "w1"},
    ]

    MAX_DISTANCE = mod.MAX_DISTANCE
    memories = []
    for r in results:
        text = r.get("text", "")
        distance = r.get("distance")
        if not text:
            continue
        if distance is not None and MAX_DISTANCE > 0 and distance > MAX_DISTANCE:
            continue
        memories.append(text)

    assert "good match" in memories
    assert "bad match" not in memories
    assert "no distance" in memories  # passthrough when distance is absent


def test_distance_zero_disables_filtering():
    mod = _load_ingest_module(max_distance="0")
    assert mod.MAX_DISTANCE == 0.0
