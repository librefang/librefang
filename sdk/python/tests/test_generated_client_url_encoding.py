"""The generated client encodes path parameters and boolean query values the way the server parses them."""

import io

import librefang.librefang_client as generated_client


class _Response(io.BytesIO):
    headers = {"content-type": "application/json"}

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        return False


def _capture(monkeypatch):
    seen = []

    def fake_urlopen(request, **_kwargs):
        seen.append(request.full_url)
        return _Response(b"{}")

    monkeypatch.setattr(generated_client, "urlopen", fake_urlopen)
    return seen


def test_boolean_query_values_serialize_as_lowercase(monkeypatch):
    seen = _capture(monkeypatch)
    client = generated_client.LibreFang("http://daemon.invalid")

    client.agents.kill_agent("agent-1", confirm=True)
    client.agents.kill_agent("agent-1", confirm=False)

    # serde only accepts `true` / `false`; Python's `True` was a 400 on every destructive confirm.
    assert seen == [
        "http://daemon.invalid/api/agents/agent-1?confirm=true",
        "http://daemon.invalid/api/agents/agent-1?confirm=false",
    ]


def test_path_parameters_are_percent_encoded_as_one_segment(monkeypatch):
    seen = _capture(monkeypatch)
    client = generated_client.LibreFang("http://daemon.invalid")

    client.memory.get_agent_kv_key("agent 1", "prefs?v=2/../x#frag%")

    assert seen == [
        "http://daemon.invalid/api/memory/agents/agent%201/kv/prefs%3Fv%3D2%2F..%2Fx%23frag%25",
    ]
