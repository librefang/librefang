from __future__ import annotations

import importlib

from librefang.librefang_client import LibreFangError


class _BasicAgents:
    def __init__(self):
        self.deleted = []

    def list_agents(self):
        return {"items": [], "total": 0}

    def spawn_agent(self, **_data):
        return {"agent_id": "created-id"}

    def send_message(self, _agent_id, **_data):
        raise LibreFangError("connection lost")

    def kill_agent(self, agent_id, **query):
        self.deleted.append((agent_id, query))


class _StreamingAgents:
    def __init__(self):
        self.deleted = []

    def spawn_agent(self, **_data):
        return {"agent_id": "stream-id"}

    def send_message_stream(self, _agent_id, **_data):
        yield {"content": "hello"}
        yield {"error": "provider failed"}

    def kill_agent(self, agent_id, **query):
        self.deleted.append((agent_id, query))


class _Client:
    def __init__(self, agents):
        self.agents = agents
        self.system = self

    def health(self):
        return {"status": "ok"}


def test_basic_example_cleans_up_created_agent_after_request_failure(capsys):
    module = importlib.import_module("examples.client_basic")
    agents = _BasicAgents()

    assert module.main(_Client(agents)) == 1

    assert agents.deleted == [("created-id", {"confirm": True})]
    assert "LibreFang request failed: connection lost" in capsys.readouterr().err


def test_streaming_example_surfaces_error_event_and_cleans_up(capsys):
    module = importlib.import_module("examples.client_streaming")
    agents = _StreamingAgents()

    assert module.main(_Client(agents)) == 1

    assert agents.deleted == [("stream-id", {"confirm": True})]
    assert "Stream error: provider failed" in capsys.readouterr().err


def test_importing_echo_example_does_not_run_agent(monkeypatch):
    calls = []
    monkeypatch.setattr("librefang.Agent.run", lambda _self: calls.append(True))

    importlib.import_module("examples.echo_agent")

    assert calls == []
