from __future__ import annotations

from librefang import librefang_client as client_module
from librefang.librefang_client import LibreFang


class FakeStreamResponse:
    """Mirrors the shape used by test_generated_client_stream.py, but models
    a real urllib response's EOF signal: a final `read()` returning `b""`
    rather than the iterator running out."""

    def __init__(self, chunks):
        self._chunks = iter(chunks)
        self.closed = False

    def read(self, _size):
        return next(self._chunks, b"")

    def close(self):
        self.closed = True


def test_final_sse_event_without_newline_is_flushed_at_eof(monkeypatch):
    # No trailing "\n" after the last event — mirrors a server that closes
    # the connection immediately after writing its last `data: ` line.
    response = FakeStreamResponse([b'data: {"final":true}'])
    monkeypatch.setattr(client_module, "urlopen", lambda _request: response)

    events = list(LibreFang("http://example.test")._stream("GET", "/events"))

    assert events == [{"final": True}], "final event was dropped at clean EOF"
    assert response.closed


def test_trailing_done_marker_without_newline_stops_the_stream(monkeypatch):
    response = FakeStreamResponse([
        b'data: {"content":"first"}\n\ndata: [DONE]',
    ])
    monkeypatch.setattr(client_module, "urlopen", lambda _request: response)

    events = list(LibreFang("http://example.test")._stream("GET", "/events"))

    assert events == [{"content": "first"}]


def test_non_data_trailing_line_is_ignored(monkeypatch):
    # A trailing SSE comment/keepalive line (no "data: " prefix) at EOF must
    # not be surfaced as an event.
    response = FakeStreamResponse([b": keepalive"])
    monkeypatch.setattr(client_module, "urlopen", lambda _request: response)

    events = list(LibreFang("http://example.test")._stream("GET", "/events"))

    assert events == []
