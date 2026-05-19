"""Schema-shape contract tests for the sidecar self-description protocol."""
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
    import pytest
    with pytest.raises(ValueError, match="unknown field type"):
        Field("X", "X", "magic")
