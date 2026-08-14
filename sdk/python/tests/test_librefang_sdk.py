from __future__ import annotations

import io
import json

import pytest

from librefang.librefang_sdk import Agent, read_input


def _response(capsys):
    return json.loads(capsys.readouterr().out)


def test_read_input_wraps_malformed_json(monkeypatch):
    monkeypatch.setattr("sys.stdin", io.StringIO('{"secret":'))

    with pytest.raises(ValueError, match="Invalid JSON input from LibreFang kernel") as error:
        read_input()

    assert isinstance(error.value.__cause__, json.JSONDecodeError)
    assert "secret" not in str(error.value)


def test_agent_reports_invalid_input_without_running_handler(monkeypatch, capsys):
    monkeypatch.setattr("sys.stdin", io.StringIO("not-json\n"))
    agent = Agent()
    calls = []
    agent.on_message(lambda _message, _context: calls.append(True))

    with pytest.raises(SystemExit) as error:
        agent.run()

    assert error.value.code == 1
    assert calls == []
    assert _response(capsys) == {
        "type": "response",
        "text": "Invalid input received from the LibreFang kernel.",
    }


def test_agent_does_not_expose_handler_exception(monkeypatch, capsys):
    monkeypatch.setattr("sys.stdin", io.StringIO('{"message":"hello"}\n'))
    agent = Agent()

    @agent.on_message
    def fail(_message, _context):
        raise RuntimeError("credential=top-secret")

    with pytest.raises(SystemExit) as error:
        agent.run()

    captured = capsys.readouterr()
    assert error.value.code == 1
    assert json.loads(captured.out) == {
        "type": "response",
        "text": "An internal error occurred while processing your request.",
    }
    assert "top-secret" not in captured.out
    assert "Agent handler error" in captured.err
    assert "credential=top-secret" in captured.err


def test_agent_does_not_expose_setup_exception(monkeypatch, capsys):
    monkeypatch.setattr("sys.stdin", io.StringIO('{"message":"hello"}\n'))
    agent = Agent()
    agent.on_message(lambda _message, _context: "unused")

    @agent.on_setup
    def fail_setup():
        raise RuntimeError("internal-path=/private/agent")

    with pytest.raises(SystemExit):
        agent.run()

    captured = capsys.readouterr()
    assert "/private/agent" not in captured.out
    assert json.loads(captured.out)["text"] == (
        "An internal error occurred while processing your request."
    )
    assert "Agent setup error" in captured.err
    assert "internal-path=/private/agent" in captured.err
