from __future__ import annotations

import pytest

from librefang import librefang_client as client_module
from librefang.librefang_client import LibreFang


class FakeStreamResponse:
    def __init__(self, chunks, close_error=None):
        self._chunks = iter(chunks)
        self.closed = False
        self.close_error = close_error

    def read(self, _size):
        chunk = next(self._chunks)
        if isinstance(chunk, BaseException):
            raise chunk
        return chunk

    def close(self):
        self.closed = True
        if self.close_error is not None:
            raise self.close_error


def test_stream_closes_response_when_done_marker_returns(monkeypatch):
    response = FakeStreamResponse([
        b'data: {"content":"hello"}\n\ndata: [DONE]\n\n',
    ])
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)

    events = list(LibreFang("http://example.test")._stream("GET", "/events"))

    assert events == [{"content": "hello"}]
    assert response.closed


def test_stream_joins_multiline_data_fields_at_event_boundary(monkeypatch):
    response = FakeStreamResponse([
        b'data: {"content":\ndata:"hello"}\n\ndata: {"next":true}\n\n',
        b"",
    ])
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)

    events = list(LibreFang("http://example.test")._stream("GET", "/events"))

    assert events == [{"content": "hello"}, {"next": True}]
    assert response.closed


def test_stream_preserves_newlines_in_non_json_multiline_data(monkeypatch):
    response = FakeStreamResponse([b"data: first\ndata: second\n\n", b""])
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)

    events = list(LibreFang("http://example.test")._stream("GET", "/events"))

    assert events == [{"raw": "first\nsecond"}]


def test_stream_closes_response_when_consumer_stops_early(monkeypatch):
    response = FakeStreamResponse([
        b'data: {"content":"first"}\n\ndata: {"content":"second"}\n\n',
    ])
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)
    stream = LibreFang("http://example.test")._stream("GET", "/events")

    assert next(stream) == {"content": "first"}
    stream.close()

    assert response.closed


def test_stream_closes_response_when_read_raises(monkeypatch):
    response = FakeStreamResponse([
        b'data: {"content":"first"}\n\n',
        OSError("socket failed"),
    ])
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)
    stream = LibreFang("http://example.test")._stream("GET", "/events")

    assert next(stream) == {"content": "first"}
    with pytest.raises(OSError, match="socket failed"):
        next(stream)

    assert response.closed


def test_stream_preserves_read_error_when_close_also_fails(monkeypatch):
    response = FakeStreamResponse(
        [OSError("read failed")],
        close_error=RuntimeError("close failed"),
    )
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)
    stream = LibreFang("http://example.test")._stream("GET", "/events")

    with pytest.raises(OSError, match="read failed"):
        next(stream)

    assert response.closed


def test_stream_consumer_close_ignores_response_close_error(monkeypatch):
    response = FakeStreamResponse(
        [b'data: {"content":"first"}\n\n'],
        close_error=RuntimeError("close failed"),
    )
    monkeypatch.setattr(client_module, "urlopen", lambda _request, **_kwargs: response)
    stream = LibreFang("http://example.test")._stream("GET", "/events")

    assert next(stream) == {"content": "first"}
    stream.close()

    assert response.closed
