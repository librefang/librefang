from librefang import librefang_client as client_module
from librefang.librefang_client import LibreFang


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
