"""Tests for librefang.sidecar.adapters.slack.

Deterministic, no network: urllib + WebSocket are monkeypatched /
replaced with a fake. Asserts the sidecar preserves the in-process
Rust ``librefang-channels::slack`` adapter's behaviour.
"""

import io
import json
import os
import urllib.error
import urllib.parse

import pytest


os.environ.setdefault("SLACK_APP_TOKEN", "xapp-test-app-token")
os.environ.setdefault("SLACK_BOT_TOKEN", "xoxb-test-bot-token")
from librefang.sidecar.adapters import slack as sa  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


# ---- _FakeUrlopen scaffolding -------------------------------------


def _adapter(**env):
    defaults = {
        "SLACK_APP_TOKEN": "xapp-test-app-token",
        "SLACK_BOT_TOKEN": "xoxb-test-bot-token",
        "SLACK_ALLOWED_CHANNELS": "",
        "SLACK_UNFURL_LINKS": "",
        "SLACK_FORCE_FLAT_REPLIES": "",
        "SLACK_REACTIONS": "",
        "SLACK_ACCOUNT_ID": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return sa.SlackAdapter()


# ---- env handling --------------------------------------------------


def test_default_api_base_and_tokens():
    a = _adapter()
    assert a.api_base == "https://slack.com/api"
    assert a.app_token == "xapp-test-app-token"
    assert a.bot_token == "xoxb-test-bot-token"
    assert a.allowed_channels == []
    assert a.unfurl_links is None
    assert a.force_flat_replies is False
    assert a.reactions_enabled is True
    assert a.account_id is None


def test_missing_app_token_exits_2():
    os.environ["SLACK_APP_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        sa.SlackAdapter()
    assert exc.value.code == 2
    os.environ["SLACK_APP_TOKEN"] = "xapp-test-app-token"


def test_missing_bot_token_exits_2():
    os.environ["SLACK_BOT_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        sa.SlackAdapter()
    assert exc.value.code == 2
    os.environ["SLACK_BOT_TOKEN"] = "xoxb-test-bot-token"


def test_allowed_channels_split():
    a = _adapter(SLACK_ALLOWED_CHANNELS="C0123, C0456 ,C0789")
    assert a.allowed_channels == ["C0123", "C0456", "C0789"]


def test_unfurl_links_tristate():
    a_unset = _adapter(SLACK_UNFURL_LINKS="")
    assert a_unset.unfurl_links is None
    a_true = _adapter(SLACK_UNFURL_LINKS="true")
    assert a_true.unfurl_links is True
    a_false = _adapter(SLACK_UNFURL_LINKS="false")
    assert a_false.unfurl_links is False


def test_force_flat_replies_default_false():
    a = _adapter()
    assert a.force_flat_replies is False
    a_true = _adapter(SLACK_FORCE_FLAT_REPLIES="true")
    assert a_true.force_flat_replies is True


def test_reactions_default_true():
    a = _adapter()
    assert a.reactions_enabled is True
    a_off = _adapter(SLACK_REACTIONS="false")
    assert a_off.reactions_enabled is False
    a_0 = _adapter(SLACK_REACTIONS="0")
    assert a_0.reactions_enabled is False


def test_account_id_passthrough():
    a = _adapter(SLACK_ACCOUNT_ID="workspace-prod")
    assert a.account_id == "workspace-prod"


# ---- _split_message ------------------------------------------------


def test_split_message_under_limit():
    assert sa._split_message("hello", 100) == ["hello"]


def test_split_message_newline_cut():
    text = "a" * 80 + "\n" + "b" * 80
    out = sa._split_message(text, 100)
    # Should cut at the newline so each chunk ends cleanly
    assert out[0] == "a" * 80
    assert out[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    text = "a" * 250
    out = sa._split_message(text, 100)
    assert out == ["a" * 100, "a" * 100, "a" * 50]


# ---- _split_csv / _bool_env ---------------------------------------


def test_split_csv_empty_and_whitespace():
    assert sa._split_csv("") == []
    assert sa._split_csv(", ,") == []
    assert sa._split_csv(" a , b") == ["a", "b"]


def test_bool_env_permissive():
    assert sa._bool_env("", default=True) is True
    assert sa._bool_env("", default=False) is False
    for s in ("true", "TRUE", "1", "yes", "ON"):
        assert sa._bool_env(s, default=False) is True
    for s in ("false", "0", "no", "OFF"):
        assert sa._bool_env(s, default=True) is False


# ---- parse_users_info ---------------------------------------------


def test_users_info_owner_precedence():
    role, err = sa.parse_users_info({
        "ok": True,
        "user": {"is_owner": True, "is_admin": True},
    })
    assert role == "owner"
    assert err is None


def test_users_info_primary_owner_treated_as_owner():
    role, _ = sa.parse_users_info({
        "ok": True,
        "user": {"is_primary_owner": True},
    })
    assert role == "owner"


def test_users_info_admin():
    role, _ = sa.parse_users_info({
        "ok": True,
        "user": {"is_admin": True},
    })
    assert role == "admin"


def test_users_info_guest():
    role, _ = sa.parse_users_info({
        "ok": True,
        "user": {"is_restricted": True},
    })
    assert role == "guest"
    role2, _ = sa.parse_users_info({
        "ok": True,
        "user": {"is_ultra_restricted": True},
    })
    assert role2 == "guest"


def test_users_info_member_fallback():
    role, _ = sa.parse_users_info({"ok": True, "user": {}})
    assert role == "member"


def test_users_info_not_found_is_silent_none():
    role, err = sa.parse_users_info({"ok": False, "error": "user_not_found"})
    assert role is None
    assert err is None


def test_users_info_unknown_error_returns_error():
    role, err = sa.parse_users_info({"ok": False, "error": "rate_limited"})
    assert role is None
    assert err == "rate_limited"


# ---- parse_slack_event --------------------------------------------


def _evt(**overrides):
    base = {
        "type": "message",
        "user": "U001",
        "channel": "C01",
        "text": "hello",
        "ts": "1700000000.000001",
    }
    base.update(overrides)
    return base


def test_parse_event_basic_text():
    ev = sa.parse_slack_event(
        _evt(),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_id"] == "C01"  # platform_id = channel
    assert ev["params"]["user_name"] == "U001"
    assert ev["params"]["content"] == {"Text": "hello"}
    assert ev["params"]["message_id"] == "1700000000.000001"
    assert ev["params"]["is_group"] is True
    assert ev["params"]["metadata"]["sender_user_id"] == "U001"


def test_parse_event_app_mention_flags_was_mentioned():
    ev = sa.parse_slack_event(
        _evt(type="app_mention", text="hi <@UBOT>"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev["params"]["metadata"]["was_mentioned"] is True


def test_parse_event_filters_self():
    ev = sa.parse_slack_event(
        _evt(user="UBOT"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_parse_event_filters_bot_id():
    ev = sa.parse_slack_event(
        _evt(**{"user": "U001"}, bot_id="B0BOT"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_parse_event_drops_unknown_subtype():
    ev = sa.parse_slack_event(
        {"type": "message", "subtype": "channel_join",
         "user": "U001", "channel": "C01", "ts": "1700000000.0"},
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_parse_event_message_changed_uses_inner_message():
    ev = sa.parse_slack_event(
        {
            "type": "message", "subtype": "message_changed",
            "channel": "C01", "ts": "1700000000.0",
            "message": {"user": "U001", "text": "edited",
                        "ts": "1699999999.0"},
        },
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["content"] == {"Text": "edited"}
    # message_changed prefers the inner ts over the event ts.
    assert ev["params"]["message_id"] == "1699999999.0"


def test_parse_event_slash_command():
    ev = sa.parse_slack_event(
        _evt(text="/agent hello world"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev["params"]["content"] == {
        "Command": {"name": "agent", "args": ["hello", "world"]},
    }


def test_parse_event_empty_text_dropped():
    ev = sa.parse_slack_event(
        _evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_parse_event_allowed_channels_filter_groups():
    ev = sa.parse_slack_event(
        _evt(channel="C99"),
        bot_user_id="UBOT", allowed_channels=["C01"], account_id=None,
    )
    assert ev is None


def test_parse_event_allowed_channels_dm_exempt():
    ev = sa.parse_slack_event(
        _evt(channel="DABC"),
        bot_user_id="UBOT", allowed_channels=["C01"], account_id=None,
    )
    assert ev is not None
    # DMs go through despite not being in the allowlist. The protocol
    # builder only emits is_group when True, so DMs surface it as
    # absent rather than `false`.
    assert ev["params"].get("is_group") in (None, False)


def test_parse_event_thread_ts_captured():
    ev = sa.parse_slack_event(
        _evt(thread_ts="1699000000.000001"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev["params"]["thread_id"] == "1699000000.000001"


def test_parse_event_top_level_thread_id_falls_back_to_ts():
    # #5302: a top-level message (no thread_ts) surfaces its own ts as
    # thread_id, so the reply threads under it (force_flat_replies opts
    # out) and on_send can finalize the :eyes: on the exact triggering
    # message — which is tracked by its own ts.
    ev = sa.parse_slack_event(
        _evt(ts="1700000000.000777"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev["params"]["thread_id"] == "1700000000.000777"
    assert ev["params"]["message_id"] == "1700000000.000777"


def test_parse_event_account_id_injected():
    ev = sa.parse_slack_event(
        _evt(),
        bot_user_id="UBOT", allowed_channels=[], account_id="ws-prod",
    )
    assert ev["params"]["metadata"]["account_id"] == "ws-prod"


# ---- parse_slack_block_action -------------------------------------


def _ba(**overrides):
    base = {
        "type": "block_actions",
        "user": {"id": "U001"},
        "channel": {"id": "C01"},
        "actions": [{"value": "approve", "action_id": "btn_approve"}],
        "message": {"text": "Do the thing?", "ts": "1700000000.0",
                    "thread_ts": "1699000000.0"},
        "trigger_id": "trg_123",
    }
    base.update(overrides)
    return base


def test_block_action_basic():
    ev = sa.parse_slack_block_action(
        _ba(),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["user_id"] == "C01"
    assert p["user_name"] == "U001"
    assert p["content"] == {
        "ButtonCallback": {"action": "approve", "message_text": "Do the thing?"},
    }
    assert p["message_id"] == "1700000000.0"
    assert p["thread_id"] == "1699000000.0"
    assert p["metadata"]["action_id"] == "btn_approve"
    assert p["metadata"]["block_action"] is True
    assert p["metadata"]["trigger_id"] == "trg_123"
    assert p["metadata"]["sender_user_id"] == "U001"


def test_block_action_drops_non_block_actions_type():
    ev = sa.parse_slack_block_action(
        _ba(type="shortcut"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_block_action_drops_self_user():
    ev = sa.parse_slack_block_action(
        _ba(user={"id": "UBOT"}),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_block_action_drops_empty_value():
    ev = sa.parse_slack_block_action(
        _ba(actions=[{"value": "", "action_id": "btn_x"}]),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    )
    assert ev is None


def test_block_action_respects_allowed_channels():
    ev = sa.parse_slack_block_action(
        _ba(channel={"id": "C99"}),
        bot_user_id="UBOT", allowed_channels=["C01"], account_id=None,
    )
    assert ev is None


# ---- _validate_bot_token ------------------------------------------


def test_validate_bot_token_happy(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True, "user_id": "UBOT"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._validate_bot_token() == "UBOT"
    call = fake.calls[0]
    assert call["url"].endswith("/auth.test")
    assert call["headers"]["authorization"] == "Bearer xoxb-test-bot-token"


def test_validate_bot_token_rejected(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": False, "error": "invalid_auth"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="invalid_auth"):
        a._validate_bot_token()


def test_validate_bot_token_missing_user_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing user_id"):
        a._validate_bot_token()


# ---- _fetch_socket_mode_url ---------------------------------------


def test_fetch_socket_mode_url(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True, "url": "wss://wss-primary.slack.com/link/?ticket=x"}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    url = a._fetch_socket_mode_url()
    assert url.startswith("wss://")
    call = fake.calls[0]
    assert call["url"].endswith("/apps.connections.open")
    # App-level token (xapp-), NOT the bot token.
    assert call["headers"]["authorization"] == "Bearer xapp-test-app-token"


def test_fetch_socket_mode_url_rejected(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": False, "error": "invalid_auth"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="invalid_auth"):
        a._fetch_socket_mode_url()


def test_fetch_socket_mode_url_non_wss(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True, "url": "https://not-a-ws-url.example.com"}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="invalid url"):
        a._fetch_socket_mode_url()


# ---- _post_message -------------------------------------------------


def test_post_message_basic_shape(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True, "ts": "1700000000.0"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("C01", "hi")
    call = fake.calls[0]
    assert call["url"].endswith("/chat.postMessage")
    assert call["method"] == "POST"
    assert call["headers"]["authorization"] == "Bearer xoxb-test-bot-token"
    body = json.loads(call["body_raw"])
    assert body == {"channel": "C01", "text": "hi"}


def test_post_message_with_thread_ts(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("C01", "reply", thread_ts="1699000000.0")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["thread_ts"] == "1699000000.0"


def test_post_message_chunks_at_3000(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True}), (200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("C01", "x" * 4500)
    assert len(fake.calls) == 2
    a1 = json.loads(fake.calls[0]["body_raw"])
    a2 = json.loads(fake.calls[1]["body_raw"])
    assert len(a1["text"]) == 3000
    assert len(a2["text"]) == 1500


def test_post_message_unfurl_links_explicit_false(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_UNFURL_LINKS="false")
    a._post_message("C01", "hi")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["unfurl_links"] is False


def test_post_message_unfurl_links_unset_omits_field(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("C01", "hi")
    body = json.loads(fake.calls[0]["body_raw"])
    assert "unfurl_links" not in body


def test_post_message_ok_false_logged_and_continues(monkeypatch):
    # Slack returns 200 with {"ok": false, "error": "channel_not_found"} —
    # we log but don't raise (fail-open, matches Rust).
    fake = _FakeUrlopen([(200, {"ok": False, "error": "channel_not_found"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("C01", "hi")  # must not raise


def test_post_message_5xx_logged_and_continues(monkeypatch):
    fake = _FakeUrlopen([(500, {"error": "server"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("C01", "hi")  # must not raise


def test_post_message_blocks_payload(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    blocks = sa._build_block_kit(
        "Pick one",
        [[
            {"label": "Approve", "action": "approve", "style": "primary"},
            {"label": "Reject", "action": "reject", "style": "danger"},
        ]],
    )
    a._post_message("C01", "Pick one", blocks=blocks)
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["channel"] == "C01"
    assert body["blocks"] == blocks


# ---- _build_block_kit ----------------------------------------------


def test_build_block_kit_text_section_first():
    blocks = sa._build_block_kit(
        "Hello",
        [[{"label": "OK", "action": "ok"}]],
    )
    assert blocks[0]["type"] == "section"
    assert blocks[0]["text"]["text"] == "Hello"


def test_build_block_kit_button_styles_and_url():
    blocks = sa._build_block_kit(
        "x",
        [[
            {"label": "Approve", "action": "approve", "style": "primary"},
            {"label": "Open", "action": "open",
             "url": "https://librefang.org"},
            {"label": "Bad", "action": "bad", "style": "warning"},  # unknown style filtered
        ]],
    )
    actions = blocks[1]
    assert actions["type"] == "actions"
    assert actions["block_id"] == "interactive_row_0"
    el = actions["elements"]
    assert el[0]["style"] == "primary"
    assert el[1]["url"] == "https://librefang.org"
    # warning is silently dropped (Slack only supports primary/danger).
    assert "style" not in el[2]


def test_build_block_kit_skips_malformed_rows():
    blocks = sa._build_block_kit(
        "x",
        [["not-a-dict-row"], [{"label": "OK", "action": "ok"}]],
    )
    # The malformed row contributes no actions block (the dict-only
    # check inside the row skips strings).
    assert len([b for b in blocks if b["type"] == "actions"]) == 1


# ---- _add_reaction / _remove_reaction ------------------------------


def test_add_reaction_disabled_no_call(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_REACTIONS="false")
    a._add_reaction("C01", "1700000000.0", "eyes")
    assert fake.calls == []


def test_add_reaction_already_reacted_is_silent(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": False, "error": "already_reacted"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._add_reaction("C01", "1700000000.0", "eyes")  # must not raise


def test_remove_reaction_no_reaction_is_silent(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": False, "error": "no_reaction"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._remove_reaction("C01", "1700000000.0", "eyes")


def test_pending_reactions_bounded():
    a = _adapter()
    # Force a tight cap so we can exercise the eviction in test time.
    a.MAX_PENDING_REACTIONS = 3
    for i in range(10):
        a._track_pending_reaction("C01", f"ts{i}", "eyes")
    assert len(a._pending_reactions) <= 3


def test_finalize_pending_reaction_uses_ts(monkeypatch):
    # Two HTTP calls: remove eyes + add white_check_mark.
    fake = _FakeUrlopen([
        (200, {"ok": True}),
        (200, {"ok": True}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._track_pending_reaction("C01", "1700000000.0", "eyes")
    a._finalize_pending_reaction("C01", "1700000000.0")
    urls = [c["url"] for c in fake.calls]
    assert urls[0].endswith("/reactions.remove")
    assert urls[1].endswith("/reactions.add")
    add_body = json.loads(fake.calls[1]["body_raw"])
    assert add_body["name"] == "white_check_mark"


def test_finalize_pending_reaction_disabled_noop(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_REACTIONS="false")
    a._finalize_pending_reaction("C01", "1700000000.0")
    assert fake.calls == []


# ---- _handle_envelope state machine -------------------------------


class _FakeWs:
    """Stand-in for ``_WebSocketClient`` used in unit tests."""

    def __init__(self):
        self.sent_text: list[str] = []
        self.sent_close = False
        self.readable_calls = 0

    def send_text(self, s):
        self.sent_text.append(s)

    def send_close(self):
        self.sent_close = True

    def settimeout(self, _t):
        pass

    def wait_readable(self, _timeout):
        self.readable_calls += 1
        return True

    def recv_frame(self):
        raise EOFError("not used in handle-envelope tests")


def test_handle_events_api_acks_and_emits(monkeypatch):
    # We must monkeypatch reactions urlopen because the events_api
    # path issues `reactions.add` (eyes) synchronously after parsing.
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "UBOT"
    ws = _FakeWs()
    emitted = []
    a._handle_envelope(
        {
            "type": "events_api",
            "envelope_id": "env-1",
            "payload": {"event": _evt()},
        },
        ws=ws,
        emit=emitted.append,
    )
    # ACK fires first with the envelope id.
    assert ws.sent_text
    assert json.loads(ws.sent_text[0]) == {"envelope_id": "env-1"}
    # One emitted message event.
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"] == {"Text": "hello"}
    # Reactions.add was called.
    assert fake.calls
    assert fake.calls[0]["url"].endswith("/reactions.add")


def test_handle_interactive_acks_and_emits(monkeypatch):
    fake = _FakeUrlopen([])  # interactive path doesn't hit HTTP
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "UBOT"
    ws = _FakeWs()
    emitted = []
    a._handle_envelope(
        {
            "type": "interactive",
            "envelope_id": "env-2",
            "payload": _ba(),
        },
        ws=ws,
        emit=emitted.append,
    )
    assert json.loads(ws.sent_text[0]) == {"envelope_id": "env-2"}
    assert len(emitted) == 1
    assert "ButtonCallback" in emitted[0]["params"]["content"]


def test_handle_hello_no_op():
    a = _adapter()
    ws = _FakeWs()
    a._handle_envelope({"type": "hello"}, ws=ws, emit=lambda _e: None)
    assert ws.sent_text == []  # no ack for hello


def test_handle_disconnect_raises():
    a = _adapter()
    ws = _FakeWs()
    with pytest.raises(RuntimeError, match="slack-disconnect"):
        a._handle_envelope(
            {"type": "disconnect", "reason": "warning"},
            ws=ws,
            emit=lambda _e: None,
        )


def test_handle_events_api_skipped_event_no_emit(monkeypatch):
    fake = _FakeUrlopen([])  # nothing should hit HTTP
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "UBOT"
    ws = _FakeWs()
    emitted = []
    a._handle_envelope(
        {
            "type": "events_api",
            "envelope_id": "env-3",
            # Self-message — parse_slack_event drops it.
            "payload": {"event": _evt(user="UBOT")},
        },
        ws=ws,
        emit=emitted.append,
    )
    # ACK still sent (mandatory regardless of whether we emit), but no
    # emit and no reactions.add.
    assert json.loads(ws.sent_text[0]) == {"envelope_id": "env-3"}
    assert emitted == []
    assert fake.calls == []


# ---- on_send routing ----------------------------------------------


@pytest.mark.asyncio
async def test_on_send_text_uses_channel_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = "C01"
        text = "hi"
        content = {"Text": "hi"}
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["channel"] == "C01"
    assert body["text"] == "hi"
    assert "thread_ts" not in body


@pytest.mark.asyncio
async def test_on_send_threads_with_thread_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.reactions_enabled = False  # avoid extra reactions calls in this test

    class _Cmd:
        channel_id = "C01"
        text = "hi"
        content = {"Text": "hi"}
        thread_id = "1699000000.0"
        user = {}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["thread_ts"] == "1699000000.0"


@pytest.mark.asyncio
async def test_on_send_force_flat_replies_drops_thread_ts(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_FORCE_FLAT_REPLIES="true")
    a.reactions_enabled = False

    class _Cmd:
        channel_id = "C01"
        text = "hi"
        content = {"Text": "hi"}
        thread_id = "1699000000.0"
        user = {}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert "thread_ts" not in body


@pytest.mark.asyncio
async def test_on_send_force_flat_finalizes_correct_message(monkeypatch):
    # Regression (#5302): in force-flat mode the *post* drops thread_ts,
    # but reaction finalization must still target the inbound message
    # (cmd.thread_id) instead of falling back to "first pending in the
    # channel" — otherwise concurrent messages flip the wrong :eyes:.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # chat.postMessage
        (200, {"ok": True}),  # reactions.remove
        (200, {"ok": True}),  # reactions.add
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_FORCE_FLAT_REPLIES="true")
    # Two concurrent inbound messages got :eyes: in the same channel; T0
    # is older, so the buggy fallback would have flipped it instead of T1.
    a._track_pending_reaction("C01", "T0", "eyes")
    a._track_pending_reaction("C01", "T1", "eyes")

    class _Cmd:
        channel_id = "C01"
        text = "hi"
        content = {"Text": "hi"}
        thread_id = "T1"
        user = {}

    await a.on_send(_Cmd())
    # The post is flat (force_flat dropped thread_ts)...
    post_body = json.loads(fake.calls[0]["body_raw"])
    assert "thread_ts" not in post_body
    # ...but finalization targets T1, not the older T0.
    assert fake.calls[1]["url"].endswith("/reactions.remove")
    assert fake.calls[2]["url"].endswith("/reactions.add")
    assert json.loads(fake.calls[1]["body_raw"])["timestamp"] == "T1"
    add_body = json.loads(fake.calls[2]["body_raw"])
    assert add_body["timestamp"] == "T1"
    assert add_body["name"] == "white_check_mark"
    # T1 finalized & removed; T0 stays pending (it was a different message).
    assert ("C01", "T1") not in a._pending_reactions
    assert ("C01", "T0") in a._pending_reactions


@pytest.mark.asyncio
async def test_on_send_interactive_uses_blocks(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.reactions_enabled = False

    class _Cmd:
        channel_id = "C01"
        text = ""
        content = {
            "Interactive": {
                "text": "Pick one",
                "buttons": [[
                    {"label": "OK", "action": "ok"},
                ]],
            },
        }
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["text"] == "Pick one"
    assert any(b["type"] == "actions" for b in body["blocks"])


@pytest.mark.asyncio
async def test_on_send_unsupported_content_placeholder(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.reactions_enabled = False

    class _Cmd:
        channel_id = "C01"
        text = ""
        content = {"Command": {"name": "noop", "args": []}}
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["text"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.reactions_enabled = False

    class _Cmd:
        channel_id = ""
        text = "hi"
        content = {"Text": "hi"}
        thread_id = None
        user = {"platform_id": "C01-fallback"}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["channel"] == "C01-fallback"


@pytest.mark.asyncio
async def test_on_send_drops_when_no_channel(monkeypatch):
    fake = _FakeUrlopen([])  # no HTTP expected
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = ""
        text = "hi"
        content = {"Text": "hi"}
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    assert fake.calls == []
