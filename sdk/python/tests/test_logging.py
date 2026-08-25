"""Tests for the sidecar's stderr-only structured logger."""

import json

from librefang.sidecar import logging as sidecar_log


def test_log_level_filters_debug_but_keeps_warning(monkeypatch, capsys):
    monkeypatch.setenv("LIBREFANG_LOG_LEVEL", "info")

    sidecar_log.debug("hidden")
    sidecar_log.warn("visible")

    lines = capsys.readouterr().err.splitlines()
    assert len(lines) == 1
    record = json.loads(lines[0])
    # `warn` is LibreFang's canonical configured level spelling.
    assert record["level"] == "warn"
    assert record["message"] == "visible"


def test_unknown_log_level_configuration_preserves_debug_default(
    monkeypatch, capsys,
):
    monkeypatch.setenv("LIBREFANG_LOG_LEVEL", "not-a-level")

    sidecar_log.debug("still visible")

    record = json.loads(capsys.readouterr().err)
    assert record["level"] == "debug"


def test_structured_write_failure_uses_raw_stderr_fallback(monkeypatch):
    writes = []

    class _BrokenStderr:
        def write(self, _value):
            raise BrokenPipeError("closed")

        def flush(self):
            raise AssertionError("write should fail first")

    monkeypatch.setattr(sidecar_log.sys, "stderr", _BrokenStderr())
    monkeypatch.setattr(
        sidecar_log.os, "write",
        lambda fd, data: writes.append((fd, data)) or len(data),
    )

    sidecar_log.error("cannot serialize to stream")

    assert writes == [(2, sidecar_log._LOG_FAILURE)]


def test_raw_fallback_failure_never_crashes_adapter(monkeypatch):
    class _BrokenStderr:
        def write(self, _value):
            raise OSError("closed")

    monkeypatch.setattr(sidecar_log.sys, "stderr", _BrokenStderr())
    monkeypatch.setattr(
        sidecar_log.os, "write",
        lambda _fd, _data: (_ for _ in ()).throw(OSError("fd closed")),
    )

    sidecar_log.error("best effort")
