"""UTF-8 decoding in the generated SSE client is independent of read boundaries."""

import librefang.librefang_client as generated_client


class _ChunkedResponse:
    def __init__(self, chunks):
        self._chunks = iter(chunks)
        self.closed = False

    def read(self, size):
        assert size == 4096
        return next(self._chunks)

    def close(self):
        self.closed = True


def test_stream_reassembles_utf8_split_at_4096_byte_boundary(monkeypatch):
    prefix = b'data: {"text":"'
    emoji = "😀".encode()
    filler = b"a" * (4096 - len(prefix) - 2)
    first = prefix + filler + emoji[:2]
    second = emoji[2:] + b'"}\n'
    assert len(first) == 4096

    response = _ChunkedResponse([first, second, b""])
    monkeypatch.setattr(generated_client, "urlopen", lambda _request: response)

    events = list(generated_client.LibreFang("http://daemon")._stream("GET", "/events"))

    assert events == [{"text": filler.decode() + "😀"}]
    assert response.closed
