"""Connection failures from the generated client use its public error type."""

from urllib.error import URLError

import pytest

import librefang.librefang_client as generated_client


@pytest.mark.parametrize("stream", [False, True])
def test_urlerror_is_wrapped_as_librefang_error(monkeypatch, stream):
    failure = URLError("name resolution failed")

    def fail_urlopen(_request):
        raise failure

    monkeypatch.setattr(generated_client, "urlopen", fail_urlopen)
    client = generated_client.LibreFang("http://unreachable.invalid")

    with pytest.raises(generated_client.LibreFangError) as caught:
        if stream:
            next(client._stream("GET", "/events"))
        else:
            client._request("GET", "/health")

    assert str(caught.value) == "Connection error: name resolution failed"
    assert caught.value.status == 0
    assert caught.value.body == ""
    assert caught.value.__cause__ is failure
