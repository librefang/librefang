"""Schema-shape contract tests for the sidecar self-description protocol."""
import pytest

from librefang.sidecar.protocol import Field, Schema


def test_field_secret_required():
    f = Field("TELEGRAM_BOT_TOKEN", "Bot Token", "secret",
              required=True, placeholder="123:ABC...")
    assert f.to_dict() == {
        "key": "TELEGRAM_BOT_TOKEN",
        "label": "Bot Token",
        "type": "secret",
        "required": True,
        "placeholder": "123:ABC...",
        "advanced": False,
    }


def test_field_advanced_list():
    f = Field("ALLOWED_USERS", "Allowed User IDs", "list", advanced=True)
    assert f.to_dict()["advanced"] is True
    assert f.to_dict()["type"] == "list"
    assert f.to_dict()["required"] is False  # default


def test_schema_serializes_fields():
    s = Schema(
        name="telegram",
        display_name="Telegram",
        description="Telegram Bot API adapter",
        fields=[
            Field("TELEGRAM_BOT_TOKEN", "Bot Token", "secret", required=True),
            Field("ALLOWED_USERS", "Allowed User IDs", "list"),
        ],
    )
    out = s.to_dict()
    assert out["name"] == "telegram"
    assert len(out["fields"]) == 2
    assert out["fields"][0]["key"] == "TELEGRAM_BOT_TOKEN"
    assert out["fields"][0]["type"] == "secret"


def test_field_rejects_unknown_type():
    with pytest.raises(ValueError, match="unknown field type"):
        Field("X", "X", "magic")


def test_field_select_serializes_copied_options():
    options = ["en", "zh"]
    field = Field("LANG", "Language", "select", options=options)

    serialized = field.to_dict()

    assert serialized["type"] == "select"
    assert serialized["options"] == ["en", "zh"]
    assert serialized["options"] is not options


def test_schema_reports_the_sdk_version():
    """`--describe` is the only path an adapter's SDK version has to the daemon.

    #7140 was a deployment running a four-month-old SDK against a current
    daemon; `librefang.__version__` existed the whole time and was on no wire.
    `--describe` resolves the same interpreter and PYTHONPATH as the eventual
    spawn, so the version it reports is the version that will serve traffic.
    """
    from librefang import __version__

    s = Schema(
        name="telegram",
        display_name="Telegram",
        description="Telegram Bot API adapter",
        fields=[],
    )
    out = s.to_dict()
    assert out["sdk_version"] == __version__
    # Emitted for every adapter, not declared per adapter — a version that has
    # to be opted into is a version that stays unset (the shape of #7140).
    assert out["sdk_version"]
