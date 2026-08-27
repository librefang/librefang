"""Tests for librefang.sidecar.adapters.slack.

Deterministic, no network: urllib + WebSocket are monkeypatched /
replaced with a fake. Asserts the sidecar preserves the in-process
Rust ``librefang-channels::slack`` adapter's behaviour.
"""

import http.client
import io
import json
import os
import socket
import urllib.error
import urllib.parse
import urllib.request

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
        "SLACK_PROGRESS_CARD": "",
        "SLACK_ACCOUNT_ID": "",
        "SLACK_FILE_DOWNLOADS": "",
        "SLACK_FILE_MAX_BYTES": "",
        "SLACK_FILE_ALLOWED_EXTENSIONS": "",
        "SLACK_FILE_DOWNLOAD_CHANNELS": "",
        "SLACK_FILE_DOWNLOAD_EXCLUDE_CHANNELS": "",
        "SLACK_RESOLVE_DISPLAY_NAMES": "",
        "SLACK_DISPLAY_NAME_TTL": "",
        "SLACK_ENUMERATE_MEMBERS": "",
        "SLACK_MEMBER_LIST_TTL": "",
        "SLACK_MEMBER_LIST_MAX": "",
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
    assert a.progress_card_enabled is True
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


def test_progress_card_defaults_to_following_reactions():
    # #6730 decoupled the card from the receipt, but an operator already running SLACK_REACTIONS=false for total silence must not start receiving cards on upgrade — so the card's default follows the receipt knob rather than being an unconditional true.
    assert _adapter().progress_card_enabled is True
    assert _adapter(SLACK_REACTIONS="false").progress_card_enabled is False
    # …and each is independently overridable, which is the point of #6730.
    a_card_only = _adapter(SLACK_REACTIONS="false", SLACK_PROGRESS_CARD="true")
    assert a_card_only.reactions_enabled is False
    assert a_card_only.progress_card_enabled is True
    a_receipt_only = _adapter(SLACK_PROGRESS_CARD="false")
    assert a_receipt_only.reactions_enabled is True
    assert a_receipt_only.progress_card_enabled is False


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


# ---- inbound attachments (#7087) ----------------------------------


_SLACK_IMAGE_URL = (
    "https://files.slack.com/files-pri/T01-F01/download/campaign.png"
)


def _file(**overrides):
    base = {
        "id": "F01",
        "name": "campaign.png",
        "title": "campaign.png",
        "mimetype": "image/png",
        "filetype": "png",
        "size": 2048,
        "url_private": _SLACK_IMAGE_URL.replace("/download/", "/"),
        "url_private_download": _SLACK_IMAGE_URL,
    }
    base.update(overrides)
    return base


def _file_evt(*, files=None, text="Post this to LinkedIn", **overrides):
    base = {
        "type": "message",
        "subtype": "file_share",
        "user": "U001",
        "channel": "C01",
        "text": text,
        "ts": "1700000000.000001",
        "files": [_file()] if files is None else files,
    }
    base.update(overrides)
    return base


def _policy(**overrides):
    defaults = {"enabled": True}
    defaults.update(overrides)
    return sa.SlackFilePolicy(**defaults)


def test_parse_event_file_share_emits_image_content():
    ev = sa.parse_slack_event(
        _file_evt(),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    )
    assert ev is not None
    assert ev["params"]["content"] == {
        "Image": {
            "url": _SLACK_IMAGE_URL,
            "caption": "Post this to LinkedIn",
            "mime_type": "image/png",
        },
    }
    # Routing metadata is unchanged by the attachment path.
    assert ev["params"]["user_id"] == "C01"
    assert ev["params"]["message_id"] == "1700000000.000001"
    assert ev["params"]["metadata"]["sender_user_id"] == "U001"


def test_parse_event_file_share_dropped_without_a_policy():
    """Default (no policy) keeps the pre-#7087 behaviour: file_share is discarded."""
    assert sa.parse_slack_event(
        _file_evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
    ) is None


def test_parse_event_file_share_with_no_comment_is_still_emitted():
    ev = sa.parse_slack_event(
        _file_evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    )
    assert ev is not None
    assert ev["params"]["content"]["Image"]["caption"] is None


def test_parse_event_file_share_video_and_audio_and_document_variants():
    video = sa.parse_slack_event(
        _file_evt(files=[_file(
            name="review.mp4", mimetype="video/mp4", filetype="mp4",
            duration_ms=90_500,
        )], text="review this"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    )
    assert video["params"]["content"] == {
        "Video": {
            "url": _SLACK_IMAGE_URL,
            "caption": "review this",
            "duration_seconds": 90,
            "filename": "review.mp4",
        },
    }

    audio = sa.parse_slack_event(
        _file_evt(files=[_file(
            name="memo.m4a", title="Standup memo", mimetype="audio/mp4",
            filetype="m4a", duration_ms=12_000,
        )], text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    )
    assert audio["params"]["content"] == {
        "Audio": {
            "url": _SLACK_IMAGE_URL,
            "caption": None,
            "duration_seconds": 12,
            "title": "Standup memo",
        },
    }

    document = sa.parse_slack_event(
        _file_evt(files=[_file(
            name="brief.pdf", mimetype="application/pdf", filetype="pdf",
        )], text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    )
    assert document["params"]["content"] == {
        "File": {"url": _SLACK_IMAGE_URL, "filename": "brief.pdf"},
    }


def test_parse_event_file_share_oversize_is_rejected():
    ev = sa.parse_slack_event(
        _file_evt(files=[_file(size=64 * 1024 * 1024)], text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(max_bytes=1024),
    )
    assert ev is None


def test_parse_event_file_share_oversize_keeps_the_companion_text():
    """A rejected attachment must not swallow the message the user typed with it."""
    ev = sa.parse_slack_event(
        _file_evt(files=[_file(size=64 * 1024 * 1024)]),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(max_bytes=1024),
    )
    assert ev["params"]["content"] == {"Text": "Post this to LinkedIn"}


def test_parse_event_file_share_disallowed_extension_is_rejected():
    ev = sa.parse_slack_event(
        _file_evt(files=[_file(name="payload.exe", filetype="exe",
                               mimetype="application/octet-stream")],
                  text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(allowed_extensions=frozenset({"png", "pdf"})),
    )
    assert ev is None


def test_parse_event_file_share_allowed_extension_passes():
    ev = sa.parse_slack_event(
        _file_evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(allowed_extensions=frozenset({"png", "pdf"})),
    )
    assert "Image" in ev["params"]["content"]


def test_parse_event_file_share_extension_falls_back_to_filetype():
    ev = sa.parse_slack_event(
        _file_evt(files=[_file(name="screenshot", title="screenshot",
                               filetype="png")], text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(allowed_extensions=frozenset({"png"})),
    )
    assert "Image" in ev["params"]["content"]


def test_parse_event_file_share_download_switch_off_drops_the_file():
    ev = sa.parse_slack_event(
        _file_evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(enabled=False),
    )
    assert ev is None


def test_parse_event_file_share_per_channel_exclude_list():
    policy = _policy(excluded_channels=("C01",))
    assert sa.parse_slack_event(
        _file_evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=policy,
    ) is None
    # A different channel is unaffected by the exclusion.
    assert sa.parse_slack_event(
        _file_evt(channel="C02", text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=policy,
    ) is not None


def test_parse_event_file_share_per_channel_allow_list():
    policy = _policy(channels=("C02",))
    assert sa.parse_slack_event(
        _file_evt(text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=policy,
    ) is None
    assert sa.parse_slack_event(
        _file_evt(channel="C02", text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=policy,
    ) is not None


def test_parse_event_file_share_rejects_non_slack_host():
    """A member-supplied `url_private` off Slack's file hosts is never forwarded."""
    for url in (
        "https://evil.example/files-pri/T01-F01/x.png",
        "https://files.slack.com.evil.example/x.png",
        "https://files.slack.com@evil.example/x.png",
        "http://files.slack.com/x.png",
    ):
        ev = sa.parse_slack_event(
            _file_evt(files=[_file(url_private=url,
                                   url_private_download=url)], text=""),
            bot_user_id="UBOT", allowed_channels=[], account_id=None,
            file_policy=_policy(),
        )
        assert ev is None, url


def test_parse_event_file_share_takes_first_eligible_and_skips_extras():
    ev = sa.parse_slack_event(
        _file_evt(files=[
            _file(name="huge.png", size=99 * 1024 * 1024,
                  url_private_download=_SLACK_IMAGE_URL + "?f=huge"),
            _file(name="second.png",
                  url_private_download=_SLACK_IMAGE_URL + "?f=second"),
            _file(name="third.png",
                  url_private_download=_SLACK_IMAGE_URL + "?f=third"),
        ], text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(max_bytes=1024 * 1024),
    )
    # The oversize first entry is skipped, the next eligible one is
    # forwarded, and the third is counted as an ignored extra.
    assert ev["params"]["content"]["Image"]["url"] == (
        _SLACK_IMAGE_URL + "?f=second"
    )


def test_parse_event_attachment_outranks_slash_command():
    ev = sa.parse_slack_event(
        _file_evt(text="/summarize please"),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    )
    assert "Image" in ev["params"]["content"]
    assert ev["params"]["content"]["Image"]["caption"] == "/summarize please"


def test_parse_event_file_share_still_honours_self_skip_and_channel_filter():
    assert sa.parse_slack_event(
        _file_evt(user="UBOT", text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    ) is None
    assert sa.parse_slack_event(
        _file_evt(channel="C99", text=""),
        bot_user_id="UBOT", allowed_channels=["C01"], account_id=None,
        file_policy=_policy(),
    ) is None


def test_parse_event_other_subtypes_are_still_dropped():
    assert sa.parse_slack_event(
        {"type": "message", "subtype": "channel_join", "user": "U001",
         "channel": "C01", "text": "joined", "ts": "1.0",
         "files": [_file()]},
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    ) is None


def test_parse_event_file_share_rejects_unfetchable_modes():
    for mode in ("tombstone", "hidden_by_limit"):
        assert sa.parse_slack_event(
            _file_evt(files=[_file(mode=mode)], text=""),
            bot_user_id="UBOT", allowed_channels=[], account_id=None,
            file_policy=_policy(),
        ) is None, mode
    # A normal hosted file is unaffected by the mode check.
    assert sa.parse_slack_event(
        _file_evt(files=[_file(mode="hosted")], text=""),
        bot_user_id="UBOT", allowed_channels=[], account_id=None,
        file_policy=_policy(),
    ) is not None


def test_parse_slack_files_ignores_malformed_arrays():
    for files in (None, {}, "campaign.png", [], [None], [{}], [{"name": "x"}]):
        assert sa.parse_slack_files(
            files, channel="C01", companion_text="", policy=_policy(),
        ) is None


def test_inbound_attachment_parsing_performs_no_http():
    """Parsing hands the daemon a URL; the adapter itself never fetches inbound files."""
    def _explode(*_a, **_k):
        raise AssertionError("inbound parsing must not perform HTTP")

    original_public = sa._public_http_request
    original_request = sa._http_request
    sa._public_http_request = _explode
    sa._http_request = _explode
    try:
        ev = sa.parse_slack_event(
            _file_evt(),
            bot_user_id="UBOT", allowed_channels=[], account_id=None,
            file_policy=_policy(),
        )
        assert "Image" in ev["params"]["content"]
        assert sa.parse_slack_event(
            _file_evt(text=""),
            bot_user_id="UBOT", allowed_channels=[], account_id=None,
            file_policy=_policy(enabled=False),
        ) is None
    finally:
        sa._public_http_request = original_public
        sa._http_request = original_request


# ---- attachment policy env wiring ---------------------------------


def test_file_policy_defaults():
    a = _adapter()
    assert a.file_policy.enabled is True
    assert a.file_policy.max_bytes == sa.DEFAULT_INBOUND_FILE_MAX_BYTES
    assert a.file_policy.allowed_extensions == frozenset()
    assert a.file_policy.channels == ()
    assert a.file_policy.excluded_channels == ()


def test_file_policy_env_parsing():
    a = _adapter(
        SLACK_FILE_MAX_BYTES="4096",
        SLACK_FILE_ALLOWED_EXTENSIONS=".PNG, jpg , ,pdf",
        SLACK_FILE_DOWNLOAD_CHANNELS="C01, C02",
        SLACK_FILE_DOWNLOAD_EXCLUDE_CHANNELS="C09",
    )
    assert a.file_policy.max_bytes == 4096
    assert a.file_policy.allowed_extensions == frozenset({"png", "jpg", "pdf"})
    assert a.file_policy.channels == ("C01", "C02")
    assert a.file_policy.excluded_channels == ("C09",)


def test_file_max_bytes_non_integer_exits_2():
    with pytest.raises(SystemExit) as e:
        _adapter(SLACK_FILE_MAX_BYTES="ten megabytes")
    assert e.value.code == 2


def test_file_max_bytes_below_one_falls_back_to_default():
    a = _adapter(SLACK_FILE_MAX_BYTES="0")
    assert a.file_policy.max_bytes == sa.DEFAULT_INBOUND_FILE_MAX_BYTES


def test_header_rules_pin_the_bot_token_to_slack_file_hosts():
    a = _adapter()
    assert a.header_rules == [
        ("files.slack.com", [["Authorization", "Bearer xoxb-test-bot-token"]]),
        ("slack-files.com", [["Authorization", "Bearer xoxb-test-bot-token"]]),
    ]
    # Every host the token is declared for is a Slack file host, so the
    # daemon's exact-host `fetch_headers_for` match can never attach it to
    # an attacker-named URL.
    hosts = [host for host, _headers in a.header_rules]
    assert hosts == sorted(sa.SLACK_FILE_HOSTS)
    for host in hosts:
        assert host == "slack-files.com" or host.endswith(".slack.com")


def test_header_rules_absent_when_downloads_are_disabled():
    """Switch the feature off and the bot token is not shipped to the daemon at all."""
    a = _adapter(SLACK_FILE_DOWNLOADS="false")
    assert a.file_policy.enabled is False
    assert a.header_rules == []
    assert "xoxb-test-bot-token" not in json.dumps(a.ready_event())


def test_header_rules_surface_in_the_ready_event():
    a = _adapter()
    rules = a.ready_event()["params"]["header_rules"]
    assert rules[0][0] == "files.slack.com"
    assert rules[0][1] == [["Authorization", "Bearer xoxb-test-bot-token"]]


def test_is_slack_file_url_host_pinning():
    assert sa._is_slack_file_url(_SLACK_IMAGE_URL) is True
    assert sa._is_slack_file_url("https://FILES.SLACK.COM./x.png") is True
    assert sa._is_slack_file_url("https://slack-files.com/x.png") is True
    assert sa._is_slack_file_url("https://evil.example/x.png") is False
    assert sa._is_slack_file_url("https://slack.com/x.png") is False
    assert sa._is_slack_file_url("") is False
    assert sa._is_slack_file_url(None) is False


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


def test_post_message_honours_retry_after_on_429(monkeypatch):
    """Slack rate-limits chat.postMessage (Tier 4: 100/min global +
    per-method quotas). The 3-tuple `_http` helper historically
    threw away response headers and conflated 429 with 5xx, silently
    dropping the chunk on every rate limit. The fix routes 429 with
    Retry-After through `parse_retry_after` and retries once."""
    fake = _FakeUrlopen([
        (429, {"ok": False, "error": "ratelimited"}, {"Retry-After": "4"}),
        (200, {"ok": True, "ts": "1700000000.0"}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(sa.time, "sleep", sleeps.append)
    a = _adapter()
    a._post_message("C01", "hi")
    # Retry-After honoured at the server-suggested 4 s window.
    assert sleeps == [4.0]
    # Original probe + retry — both with the same body shape so the
    # retry actually re-posts the chunk rather than a stale snapshot.
    assert len(fake.calls) == 2
    body = json.loads(fake.calls[1]["body_raw"])
    assert body == {"channel": "C01", "text": "hi"}


def test_post_message_surfaces_second_429(monkeypatch):
    """If the server still returns 429 after honouring Retry-After,
    the second response falls through to the `status >= 300` arm and
    is logged-and-continued (matching the Rust fail-open behaviour)
    rather than looping forever."""
    fake = _FakeUrlopen([
        (429, {"ok": False}, {"Retry-After": "1"}),
        (429, {"ok": False}, {"Retry-After": "1"}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(sa.time, "sleep", lambda _s: None)
    a = _adapter()
    a._post_message("C01", "hi")  # must not raise / must not loop
    # Exactly one sleep then surface — two POSTs total.
    assert len(fake.calls) == 2


def test_post_message_429_without_retry_after_uses_default(monkeypatch):
    fake = _FakeUrlopen([
        (429, {"ok": False}, {}),
        (200, {"ok": True}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(sa.time, "sleep", sleeps.append)
    a = _adapter()
    a._post_message("C01", "hi")
    from librefang.sidecar import common as _common
    assert sleeps == [_common.RETRY_AFTER_DEFAULT_SECS]


def test_add_reaction_honours_retry_after_on_429(monkeypatch):
    """Slack rate-limits reactions.add (Tier 3) — same fix applies
    via the shared `_http` helper, no per-callsite change needed."""
    fake = _FakeUrlopen([
        (429, {"ok": False}, {"Retry-After": "2"}),
        (200, {"ok": True}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(sa.time, "sleep", sleeps.append)
    a = _adapter()
    a._add_reaction("C01", "1700000000.0", "thumbsup")
    assert sleeps == [2.0]
    assert len(fake.calls) == 2


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


def test_upload_file_bytes_uses_external_upload_flow(monkeypatch):
    fake = _FakeUrlopen([
        (200, {
            "ok": True,
            "upload_url": "https://files.slack.com/upload/v1/TICKET",
            "file_id": "F123",
        }),
        (200, {"ok": True, "files": [{"id": "F123"}]}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    uploads = []
    monkeypatch.setattr(
        sa,
        "_public_http_request",
        lambda url, **kwargs: uploads.append((url, kwargs)) or (200, b""),
    )
    a = _adapter()

    assert a._upload_file_bytes(
        "C01", b"report-bytes", "report.xlsx", thread_ts="1700000000.0",
    ) is True

    assert [call["url"] for call in fake.calls] == [
        "https://slack.com/api/files.getUploadURLExternal",
        "https://slack.com/api/files.completeUploadExternal",
    ]
    assert fake.calls[0]["body"] == {
        "filename": "report.xlsx",
        "length": len(b"report-bytes"),
    }
    assert uploads == [(
        "https://files.slack.com/upload/v1/TICKET",
        {
            "method": "POST",
            "body": b"report-bytes",
            "headers": {"Content-Type": "application/octet-stream"},
            "max_bytes": 200,
            "require_https": True,
        },
    )]
    assert fake.calls[1]["body"] == {
        "files": [{"id": "F123", "title": "report.xlsx"}],
        "channel_id": "C01",
        "thread_ts": "1700000000.0",
    }


def test_upload_file_bytes_stops_when_ticket_is_rejected(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": False, "error": "missing_scope"}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()

    assert a._upload_file_bytes("C01", b"x", "x.txt") is False
    assert len(fake.calls) == 1


def test_upload_file_bytes_stops_when_byte_upload_fails(monkeypatch):
    fake = _FakeUrlopen([
        (200, {
            "ok": True,
            "upload_url": "https://files.slack.com/upload/v1/TICKET",
            "file_id": "F123",
        }),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(sa, "_public_http_request", lambda *_args, **_kwargs: (500, b"upload_failed"))
    a = _adapter()

    assert a._upload_file_bytes("C01", b"x", "x.txt") is False
    assert len(fake.calls) == 1


def test_upload_file_bytes_surfaces_completion_rejection(monkeypatch):
    fake = _FakeUrlopen([
        (200, {
            "ok": True,
            "upload_url": "https://files.slack.com/upload/v1/TICKET",
            "file_id": "F123",
        }),
        (200, {"ok": False, "error": "not_in_channel"}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(sa, "_public_http_request", lambda *_args, **_kwargs: (200, b""))
    a = _adapter()

    assert a._upload_file_bytes("C01", b"x", "x.txt") is False
    assert len(fake.calls) == 2


def test_upload_file_bytes_rejects_oversize_before_network(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.MAX_UPLOAD_BYTES = 3

    assert a._upload_file_bytes("C01", b"four", "x.txt") is False
    assert fake.calls == []


def test_validate_file_url_rejects_local_and_non_http_targets(monkeypatch):
    def _addresses(host, port, **_kwargs):
        address = "127.0.0.1" if host in {"127.0.0.1", "127.1", "localtest.me"} else "93.184.216.34"
        return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", (address, port))]

    monkeypatch.setattr(sa.socket, "getaddrinfo", _addresses)
    assert sa._validate_file_url("https://example.com/report.pdf") is None
    assert sa._validate_file_url("https://deadbeef/report.pdf") is None
    assert sa._validate_file_url("file:///etc/passwd") is not None
    assert sa._validate_file_url("http://127.0.0.1/private") is not None
    assert sa._validate_file_url("http://127.1/private") is not None
    assert sa._validate_file_url("http://localtest.me/private") is not None
    assert sa._validate_file_url("http://metadata.google.internal/latest") is not None


def test_validate_file_url_rejects_mixed_public_private_dns(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("169.254.169.254", 443)),
        ],
    )

    assert sa._validate_file_url("https://mixed.example/file") is not None


def test_public_http_request_pins_validated_address(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
        ],
    )
    opened = []
    monkeypatch.setattr(
        sa,
        "_request_pinned_once",
        lambda parsed, hostname, target, **kwargs: opened.append(
            (parsed.hostname, hostname, target, kwargs),
        ) or (200, b"file", None),
    )

    assert sa._public_http_request(
        "https://files.example/report.pdf", method="GET", max_bytes=10,
    ) == (200, b"file")
    assert opened[0][2][1][0] == "93.184.216.34"


def test_request_pinned_once_classifies_connect_failure_as_pre_send(monkeypatch):
    request_calls = []

    class _Connection:
        def __init__(self, *_args, **_kwargs):
            pass

        def connect(self):
            raise OSError("no route")

        def request(self, *_args, **_kwargs):
            request_calls.append(True)

        def close(self):
            pass

    monkeypatch.setattr(sa.http.client, "HTTPConnection", _Connection)

    with pytest.raises(sa._PreSendConnectionError, match="no route"):
        sa._request_pinned_once(
            urllib.parse.urlsplit("http://files.example/upload"),
            "files.example",
            (socket.AF_INET, ("93.184.216.34", 80)),
            method="POST",
            body=b"payload",
            headers=None,
            max_bytes=200,
        )
    assert request_calls == []


def test_request_pinned_once_preserves_failure_after_request_started(monkeypatch):
    class _Connection:
        def __init__(self, *_args, **_kwargs):
            self.requested = False

        def connect(self):
            pass

        def request(self, *_args, **_kwargs):
            self.requested = True

        def getresponse(self):
            assert self.requested
            raise http.client.BadStatusLine("response reset")

        def close(self):
            pass

    monkeypatch.setattr(sa.http.client, "HTTPConnection", _Connection)

    with pytest.raises(http.client.BadStatusLine, match="response reset"):
        sa._request_pinned_once(
            urllib.parse.urlsplit("http://files.example/upload"),
            "files.example",
            (socket.AF_INET, ("93.184.216.34", 80)),
            method="POST",
            body=b"payload",
            headers=None,
            max_bytes=200,
        )


def test_public_http_request_post_failover_only_before_send(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.35", 443)),
        ],
    )
    opened = []

    def _request(_parsed, _hostname, target, **_kwargs):
        opened.append(target)
        if len(opened) == 1:
            raise sa._PreSendConnectionError("connect failed")
        return (200, b"uploaded", None)

    monkeypatch.setattr(sa, "_request_pinned_once", _request)

    assert sa._public_http_request(
        "https://files.slack.com/upload/v1/TICKET",
        method="POST",
        body=b"payload",
        max_bytes=200,
        require_https=True,
    ) == (200, b"uploaded")
    assert len(opened) == 2


def test_public_http_request_does_not_replay_post_after_uncertain_failure(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.35", 443)),
        ],
    )
    opened = []

    def _request(_parsed, _hostname, target, **_kwargs):
        opened.append(target)
        raise http.client.BadStatusLine("response reset")

    monkeypatch.setattr(sa, "_request_pinned_once", _request)

    with pytest.raises(RuntimeError, match="refusing to retry POST"):
        sa._public_http_request(
            "https://files.slack.com/upload/v1/TICKET",
            method="POST",
            body=b"payload",
            max_bytes=200,
            require_https=True,
        )
    assert len(opened) == 1


def test_public_http_request_get_can_failover_after_response_failure(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.35", 443)),
        ],
    )
    opened = []

    def _request(_parsed, _hostname, target, **_kwargs):
        opened.append(target)
        if len(opened) == 1:
            raise http.client.BadStatusLine("response reset")
        return (200, b"file", None)

    monkeypatch.setattr(sa, "_request_pinned_once", _request)

    assert sa._public_http_request(
        "https://files.example/report.pdf",
        method="GET",
        max_bytes=200,
    ) == (200, b"file")
    assert len(opened) == 2


def test_public_http_request_rejects_upload_redirect_downgrade(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
        ],
    )
    monkeypatch.setattr(
        sa,
        "_request_pinned_once",
        lambda *_args, **_kwargs: (307, b"", "http://example.com/upload-next"),
    )

    with pytest.raises(RuntimeError, match="HTTPS"):
        sa._public_http_request(
            "https://files.slack.com/upload/v1/TICKET",
            method="POST",
            body=b"payload",
            max_bytes=200,
            require_https=True,
        )


def test_public_http_request_normalizes_malformed_redirect(monkeypatch):
    monkeypatch.setattr(
        sa.socket,
        "getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
        ],
    )
    monkeypatch.setattr(
        sa,
        "_request_pinned_once",
        lambda *_args, **_kwargs: (307, b"", "https://["),
    )

    with pytest.raises(RuntimeError, match="invalid URL"):
        sa._public_http_request(
            "https://files.slack.com/upload/v1/TICKET",
            method="POST",
            body=b"payload",
            max_bytes=200,
            require_https=True,
        )


def test_public_http_request_revalidates_redirect_dns(monkeypatch):
    def _addresses(host, port, **_kwargs):
        address = "169.254.169.254" if host == "redirected.example" else "93.184.216.34"
        return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", (address, port))]

    monkeypatch.setattr(sa.socket, "getaddrinfo", _addresses)
    monkeypatch.setattr(
        sa,
        "_request_pinned_once",
        lambda *_args, **_kwargs: (302, b"", "https://redirected.example/private"),
    )

    with pytest.raises(RuntimeError, match="non-public IP"):
        sa._public_http_request(
            "https://public.example/file",
            method="GET",
            max_bytes=200,
        )


def test_fetch_file_url_enforces_streamed_size_cap(monkeypatch):
    with pytest.raises(RuntimeError, match="upload cap"):
        sa._read_bounded_response(_FakeResp(200, b"four", _HdrShim({})), 3)


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
    a._finalize_pending_reaction("C01", "1700000000.0", "white_check_mark")
    urls = [c["url"] for c in fake.calls]
    assert urls[0].endswith("/reactions.remove")
    assert urls[1].endswith("/reactions.add")
    add_body = json.loads(fake.calls[1]["body_raw"])
    assert add_body["name"] == "white_check_mark"


def test_finalize_pending_reaction_disabled_noop(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_REACTIONS="false")
    a._finalize_pending_reaction("C01", "1700000000.0", "white_check_mark")
    assert fake.calls == []


def test_finalize_pending_reaction_unknown_key_is_noop(monkeypatch):
    # #6731: the "first pending entry in this channel" fallback is gone.
    # A miss must touch nothing — the old fallback flipped an unrelated sibling message's receipt instead, which is exactly what happened on every in-thread reply (the send hook keyed off the thread root).
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._track_pending_reaction("C01", "SIBLING", "eyes")
    a._finalize_pending_reaction("C01", "MINE", "white_check_mark")
    assert fake.calls == []
    assert ("C01", "SIBLING") in a._pending_reactions


def test_finalize_pending_reaction_empty_emoji_removes_only(monkeypatch):
    # The daemon's `clear_done_reaction` knob puts an empty emoji on the terminal frame — remove the eyes, add nothing.
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._track_pending_reaction("C01", "T1", "eyes")
    a._finalize_pending_reaction("C01", "T1", None)
    assert len(fake.calls) == 1
    assert fake.calls[0]["url"].endswith("/reactions.remove")


def test_finalize_pending_reaction_is_idempotent(monkeypatch):
    # A repeated terminal phase must not add a second check: the first call pops the pending entry, the second finds nothing.
    fake = _FakeUrlopen([(200, {"ok": True}), (200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._track_pending_reaction("C01", "T1", "eyes")
    a._finalize_pending_reaction("C01", "T1", "white_check_mark")
    a._finalize_pending_reaction("C01", "T1", "white_check_mark")
    assert len(fake.calls) == 2


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
    fake = _FakeUrlopen([])  # receiving a message issues no HTTP (#6731)
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


def test_receive_adds_no_reaction(monkeypatch):
    # #6731: the eyes used to be added here, on receive.
    # The daemon can still decline the turn afterwards (mention-only group gating, a rate limit, a slash command it handles itself), and nothing ever came back to clear it — so a declined message kept a permanent eyes.
    # The receipt now rides the `queued` lifecycle phase instead, which only fires for a turn that is actually dispatched.
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "UBOT"
    emitted = []
    a._handle_envelope(
        {
            "type": "events_api",
            "envelope_id": "env-gate",
            "payload": {"event": _evt()},
        },
        ws=_FakeWs(),
        emit=emitted.append,
    )
    assert len(emitted) == 1
    assert fake.calls == []
    # Nothing was tracked either, so there is no leaked pending entry a later message's terminal phase could accidentally flip.
    assert a._pending_reactions == {}


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
async def test_on_send_no_longer_touches_reactions(monkeypatch):
    # #6731: the send hook used to finalize the receipt, keyed on `cmd.thread_id` — the thread ROOT ts for an in-thread reply, while the eyes was tracked under the message's own ts.
    # The exact key missed and the deleted fallback flipped an arbitrary sibling instead.
    # Finalization now lives on the lifecycle stream, so `on_send` posts and stops.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # chat.postMessage — the only call expected
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_FORCE_FLAT_REPLIES="true")
    a._track_pending_reaction("C01", "T0", "eyes")
    a._track_pending_reaction("C01", "T1", "eyes")

    class _Cmd:
        channel_id = "C01"
        text = "hi"
        content = {"Text": "hi"}
        thread_id = "T1"
        user = {}

    await a.on_send(_Cmd())
    # The post is flat (force_flat dropped thread_ts)…
    post_body = json.loads(fake.calls[0]["body_raw"])
    assert "thread_ts" not in post_body
    # …and nothing else happened: no reactions call, both entries untouched.
    assert len(fake.calls) == 1
    assert ("C01", "T0") in a._pending_reactions
    assert ("C01", "T1") in a._pending_reactions


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
async def test_on_send_file_data_uploads_to_requested_thread(monkeypatch):
    fake = _FakeUrlopen([
        (200, {
            "ok": True,
            "upload_url": "https://files.slack.com/upload/v1/TICKET",
            "file_id": "F123",
        }),
        (200, {"ok": True, "files": [{"id": "F123"}]}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    uploads = []
    monkeypatch.setattr(
        sa,
        "_public_http_request",
        lambda url, **kwargs: uploads.append((url, kwargs)) or (200, b""),
    )
    a = _adapter()

    class _Cmd:
        channel_id = "C01"
        text = ""
        content = {
            "FileData": {
                "data": [0x50, 0x4B, 0x03, 0x04],
                "filename": "report.xlsx",
                "mime_type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            },
        }
        thread_id = "1700000000.0"
        user = {}

    await a.on_send(_Cmd())

    assert uploads[0][1]["body"] == b"PK\x03\x04"
    complete = fake.calls[1]["body"]
    assert complete["channel_id"] == "C01"
    assert complete["thread_ts"] == "1700000000.0"


@pytest.mark.asyncio
async def test_on_send_file_url_fetches_then_uploads(monkeypatch):
    a = _adapter()
    fetched = []
    uploaded = []
    monkeypatch.setattr(
        a,
        "_fetch_file_url",
        lambda url: fetched.append(url) or b"downloaded",
    )
    monkeypatch.setattr(
        a,
        "_upload_file_bytes",
        lambda channel, data, filename, *, thread_ts=None: uploaded.append(
            (channel, data, filename, thread_ts)
        ) or True,
    )

    class _Cmd:
        channel_id = "C01"
        text = ""
        content = {
            "File": {
                "url": "https://example.com/generated/report.docx",
                "filename": "report.docx",
            },
        }
        thread_id = None
        user = {}

    await a.on_send(_Cmd())

    assert fetched == ["https://example.com/generated/report.docx"]
    assert uploaded == [("C01", b"downloaded", "report.docx", None)]


@pytest.mark.asyncio
async def test_on_send_invalid_file_data_does_not_call_slack(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = "C01"
        text = ""
        content = {
            "FileData": {
                "data": [0, 256],
                "filename": "broken.bin",
                "mime_type": "application/octet-stream",
            },
        }
        thread_id = None
        user = {}

    await a.on_send(_Cmd())

    assert fake.calls == []


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


# ---- Markdown → Slack mrkdwn conversion ----------------------------


def test_mrkdwn_bold():
    assert sa._markdown_to_mrkdwn("**bold** and __also__") == "*bold* and *also*"


def test_mrkdwn_headers_become_bold():
    assert sa._markdown_to_mrkdwn("# Title") == "*Title*"
    assert sa._markdown_to_mrkdwn("### Sub ###") == "*Sub*"


def test_mrkdwn_bullets():
    assert (
        sa._markdown_to_mrkdwn("- one\n* two\n+ three")
        == "•   one\n•   two\n•   three"
    )


def test_mrkdwn_links():
    assert (
        sa._markdown_to_mrkdwn("see [docs](https://x.example/y)")
        == "see <https://x.example/y|docs>"
    )


def test_mrkdwn_strikethrough():
    assert sa._markdown_to_mrkdwn("~~gone~~") == "~gone~"


def test_mrkdwn_preserves_code_spans():
    # ** and ## inside code must survive verbatim.
    src = "inline `a**b` and\n```\n## not a header\n**not bold**\n```"
    assert sa._markdown_to_mrkdwn(src) == src


def test_mrkdwn_passes_through_italic_and_quote():
    # Slack shares `_italic_` and `> quote` syntax with Markdown.
    assert sa._markdown_to_mrkdwn("_em_\n> quote") == "_em_\n> quote"


def test_mrkdwn_bold_wrapping_inline_code():
    # Regression: bold spanning an inline code span must still convert.
    # The old segment-split approach chunked on code delimiters first, so the bold regex never saw both ** markers and they leaked verbatim.
    assert (
        sa._markdown_to_mrkdwn("**the `foo()` function** is broken")
        == "*the `foo()` function* is broken"
    )


def test_mrkdwn_bold_wrapping_multiple_code_spans():
    assert (
        sa._markdown_to_mrkdwn("**a `b` c `d` e**")
        == "*a `b` c `d` e*"
    )


def test_mrkdwn_bold_after_code_span():
    assert (
        sa._markdown_to_mrkdwn("run `pip install x` then **restart**")
        == "run `pip install x` then *restart*"
    )


def test_mrkdwn_header_with_inline_code_stays_one_bold_span():
    # Regression: the old split chopped the header line at the code span, so only the prefix got the line-level bold treatment.
    assert (
        sa._markdown_to_mrkdwn("# Title `code` more")
        == "*Title `code` more*"
    )


def test_mrkdwn_bullet_with_inline_code():
    assert (
        sa._markdown_to_mrkdwn("- run `pip install x` now")
        == "•   run `pip install x` now"
    )


def test_mrkdwn_link_text_with_inline_code():
    assert (
        sa._markdown_to_mrkdwn("[`api` docs](https://e.example)")
        == "<https://e.example|`api` docs>"
    )


def test_mrkdwn_strike_wrapping_inline_code():
    assert sa._markdown_to_mrkdwn("~~old `f()`~~") == "~old `f()`~"


def test_mrkdwn_table_cell_with_inline_code_stays_aligned():
    # Backticks are stripped inside the monospace grid (as before), even though code spans are now masked before table detection runs.
    src = "| A | B |\n|---|---|\n| `x` | 22 |"
    assert sa._markdown_to_mrkdwn(src) == (
        "```\n"
        "A | B \n"
        "--+---\n"
        "x | 22\n"
        "```"
    )


def test_mrkdwn_table_becomes_code_block():
    src = "| A | B |\n|---|---|\n| 1 | 22 |\n| 333 | 4 |"
    assert sa._markdown_to_mrkdwn(src) == (
        "```\n"
        "A   | B \n"
        "----+---\n"
        "1   | 22\n"
        "333 | 4 \n"
        "```"
    )


# ---- blank-line collapsing (#6730) ---------------------------------


def test_markdown_collapses_blank_line_runs():
    # `_convert_md_lines` is a 1:1 line mapper, so a model that pads its answer with blank lines produced a wall of whitespace in Slack.
    assert sa._markdown_to_mrkdwn("a\n\n\n\n\nb") == "a\n\nb"


def test_markdown_preserves_single_blank_line():
    # One blank line is Slack's paragraph separator — collapsing it too would run paragraphs together.
    assert sa._markdown_to_mrkdwn("a\n\nb") == "a\n\nb"
    assert sa._markdown_to_mrkdwn("a\nb") == "a\nb"


def test_markdown_preserves_blank_lines_inside_fenced_code():
    # The collapse runs on the code-MASKED string, where a fenced block is a single token — a naive pre-mask `re.sub` would reflow code.
    src = "intro\n\n```\nx = 1\n\n\n\ny = 2\n```\n\n\n\nouttro"
    out = sa._markdown_to_mrkdwn(src)
    assert "x = 1\n\n\n\ny = 2" in out
    assert out.endswith("```\n\nouttro")


def test_markdown_preserves_blank_lines_inside_an_unclosed_fence():
    # A truncated model response ends mid-block.
    # Slack renders an unterminated ``` as code to the end of the message, so the collapse must not reflow what the user sees as code either.
    src = "intro\n\n\n\n```\nx = 1\n\n\n\ny = 2"
    out = sa._markdown_to_mrkdwn(src)
    assert "x = 1\n\n\n\ny = 2" in out
    # Prose before the fence is still collapsed.
    assert out.startswith("intro\n\n```")


def test_markdown_still_collapses_inside_a_tilde_fence():
    # `~~~` is GitHub-flavoured Markdown that Slack does not render as code, so its contents are prose and the blank-line collapse applies.
    # Pinned so the deliberate boundary is not "fixed" into masking it.
    src = "~~~\na\n\n\n\nb\n~~~"
    assert "a\n\nb" in sa._markdown_to_mrkdwn(src)


def test_markdown_collapses_run_left_by_empty_header():
    # A content-less ATX header (hashes, a space, nothing else) emits an empty line of its own, so the converter can manufacture a blank-line run that was not in the source at all.
    assert sa._markdown_to_mrkdwn("a\n\n## \n\nb") == "a\n\nb"


# ---- interactive section cap (#6730) -------------------------------


def test_build_block_kit_caps_section_text():
    # Slack rejects a `section` over 3000 chars and `_post_message` skips its chunking when blocks are present, so a long interactive reply used to be rejected wholesale and dropped with only a log line.
    blocks = sa._build_block_kit("x" * 9000, [[{"label": "OK", "action": "ok"}]])
    sections = [b for b in blocks if b["type"] == "section"]
    assert len(sections) == 3
    for s in sections:
        assert len(s["text"]["text"]) <= sa.SLACK_MSG_LIMIT
    # The full text survives across the sections.
    assert sum(len(s["text"]["text"]) for s in sections) == 9000
    # Buttons still attach after the text.
    assert blocks[-1]["type"] == "actions"


def test_build_block_kit_never_exceeds_block_limit():
    # Slack also caps a message at 50 blocks.
    # Text is truncated to fit; the buttons — the functional payload — are never dropped.
    buttons = [[{"label": f"B{i}", "action": f"a{i}"}] for i in range(3)]
    blocks = sa._build_block_kit("y" * 400_000, buttons)
    assert len(blocks) <= sa.MAX_BLOCKS_PER_MESSAGE
    assert len([b for b in blocks if b["type"] == "actions"]) == 3
    assert any("truncated" in b.get("text", {}).get("text", "")
               for b in blocks if b["type"] == "section")


def test_build_block_kit_button_rows_alone_exceed_block_cap():
    # When the rows alone reach the cap there is no room for text, so the payload is the marker plus as many rows as still fit.
    # The whole message must stay within the cap: Slack rejects an over-cap message, so emitting 51 blocks to "keep every button" delivers no buttons.
    buttons = [[{"label": f"B{i}", "action": f"a{i}"}]
               for i in range(sa.MAX_BLOCKS_PER_MESSAGE)]
    blocks = sa._build_block_kit("some text", buttons)
    assert len(blocks) <= sa.MAX_BLOCKS_PER_MESSAGE
    sections = [b for b in blocks if b["type"] == "section"]
    # No room for the text itself, just the truncation marker — never a stray real chunk of the text (the unclamped negative-slice bug).
    assert len(sections) == 1
    assert "truncated" in sections[0]["text"]["text"]
    assert len([b for b in blocks if b["type"] == "actions"]) == \
        sa.MAX_BLOCKS_PER_MESSAGE - 1


def test_build_block_kit_keeps_text_when_rows_exactly_fill_the_budget():
    # Regression: the marker slot used to be reserved before checking whether truncation was needed, so at exactly MAX-1 rows the real text was dropped and replaced by "_(message truncated)_" even though one section plus the rows is exactly the cap.
    # `/agents` builds one row per agent with no cap, so a daemon with 49 agents hit this.
    rows = sa.MAX_BLOCKS_PER_MESSAGE - 1
    buttons = [[{"label": f"B{i}", "action": f"a{i}"}] for i in range(rows)]
    blocks = sa._build_block_kit("Select an agent:", buttons)

    assert len(blocks) == sa.MAX_BLOCKS_PER_MESSAGE
    sections = [b for b in blocks if b["type"] == "section"]
    assert len(sections) == 1
    assert sections[0]["text"]["text"] == "Select an agent:"
    assert not any("truncated" in b.get("text", {}).get("text", "")
                   for b in blocks if b["type"] == "section")
    assert len([b for b in blocks if b["type"] == "actions"]) == rows


@pytest.mark.parametrize("rows", [1, 10, 47, 48, 49, 50, 60])
def test_build_block_kit_never_exceeds_cap_for_any_row_count(rows):
    # The cap is a hard Slack limit, so it has to hold across the whole range rather than at the two counts the other tests happen to use.
    buttons = [[{"label": f"B{i}", "action": f"a{i}"}] for i in range(rows)]
    blocks = sa._build_block_kit("some text", buttons)
    assert len(blocks) <= sa.MAX_BLOCKS_PER_MESSAGE, \
        f"{rows} rows produced {len(blocks)} blocks"


def test_post_message_with_blocks_bounds_fallback_text(monkeypatch):
    # With blocks, `text` is only the notification preview — the blocks carry the content.
    # It is bounded rather than chunked, because chunking it would post the same blocks once per chunk.
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    long_text = "z" * 9000
    a._post_message("C01", long_text, blocks=sa._build_block_kit(long_text, []))
    assert len(fake.calls) == 1
    body = json.loads(fake.calls[0]["body_raw"])
    assert len(body["text"]) == sa.SLACK_MSG_LIMIT
    # …while the blocks still carry all 9000 characters.
    assert sum(len(b["text"]["text"]) for b in body["blocks"]) == 9000


def test_build_block_kit_short_text_stays_one_section():
    # No behaviour change for the common case.
    blocks = sa._build_block_kit("Hello", [])
    assert blocks == [
        {"type": "section", "text": {"type": "mrkdwn", "text": "Hello"}},
    ]


@pytest.mark.asyncio
async def test_on_send_converts_markdown_to_mrkdwn(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = "C01"
        text = "## Title\n**bold** [x](https://e.example)"
        content = {"Text": text}
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["text"] == "*Title*\n*bold* <https://e.example|x>"


# ---- multi-step task progress (#6451) ------------------------------


def _reaction(phase, tool_name=None, channel_id="C01", message_id="T1",
              emoji="x"):
    """Build an AgentPhase lifecycle `reaction` command the way the bridge serializes it (channel_id, message_id, emoji, phase, tool_name).

    `emoji` is the wire emoji the daemon computed for the phase.
    Its only load-bearing value is the empty string, which is the `clear_done_reaction` signal on a terminal frame; the adapter maps every other value to a Slack emoji *name* itself, because `reactions.add` takes `white_check_mark`, not the `✅` codepoint.
    """
    return sa.protocol.Reaction(channel_id, message_id, emoji, phase,
                                tool_name)


def test_capabilities_include_reaction():
    # `reaction` is what carries the AgentPhase lifecycle to the adapter;
    # the existing interactive/thread caps stay declared.
    assert "reaction" in sa.SlackAdapter.capabilities
    assert "interactive" in sa.SlackAdapter.capabilities
    assert "thread" in sa.SlackAdapter.capabilities


def test_build_task_progress_blocks_active_and_done():
    steps = [("thinking", None), ("tool_use", "web_fetch")]
    text, blocks = sa._build_task_progress_blocks(steps, "tool_use")
    assert blocks[0]["type"] == "section"
    assert blocks[0]["text"]["type"] == "mrkdwn"
    lines = text.split("\n")
    assert "Working" in lines[0]
    # Completed thinking step gets a checkmark; the active tool step
    # keeps its own (non-check) phase icon and shows the tool name.
    assert lines[1].startswith("✓")
    assert "Thinking" in lines[1]
    assert not lines[2].startswith("✓")
    assert "`web_fetch`" in lines[2]
    # Terminal render flips the header and marks every step done.
    dtext, _ = sa._build_task_progress_blocks(steps, "done")
    assert "Task complete" in dtext.split("\n")[0]
    for ln in dtext.split("\n")[1:]:
        assert ln.startswith("✓")
    etext, _ = sa._build_task_progress_blocks(steps, "error")
    assert "Task failed" in etext.split("\n")[0]


def test_build_task_progress_blocks_bounds_long_step_list():
    # Enough steps that a naive join blows past Slack's 3000-char section limit;
    # the card must stay under the limit by collapsing older steps into a
    # summary line while keeping the most recent ones (#6451 review).
    steps = [("tool_use", f"very_long_tool_name_number_{i:04d}") for i in range(200)]
    text, blocks = sa._build_task_progress_blocks(steps, "tool_use")
    assert len(text) <= sa.SLACK_MSG_LIMIT, (
        f"the progress card must stay under the Slack section limit; got {len(text)}"
    )
    # The rendered block carries exactly the bounded text.
    assert blocks[0]["text"]["text"] == text
    # Older steps are collapsed into a summary line; the newest step survives.
    assert "earlier step" in text
    assert "very_long_tool_name_number_0199" in text


@pytest.mark.asyncio
async def test_phase_single_step_posts_no_card(monkeypatch):
    # A turn that never runs a tool (queued → thinking → done) posts no card — single-step UX stays exactly as before (#6451).
    # It does get the receipt reaction, which is the whole point of #6731: the eyes on `queued`, flipped on `done`.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # reactions.add eyes
        (200, {"ok": True}),  # reactions.remove eyes
        (200, {"ok": True}),  # reactions.add white_check_mark
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("thinking"))
    await a.on_command(_reaction("done"))
    urls = [c["url"] for c in fake.calls]
    assert [u.rsplit("/", 1)[-1] for u in urls] == [
        "reactions.add", "reactions.remove", "reactions.add",
    ]
    # No chat.postMessage / chat.update: no card for a single-step turn.
    assert not any("chat." in u for u in urls)
    # State is cleaned up on the terminal phase.
    assert a._task_progress == {}
    assert a._pending_reactions == {}


@pytest.mark.asyncio
async def test_queued_phase_adds_eyes(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("queued", message_id="TS-Q"))
    assert len(fake.calls) == 1
    assert fake.calls[0]["url"].endswith("/reactions.add")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["name"] == "eyes"
    assert body["timestamp"] == "TS-Q"
    # Tracked so the terminal phase can flip exactly this message.
    assert a._pending_reactions == {("C01", "TS-Q"): "eyes"}
    # `queued` is never rendered, so no card state was materialized —
    # otherwise every turn would look multi-step.
    assert a._task_progress == {}


@pytest.mark.asyncio
async def test_done_phase_flips_to_check(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # eyes
        (200, {"ok": True}),  # remove
        (200, {"ok": True}),  # check
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("done", emoji="✅"))
    assert json.loads(fake.calls[1]["body_raw"])["name"] == "eyes"
    assert fake.calls[1]["url"].endswith("/reactions.remove")
    add_body = json.loads(fake.calls[2]["body_raw"])
    assert add_body["name"] == "white_check_mark"
    assert add_body["timestamp"] == "T1"


@pytest.mark.asyncio
async def test_error_phase_flips_to_x(monkeypatch):
    # A failed turn used to leave the eyes stuck forever — the send hook never ran because there was no reply.
    # It now gets an explicit ❌.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # eyes
        (200, {"ok": True}),  # remove
        (200, {"ok": True}),  # x
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("error", emoji="❌"))
    assert json.loads(fake.calls[2]["body_raw"])["name"] == "x"
    assert a._pending_reactions == {}


@pytest.mark.asyncio
async def test_done_with_empty_reaction_removes_only(monkeypatch):
    # `clear_done_reaction = true` on the daemon side makes the Done frame carry an empty emoji.
    # Slack must lose the eyes and gain nothing.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # eyes
        (200, {"ok": True}),  # remove
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("done", emoji=""))
    assert len(fake.calls) == 2
    assert fake.calls[1]["url"].endswith("/reactions.remove")
    assert a._pending_reactions == {}


@pytest.mark.asyncio
async def test_in_thread_reply_finalizes_own_message_not_a_sibling(
    monkeypatch,
):
    # #6731 regression guard.
    # Two turns in flight in one channel; the terminal phase of the second must touch only the second.
    # The deleted "first pending entry in this channel" fallback flipped the older one.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # eyes on TS-A
        (200, {"ok": True}),  # eyes on TS-B
        (200, {"ok": True}),  # remove on TS-B
        (200, {"ok": True}),  # check on TS-B
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("queued", message_id="TS-A"))
    await a.on_command(_reaction("queued", message_id="TS-B"))
    await a.on_command(_reaction("done", message_id="TS-B", emoji="✅"))
    assert len(fake.calls) == 4
    assert json.loads(fake.calls[2]["body_raw"])["timestamp"] == "TS-B"
    assert json.loads(fake.calls[3]["body_raw"])["timestamp"] == "TS-B"
    # TS-A is still pending — its own turn has not finished yet.
    assert a._pending_reactions == {("C01", "TS-A"): "eyes"}


@pytest.mark.asyncio
async def test_terminal_phase_without_queued_adds_nothing(monkeypatch):
    # Defensive: an adapter that starts mid-turn (restarted sidecar) has no pending entry, so there is no eyes to flip and it must not stamp a bare check onto a message it never marked.
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("done", emoji="✅"))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_phase_multi_step_posts_then_updates_card(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True, "ts": "CARD1"}),  # first tool_use → post
        (200, {"ok": True}),                 # second tool_use → update
        (200, {"ok": True}),                 # done → finalize update
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("thinking"))
    await a.on_command(_reaction("tool_use", tool_name="web_fetch"))
    await a.on_command(_reaction("tool_use", tool_name="read_file"))
    await a.on_command(_reaction("done"))
    assert len(fake.calls) == 3
    assert fake.calls[0]["url"].endswith("/chat.postMessage")
    assert fake.calls[1]["url"].endswith("/chat.update")
    assert fake.calls[2]["url"].endswith("/chat.update")
    # The card threads under the triggering message ts by default.
    post_body = json.loads(fake.calls[0]["body_raw"])
    assert post_body["thread_ts"] == "T1"
    assert any(b["type"] == "section" for b in post_body["blocks"])
    # Updates target the posted card's ts (returned from the first post).
    assert json.loads(fake.calls[1]["body_raw"])["ts"] == "CARD1"
    assert json.loads(fake.calls[2]["body_raw"])["ts"] == "CARD1"
    # Both tool names survive into the finalized card.
    final = json.loads(fake.calls[2]["body_raw"])
    final_text = final["blocks"][0]["text"]["text"]
    assert "web_fetch" in final_text and "read_file" in final_text
    assert "Task complete" in final_text
    # State cleaned up after the terminal phase.
    assert a._task_progress == {}


@pytest.mark.asyncio
async def test_phase_card_force_flat_omits_thread_ts(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True, "ts": "CARD1"}),  # tool_use → post
        (200, {"ok": True}),                 # error → finalize
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_FORCE_FLAT_REPLIES="true")
    await a.on_command(_reaction("thinking"))
    await a.on_command(_reaction("tool_use", tool_name="web_fetch"))
    await a.on_command(_reaction("error"))
    post_body = json.loads(fake.calls[0]["body_raw"])
    assert "thread_ts" not in post_body
    final = json.loads(fake.calls[1]["body_raw"])
    assert "Task failed" in final["blocks"][0]["text"]["text"]


@pytest.mark.asyncio
async def test_phase_card_suppressed_when_reactions_disabled(monkeypatch):
    # SLACK_REACTIONS=false still silences everything, because the card's default follows it.
    # Both indicators off = zero HTTP.
    fake = _FakeUrlopen([])  # no HTTP expected
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_REACTIONS="false")
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("thinking"))
    await a.on_command(_reaction("tool_use", tool_name="web_fetch"))
    await a.on_command(_reaction("done"))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_progress_card_disabled_no_card_post(monkeypatch):
    # #6730: SLACK_PROGRESS_CARD=false silences the card while the receipt keeps working.
    # Before the split, the only way to stop the card was to stop the receipt too.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # eyes
        (200, {"ok": True}),  # remove
        (200, {"ok": True}),  # check
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_PROGRESS_CARD="false")
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("tool_use", tool_name="web_fetch"))
    await a.on_command(_reaction("done", emoji="✅"))
    urls = [c["url"] for c in fake.calls]
    assert not any("chat." in u for u in urls)
    assert [u.rsplit("/", 1)[-1] for u in urls] == [
        "reactions.add", "reactions.remove", "reactions.add",
    ]
    # No card state was ever materialized.
    assert a._task_progress == {}


@pytest.mark.asyncio
async def test_reactions_disabled_still_posts_card(monkeypatch):
    # The other half of the #6730 split: card without emoji noise.
    fake = _FakeUrlopen([
        (200, {"ok": True, "ts": "CARD1"}),  # tool_use -> post
        (200, {"ok": True}),                 # done -> update
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_REACTIONS="false", SLACK_PROGRESS_CARD="true")
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("tool_use", tool_name="web_fetch"))
    await a.on_command(_reaction("done", emoji="✅"))
    urls = [c["url"] for c in fake.calls]
    assert urls[0].endswith("/chat.postMessage")
    assert urls[1].endswith("/chat.update")
    assert not any("reactions." in u for u in urls)


@pytest.mark.asyncio
async def test_card_disabled_still_flips_receipt_on_multi_step(monkeypatch):
    # The receipt half must not depend on the card half's bookkeeping: a multi-step turn with the card off still gets eyes -> check.
    fake = _FakeUrlopen([
        (200, {"ok": True}),  # eyes
        (200, {"ok": True}),  # remove
        (200, {"ok": True}),  # check
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_PROGRESS_CARD="false")
    await a.on_command(_reaction("queued"))
    await a.on_command(_reaction("thinking"))
    await a.on_command(_reaction("tool_use", tool_name="web_fetch"))
    await a.on_command(_reaction("streaming"))
    await a.on_command(_reaction("error", emoji="❌"))
    assert json.loads(fake.calls[2]["body_raw"])["name"] == "x"


@pytest.mark.asyncio
async def test_phase_legacy_emoji_only_reaction_ignored(monkeypatch):
    # A pre-#6451 reaction command with no `phase` carries nothing we
    # can render — it must not post a card.
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_reaction("", emoji="👀"))
    assert fake.calls == []
    assert a._task_progress == {}


@pytest.mark.asyncio
async def test_on_command_send_still_routes_to_on_send(monkeypatch):
    # The on_command override must not break the default Send → on_send
    # dispatch.
    fake = _FakeUrlopen([(200, {"ok": True})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.reactions_enabled = False
    cmd = sa.protocol.Send("C01", "hi", {"Text": "hi"}, None, {})
    await a.on_command(cmd)
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["channel"] == "C01" and body["text"] == "hi"


# ---- display-name resolution (#7086) -------------------------------


def _group_event(user="U1", text="hello"):
    """One inbound group message, parsed the way the envelope handler parses it."""
    return sa.parse_slack_event(
        {"type": "message", "channel": "C0DESIGN", "user": user,
         "text": text, "ts": "1.0"},
        bot_user_id="UBOT",
        allowed_channels=[],
        account_id=None,
        file_policy=sa.SlackFilePolicy(),
    )


def test_users_identity_prefers_the_chosen_display_name():
    identity, err = sa.parse_users_identity({
        "ok": True,
        "user": {"name": "ana", "real_name": "Ana Legal Name",
                 "profile": {"display_name": "Ana", "real_name": "Ana Legal Name"}},
    })
    assert err is None
    assert identity.display_name == "Ana"
    assert identity.username == "ana"


def test_users_identity_walks_the_fallback_ladder():
    # `profile.display_name` is blank for a large share of real accounts, so the
    # ladder is the difference between a name and a regression to the raw id.
    blank_display, _ = sa.parse_users_identity({
        "ok": True,
        "user": {"name": "ana", "real_name": "Ana Legal Name",
                 "profile": {"display_name": "   "}},
    })
    assert blank_display.display_name == "Ana Legal Name"

    handle_only, _ = sa.parse_users_identity({"ok": True, "user": {"name": "ana"}})
    assert handle_only.display_name == "ana"
    assert handle_only.username == "ana"


def test_users_identity_returns_nothing_when_every_name_is_blank():
    identity, err = sa.parse_users_identity({"ok": True, "user": {"profile": {}}})
    assert identity is None
    assert err is None


def test_users_identity_separates_a_definitive_absence_from_a_transient_error():
    # A user who does not exist is an answer worth caching for the full TTL;
    # a rate limit is not.
    absent, err = sa.parse_users_identity({"ok": False, "error": "user_not_found"})
    assert absent is None and err is None
    transient, err = sa.parse_users_identity({"ok": False, "error": "ratelimited"})
    assert transient is None and err == "ratelimited"
    scope, err = sa.parse_users_identity({"ok": False, "error": "missing_scope"})
    assert scope is None and err == "missing_scope"


def test_identity_cache_hit_miss_and_expiry():
    cache = sa._IdentityCache(ttl_secs=3600, max_entries=8)
    assert cache.get("U1") == (False, None)
    cache.put("U1", sa.SlackIdentity(display_name="Ana"))
    hit, identity = cache.get("U1")
    assert hit is True and identity.display_name == "Ana"

    # A cached absence is a hit carrying `None` — that is what stops one doomed
    # lookup per message for a deleted user.
    cache.put("U2", None)
    assert cache.get("U2") == (True, None)

    # An expired entry is indistinguishable from never having been cached.
    cache.put("U3", sa.SlackIdentity(display_name="Bo"), ttl_secs=0)
    assert cache.get("U3") == (False, None)


def test_identity_cache_evicts_oldest_first_at_the_cap():
    cache = sa._IdentityCache(ttl_secs=3600, max_entries=2)
    cache.put("U1", sa.SlackIdentity(display_name="Ana"))
    cache.put("U2", sa.SlackIdentity(display_name="Bo"))
    cache.put("U3", sa.SlackIdentity(display_name="Cy"))
    assert cache.get("U1") == (False, None)
    assert cache.get("U2")[0] is True
    assert cache.get("U3")[0] is True


def test_display_names_are_not_resolved_unless_the_operator_opts_in(monkeypatch):
    # Default OFF: the feature needs the `users:read` scope a pre-#7086 install
    # does not have, and switching it on changes what the daemon persists about
    # real people, not just what it renders.
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a.resolve_display_names is False

    ev = a._apply_identity(_group_event())
    assert ev["params"]["user_name"] == "U1"
    assert fake.calls == []


def test_display_name_replaces_the_raw_id_when_enabled(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True, "user": {"name": "ana", "profile": {"display_name": "Ana"}}}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")

    ev = a._apply_identity(_group_event())
    assert ev["params"]["user_name"] == "Ana"
    # The same request already answered for the handle, and the roster has a
    # column waiting for it.
    assert ev["params"]["metadata"]["sender_username"] == "ana"
    # The user id itself is untouched — DM routing and the `[users]` mapping run
    # on the id, not the name.
    assert ev["params"]["metadata"]["sender_user_id"] == "U1"
    assert len(fake.calls) == 1
    assert fake.calls[0]["url"].endswith("/users.info")
    assert fake.calls[0]["params"]["user"] == "U1"


def test_repeated_ids_cost_exactly_one_users_info_call(monkeypatch):
    # The whole reason the cache exists: `users.info` sits in a tiered per-method
    # rate limit, and a busy channel produces one message per member per minute.
    # The script holds a single response, so a second lookup would fail loudly.
    fake = _FakeUrlopen([
        (200, {"ok": True, "user": {"name": "ana", "profile": {"display_name": "Ana"}}}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")

    names = [
        a._apply_identity(_group_event(text=f"msg {i}"))["params"]["user_name"]
        for i in range(5)
    ]
    assert names == ["Ana"] * 5
    assert len(fake.calls) == 1


def test_distinct_ids_are_resolved_independently(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"ok": True, "user": {"name": "ana", "profile": {"display_name": "Ana"}}}),
        (200, {"ok": True, "user": {"name": "bo", "profile": {"display_name": "Bo"}}}),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")

    assert a._apply_identity(_group_event(user="U1"))["params"]["user_name"] == "Ana"
    assert a._apply_identity(_group_event(user="U2"))["params"]["user_name"] == "Bo"
    # And neither one is re-fetched.
    assert a._apply_identity(_group_event(user="U1"))["params"]["user_name"] == "Ana"
    assert len(fake.calls) == 2


def test_unresolvable_user_keeps_the_raw_id_and_is_not_re_fetched(monkeypatch):
    # Slack has nothing to say about this user. The raw id is a worse label than
    # a real name but a better one than an empty string, and it is exactly what
    # every pre-#7086 deployment already shows — so the path degrades to the old
    # behaviour rather than to a blank sender.
    fake = _FakeUrlopen([(200, {"ok": False, "error": "user_not_found"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")

    for _ in range(3):
        ev = a._apply_identity(_group_event())
        assert ev["params"]["user_name"] == "U1"
        assert "sender_username" not in ev["params"]["metadata"]
    assert len(fake.calls) == 1


def test_missing_scope_does_not_repeat_the_doomed_lookup(monkeypatch):
    # The feature switched on without `users:read` granted: one warning, one
    # request, then the adapter goes back to reporting ids until the cooldown.
    fake = _FakeUrlopen([(200, {"ok": False, "error": "missing_scope"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")

    assert a._apply_identity(_group_event())["params"]["user_name"] == "U1"
    assert a._apply_identity(_group_event())["params"]["user_name"] == "U1"
    assert len(fake.calls) == 1


def test_transient_failure_uses_the_short_cooldown_not_the_full_ttl(monkeypatch):
    # A rate limit that has passed must not keep the whole workspace anonymous
    # for six hours.
    fake = _FakeUrlopen([(200, {"ok": False, "error": "ratelimited"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")
    a._apply_identity(_group_event())

    expires_at, identity = a._identity_cache._entries["U1"]
    assert identity is None
    remaining = expires_at - sa.time.monotonic()
    assert 0 < remaining <= sa.NEGATIVE_TTL_SECS
    assert remaining < a.display_name_ttl


def test_transport_failure_falls_back_to_the_raw_id(monkeypatch):
    def _boom(_req, timeout=None):
        raise OSError("connection reset")

    monkeypatch.setattr(sa.urllib.request, "urlopen", _boom)
    a = _adapter(SLACK_RESOLVE_DISPLAY_NAMES="true")
    ev = a._apply_identity(_group_event())
    assert ev["params"]["user_name"] == "U1"


def test_display_name_ttl_env_is_validated():
    with pytest.raises(SystemExit) as exc:
        _adapter(SLACK_DISPLAY_NAME_TTL="soon")
    assert exc.value.code == 2
    # A non-positive TTL would make the cache useless; fall back to the default.
    a = _adapter(SLACK_DISPLAY_NAME_TTL="0")
    assert a.display_name_ttl == float(sa.DEFAULT_DISPLAY_NAME_TTL_SECS)
    a2 = _adapter(SLACK_DISPLAY_NAME_TTL="60")
    assert a2.display_name_ttl == 60.0


# ---- bulk member enumeration (#7086) -------------------------------


def _members_page(members, cursor=""):
    return (200, {
        "ok": True,
        "members": members,
        "response_metadata": {"next_cursor": cursor},
    })


def test_conversations_members_parses_one_page():
    members, cursor, err = sa.parse_conversations_members(
        {"ok": True, "members": ["U1", "U2"],
         "response_metadata": {"next_cursor": "c2"}})
    assert err is None
    assert members == ["U1", "U2"]
    assert cursor == "c2"


def test_conversations_members_treats_an_empty_cursor_as_the_end():
    # Slack signals "last page" with an empty string, which would otherwise read
    # as "keep paginating from the start" and loop forever.
    _, cursor, err = sa.parse_conversations_members(
        {"ok": True, "members": ["U1"], "response_metadata": {"next_cursor": ""}})
    assert err is None and cursor is None
    _, cursor, _ = sa.parse_conversations_members({"ok": True, "members": ["U1"]})
    assert cursor is None


def test_conversations_members_surfaces_platform_errors():
    members, cursor, err = sa.parse_conversations_members(
        {"ok": False, "error": "missing_scope"})
    assert members == [] and cursor is None and err == "missing_scope"
    _, _, err = sa.parse_conversations_members("not a dict")
    assert err == "non-object response"


def test_conversations_members_drops_non_string_entries():
    members, _, err = sa.parse_conversations_members(
        {"ok": True, "members": ["U1", None, 7, "", "U2"]})
    assert err is None
    assert members == ["U1", "U2"]


def test_member_list_cache_hit_miss_and_expiry():
    cache = sa._MemberListCache(ttl_secs=3600, max_entries=4)
    assert cache.get("C1") == (False, ())
    cache.put("C1", ("U1", "U2"))
    assert cache.get("C1") == (True, ("U1", "U2"))
    # An empty tuple is a real cached answer — a channel the bot cannot read must
    # cost one sweep per cooldown, not one per message.
    cache.put("C2", ())
    assert cache.get("C2") == (True, ())
    cache.put("C3", ("U9",), ttl_secs=0)
    assert cache.get("C3") == (False, ())


def test_member_list_cache_evicts_oldest_first_at_the_cap():
    cache = sa._MemberListCache(ttl_secs=3600, max_entries=2)
    cache.put("C1", ("U1",))
    cache.put("C2", ("U2",))
    cache.put("C3", ("U3",))
    assert cache.get("C1") == (False, ())
    assert cache.get("C2")[0] is True
    assert cache.get("C3")[0] is True


def test_members_are_not_enumerated_unless_the_operator_opts_in(monkeypatch):
    # Default OFF, and for a larger reason than the display-name knob: this
    # changes how many people the daemon stores, from those who addressed the
    # agent to everyone the workspace lists in the channel.
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a.enumerate_members is False

    ev = a._apply_enumerated_members(_group_event())
    assert "group_members" not in ev["params"]["metadata"]
    assert fake.calls == []


def test_enumeration_stamps_group_members_metadata(monkeypatch):
    fake = _FakeUrlopen([_members_page(["U2", "U1"])])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")

    ev = a._apply_enumerated_members(_group_event())
    # Sorted, so the metadata is byte-identical across sweeps that return the
    # same people in a different page order — this list reaches an LLM prompt
    # through `channel_members` (#3298).
    assert ev["params"]["metadata"]["group_members"] == [
        {"user_id": "U1", "display_name": "U1"},
        {"user_id": "U2", "display_name": "U2"},
    ]
    assert fake.calls[0]["url"].endswith("/conversations.members")
    assert fake.calls[0]["params"]["channel"] == "C0DESIGN"


def test_enumeration_paginates_until_the_cursor_runs_out(monkeypatch):
    fake = _FakeUrlopen([
        _members_page(["U1", "U2"], cursor="page2"),
        _members_page(["U3"]),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")

    ev = a._apply_enumerated_members(_group_event())
    ids = [m["user_id"] for m in ev["params"]["metadata"]["group_members"]]
    assert ids == ["U1", "U2", "U3"]
    assert len(fake.calls) == 2
    assert fake.calls[1]["params"]["cursor"] == "page2"


def test_enumeration_stops_at_the_configured_cap(monkeypatch):
    # A general channel in a large workspace lists everyone, and every one of
    # them would become a stored identity row in the daemon's roster.
    fake = _FakeUrlopen([
        _members_page(["U1", "U2", "U3"], cursor="more"),
        _members_page(["U4", "U5"], cursor="more"),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true", SLACK_MEMBER_LIST_MAX="2")

    ev = a._apply_enumerated_members(_group_event())
    ids = [m["user_id"] for m in ev["params"]["metadata"]["group_members"]]
    assert ids == ["U1", "U2"]
    assert len(fake.calls) == 1


def test_repeated_messages_cost_exactly_one_member_sweep(monkeypatch):
    # `conversations.members` is rate-limited per call while membership changes
    # a few times a week; the script holds one page, so a second sweep would
    # fail loudly.
    fake = _FakeUrlopen([_members_page(["U1", "U2"])])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")

    for i in range(4):
        ev = a._apply_enumerated_members(_group_event(text=f"msg {i}"))
        assert len(ev["params"]["metadata"]["group_members"]) == 2
    assert len(fake.calls) == 1


def test_enumeration_skips_direct_messages(monkeypatch):
    # A one-to-one chat has no membership to enumerate, and spending a call to
    # discover that would be one per DM.
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")

    dm = sa.parse_slack_event(
        {"type": "message", "channel": "D0PRIVATE", "user": "U1",
         "text": "hi", "ts": "1.0"},
        bot_user_id="UBOT",
        allowed_channels=[],
        account_id=None,
        file_policy=sa.SlackFilePolicy(),
    )
    ev = a._apply_enumerated_members(dm)
    assert "group_members" not in ev["params"]["metadata"]
    assert fake.calls == []


def test_missing_scope_does_not_repeat_the_doomed_sweep(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": False, "error": "missing_scope"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")

    for _ in range(3):
        ev = a._apply_enumerated_members(_group_event())
        assert "group_members" not in ev["params"]["metadata"]
    assert len(fake.calls) == 1


def test_a_failed_sweep_uses_the_short_cooldown_not_the_full_ttl(monkeypatch):
    fake = _FakeUrlopen([(200, {"ok": False, "error": "ratelimited"})])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")
    a._apply_enumerated_members(_group_event())

    expires_at, members = a._member_list_cache._entries["C0DESIGN"]
    assert members == ()
    remaining = expires_at - sa.time.monotonic()
    assert 0 < remaining <= sa.NEGATIVE_TTL_SECS
    assert remaining < a.member_list_ttl


def test_transport_failure_leaves_the_message_unenumerated(monkeypatch):
    def _boom(_req, timeout=None):
        raise OSError("connection reset")

    monkeypatch.setattr(sa.urllib.request, "urlopen", _boom)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")
    ev = a._apply_enumerated_members(_group_event())
    assert "group_members" not in ev["params"]["metadata"]


def test_enumeration_names_come_from_the_cache_and_never_from_users_info(monkeypatch):
    # A sweep is bulk by nature: resolving 500 members would spend the whole
    # `users.info` budget naming people who have never spoken. Anyone who has
    # spoken is already cached by `_apply_identity`, so in practice the people an
    # agent can act on carry names and the rest carry ids.
    fake = _FakeUrlopen([
        (200, {"ok": True, "user": {"name": "ana", "profile": {"display_name": "Ana"}}}),
        _members_page(["U1", "U2"]),
    ])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true", SLACK_RESOLVE_DISPLAY_NAMES="true")

    ev = a._apply_enumerated_members(a._apply_identity(_group_event(user="U1")))
    assert ev["params"]["metadata"]["group_members"] == [
        {"user_id": "U1", "display_name": "Ana", "username": "ana"},
        {"user_id": "U2", "display_name": "U2"},
    ]
    # Exactly two calls: one `users.info` for the speaker, one members sweep.
    # A third would mean the sweep resolved a name it should not have.
    assert len(fake.calls) == 2


def test_enumeration_records_ids_only_when_display_names_stay_off(monkeypatch):
    # The least-data configuration that still answers "who is in this channel?".
    fake = _FakeUrlopen([_members_page(["U1", "U2"])])
    monkeypatch.setattr(sa.urllib.request, "urlopen", fake)
    a = _adapter(SLACK_ENUMERATE_MEMBERS="true")

    ev = a._apply_enumerated_members(a._apply_identity(_group_event(user="U1")))
    assert ev["params"]["metadata"]["group_members"] == [
        {"user_id": "U1", "display_name": "U1"},
        {"user_id": "U2", "display_name": "U2"},
    ]
    assert len(fake.calls) == 1


def test_member_list_env_is_validated():
    with pytest.raises(SystemExit) as exc:
        _adapter(SLACK_MEMBER_LIST_TTL="soon")
    assert exc.value.code == 2
    with pytest.raises(SystemExit) as exc:
        _adapter(SLACK_MEMBER_LIST_MAX="lots")
    assert exc.value.code == 2
    a = _adapter(SLACK_MEMBER_LIST_TTL="0", SLACK_MEMBER_LIST_MAX="0")
    assert a.member_list_ttl == float(sa.DEFAULT_MEMBER_LIST_TTL_SECS)
    assert a.member_list_max == sa.DEFAULT_MEMBER_LIST_MAX
    a2 = _adapter(SLACK_MEMBER_LIST_TTL="90", SLACK_MEMBER_LIST_MAX="12")
    assert a2.member_list_ttl == 90.0
    assert a2.member_list_max == 12
