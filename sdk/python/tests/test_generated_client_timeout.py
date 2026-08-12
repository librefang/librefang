import socket

import pytest

from librefang import librefang_client as client_module
from librefang.librefang_client import LibreFang, LibreFangError


class FakeResponse:
    headers = {"content-type": "application/json"}

    def __init__(self):
        self.reads = 0

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self, *_args):
        self.reads += 1
        return b"{}" if self.reads == 1 else b""

    def close(self):
        pass


def test_client_applies_default_timeout_to_request_and_stream(monkeypatch):
    timeouts = []
    monkeypatch.setattr(
        client_module,
        "urlopen",
        lambda _request, *, timeout: (timeouts.append(timeout), FakeResponse())[1],
    )
    client = LibreFang("http://example.test")

    client._request("GET", "/health")
    list(client._stream("GET", "/events"))

    assert timeouts == [30.0, 30.0]


def test_client_allows_timeout_override(monkeypatch):
    timeouts = []
    monkeypatch.setattr(
        client_module,
        "urlopen",
        lambda _request, *, timeout: (timeouts.append(timeout), FakeResponse())[1],
    )
    client = LibreFang("http://example.test", timeout=4.5)

    client._request("GET", "/health")
    list(client._stream("GET", "/events"))

    assert timeouts == [4.5, 4.5]


def test_request_wraps_response_timeout_as_libre_fang_error(monkeypatch):
    # urlopen raises socket.timeout (an OSError subclass; identical to the builtin TimeoutError from Python 3.10 on, but a distinct class on the 3.8/3.9 this SDK still supports) when the connection succeeds but the server is too slow to send a response within the timeout window — this is the exact case a bounded timeout exists to catch, so it must surface through the SDK's own error type rather than leaking a raw stdlib exception past the client's error contract.
    def timed_out(_request, *, timeout):
        raise socket.timeout("timed out")

    monkeypatch.setattr(client_module, "urlopen", timed_out)
    client = LibreFang("http://example.test", timeout=1.0)

    with pytest.raises(LibreFangError, match="timed out after 1.0s"):
        client._request("GET", "/health")


def test_stream_open_wraps_response_timeout_as_libre_fang_error(monkeypatch):
    def timed_out(_request, *, timeout):
        raise socket.timeout("timed out")

    monkeypatch.setattr(client_module, "urlopen", timed_out)
    client = LibreFang("http://example.test", timeout=1.0)

    with pytest.raises(LibreFangError, match="timed out after 1.0s"):
        list(client._stream("GET", "/events"))


class StalledStreamResponse:
    # urlopen() succeeds and headers arrive, but the connection goes quiet partway through the body — the exact "stalled socket read" case the bounded timeout exists to catch, distinct from a timeout during connection setup.
    headers = {"content-type": "text/event-stream"}

    def read(self, *_args):
        raise socket.timeout("timed out")

    def close(self):
        pass


def test_stream_body_read_wraps_response_timeout_as_libre_fang_error(monkeypatch):
    monkeypatch.setattr(
        client_module,
        "urlopen",
        lambda _request, *, timeout: StalledStreamResponse(),
    )
    client = LibreFang("http://example.test", timeout=1.0)

    with pytest.raises(LibreFangError, match="timed out after 1.0s"):
        list(client._stream("GET", "/events"))
