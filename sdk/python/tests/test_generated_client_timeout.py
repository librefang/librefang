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
    # urlopen raises a bare TimeoutError (not URLError) when the connection succeeds but the server is too slow to send a response within the timeout window — this is the exact case a bounded timeout exists to catch, so it must surface through the SDK's own error type rather than leaking a raw stdlib exception past the client's error contract.
    def timed_out(_request, *, timeout):
        raise TimeoutError("timed out")

    monkeypatch.setattr(client_module, "urlopen", timed_out)
    client = LibreFang("http://example.test", timeout=1.0)

    with pytest.raises(LibreFangError, match="timed out after 1.0s"):
        client._request("GET", "/health")


def test_stream_open_wraps_response_timeout_as_libre_fang_error(monkeypatch):
    def timed_out(_request, *, timeout):
        raise TimeoutError("timed out")

    monkeypatch.setattr(client_module, "urlopen", timed_out)
    client = LibreFang("http://example.test", timeout=1.0)

    with pytest.raises(LibreFangError, match="timed out after 1.0s"):
        list(client._stream("GET", "/events"))
