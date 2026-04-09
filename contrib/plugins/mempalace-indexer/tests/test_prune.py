"""Tests for the prune hook."""
import importlib.util
import json
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

HOOK = Path(__file__).parent.parent / "hooks" / "prune.py"


def _load():
    spec = importlib.util.spec_from_file_location("prune", HOOK)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_mod = _load()


def test_disabled_when_max_age_zero(monkeypatch):
    monkeypatch.setattr(_mod, "MAX_AGE_DAYS", 0)
    result = _mod.prune()
    assert result["status"] == "skip"


def test_error_when_palace_missing(monkeypatch, tmp_path):
    try:
        import chromadb  # noqa: F401
    except ImportError:
        import pytest
        pytest.skip("chromadb not installed")
    monkeypatch.setattr(_mod, "PALACE_PATH", str(tmp_path / "nonexistent"))
    monkeypatch.setattr(_mod, "MAX_AGE_DAYS", 30)
    result = _mod.prune()
    assert result["status"] == "error"
    assert "not found" in result["error"]


def test_parse_filed_at_valid():
    dt = _mod._parse_filed_at("2026-01-01T10:00:00")
    assert dt is not None
    assert dt.year == 2026


def test_parse_filed_at_empty():
    assert _mod._parse_filed_at("") is None


def test_parse_filed_at_invalid():
    assert _mod._parse_filed_at("not-a-date") is None


def test_parse_filed_at_timezone_aware():
    dt = _mod._parse_filed_at("2026-01-01T10:00:00+05:00")
    assert dt.tzinfo is not None


def test_prune_dry_run_with_chromadb(monkeypatch, tmp_path):
    """Integration test: create a real ChromaDB collection and prune old entries."""
    try:
        import chromadb
    except ImportError:
        import pytest
        pytest.skip("chromadb not installed")

    monkeypatch.setattr(_mod, "PALACE_PATH", str(tmp_path))
    monkeypatch.setattr(_mod, "MAX_AGE_DAYS", 30)

    client = chromadb.PersistentClient(path=str(tmp_path))
    col = client.get_or_create_collection("mempalace_drawers")

    old_date = (datetime.now(tz=timezone.utc) - timedelta(days=60)).isoformat()
    new_date = datetime.now(tz=timezone.utc).isoformat()

    col.add(
        ids=["old-1", "new-1"],
        documents=["old memory", "new memory"],
        metadatas=[
            {"filed_at": old_date, "wing": "default", "room": "sessions"},
            {"filed_at": new_date, "wing": "default", "room": "sessions"},
        ],
    )

    result = _mod.prune(dry_run=True)
    assert result["status"] == "dry-run"
    assert result["deleted"] == 1
    assert result["kept"] == 1
    # dry-run: nothing actually deleted
    assert col.count() == 2


def test_prune_actually_deletes(monkeypatch, tmp_path):
    try:
        import chromadb
    except ImportError:
        import pytest
        pytest.skip("chromadb not installed")

    monkeypatch.setattr(_mod, "PALACE_PATH", str(tmp_path))
    monkeypatch.setattr(_mod, "MAX_AGE_DAYS", 30)

    client = chromadb.PersistentClient(path=str(tmp_path))
    col = client.get_or_create_collection("mempalace_drawers")

    old_date = (datetime.now(tz=timezone.utc) - timedelta(days=60)).isoformat()
    col.add(
        ids=["old-2"],
        documents=["stale memory"],
        metadatas=[{"filed_at": old_date}],
    )

    result = _mod.prune(dry_run=False)
    assert result["status"] == "ok"
    assert result["deleted"] == 1
    assert col.count() == 0
