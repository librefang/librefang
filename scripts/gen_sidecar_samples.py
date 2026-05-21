#!/usr/bin/env python3
"""Generate the per-adapter `[[sidecar_channels]]` comment blocks
that get pasted into `librefang.toml.example` and
`crates/librefang-cli/templates/init_default_config.toml`.

Source of truth for every block is the adapter's own `SCHEMA`
declaration — pulled via `python3 -m librefang.sidecar.adapters.<name>
--describe`. Keeping the sample SCHEMA-driven means a new adapter
landing (or an existing one rotating an env var) only needs a
re-run of this script; the sample can never silently drift from
the actual env-var contract the adapter enforces at startup.

Usage:

    cd sdk/python
    python3 ../../scripts/gen_sidecar_samples.py > /tmp/blocks.txt

then paste the output between the marker lines in both sample
files and run `cargo xtask schema-check gen` to refresh the
`config.sha256` baseline.

Sanitisation rules — every one is here because the raw SCHEMA
placeholder violated some invariant the sample format requires:

* **Description text**: split on `". "` (period + space) instead
  of just `.` so abbreviations like "Rocket.Chat REST API" don't
  truncate to "Rocket". Trailing `.` from the first sentence is
  dropped to avoid `Name — text..` on render.
* **Display + description**: when the desc is just the display
  name with a generic suffix like "(out-of-process sidecar)",
  drop the suffix — the section header already documents that
  every block in this list is an out-of-process sidecar.
* **Secret values**: render as `"..."` (the dashboard SCHEMA
  hint is free-text prose like "from Settings → Development →
  Your apps" which doesn't belong in a TOML literal). The
  original hint, when meaningful, is preserved as a trailing
  `# <hint>` comment on the same line so operators don't lose
  the semantic guidance.
* **Text-type placeholders** containing `"`: escape via `\\"`.
* **All-optional adapters** (wechat / whatsapp): every env-var
  line is double-commented (`# # KEY = ...  # optional`) plus an
  explicit "Requires at least…" note above the env table — these
  adapters refuse to start with zero config, but the SCHEMA
  marks every var as optional because there are two valid
  configuration paths.
"""
from __future__ import annotations

import json
import subprocess
import sys
from typing import Iterable

ADAPTERS = [
    "bluesky", "dingtalk", "discord", "email", "feishu",
    "google_chat", "gotify", "line", "mastodon", "matrix",
    "mattermost", "nextcloud", "ntfy", "qq", "reddit",
    "rocketchat", "signal", "slack", "teams", "telegram",
    "twitch", "webex", "webhook", "wechat", "wecom",
    "whatsapp", "zulip",
]

# Adapters that replaced an in-process Rust channel in the sidecar
# migration project (#5224 → #5459). The sample now flags these so
# operators upgrading a pre-migration config get a one-line warning
# that their old `[channels.X]` block no longer parses.
MIGRATED_FROM_IN_PROCESS = {
    "bluesky": 5277, "dingtalk": 5417, "discord": 5299,
    "email": 5408, "feishu": 5380, "google_chat": 5459,
    "gotify": 5263, "line": 5312, "mastodon": 5264,
    "matrix": 5368, "mattermost": 5315, "nextcloud": 5301,
    "ntfy": 5224, "qq": 5325, "reddit": 5281, "rocketchat": 5298,
    "signal": 5317, "slack": 5302, "teams": 5433,
    "telegram": 5241, "twitch": 5297, "webex": 5309,
    "webhook": 5455, "wechat": 5421, "wecom": 5392,
    "whatsapp": 5445, "zulip": 5310,
}

# Adapters whose SCHEMA marks every env-var optional but which
# still require operator input to start. A 1-line either/or hint
# above the env table flags the configuration paths.
TWO_PATH_HINTS = {
    "wechat": (
        "Requires either WECHAT_BOT_TOKEN (pre-supplied) OR no "
        "token set (triggers QR-login on first start)."
    ),
    "whatsapp": (
        "Requires EITHER (Cloud API path) WHATSAPP_PHONE_NUMBER_ID "
        "+ WHATSAPP_ACCESS_TOKEN + WHATSAPP_VERIFY_TOKEN OR "
        "(Baileys gateway path) WHATSAPP_GATEWAY_URL pointing at a "
        "running Baileys instance."
    ),
}

# Adapters whose default routes through a public third-party
# service; an explicit privacy/security note is warranted so a
# security-conscious operator doesn't accept the default by
# accident.
PRIVACY_NOTES = {
    "ntfy": (
        "NTFY_SERVER_URL defaults to the PUBLIC ntfy.sh server — "
        "set this to your own server for private notifications."
    ),
}


def fetch(name: str) -> dict:
    """Pull the SCHEMA payload for one adapter via --describe."""
    out = subprocess.check_output(
        ["python3", "-m", f"librefang.sidecar.adapters.{name}",
         "--describe"],
    ).decode()
    return json.loads(out)


def first_sentence(text: str) -> str:
    """Return the first sentence of `text`, splitting on `. `
    (period + space) so abbreviations stay intact. Trailing
    period is stripped — the renderer adds its own punctuation."""
    return (text or "").strip().split(". ")[0].strip().rstrip(".")


def clean_text_placeholder(raw: str) -> str:
    """Strip newlines and escape `"` for safe inclusion in a TOML
    string literal."""
    s = (raw or "").replace("\r", " ").replace("\n", " ").strip()
    return s.replace("\\", "\\\\").replace('"', '\\"')


def render_value(field: dict) -> tuple[str, str]:
    """Return `(value_literal, trailing_hint)` for one env-var.

    Secret values always render as `"..."` so the TOML stays clean.
    The original SCHEMA placeholder, when present and meaningful,
    becomes a trailing `# <hint>` so operators retain the semantic
    guidance (e.g. "the prefix is auto-added" for TWITCH_OAUTH_TOKEN).

    Non-secret values reuse the SCHEMA placeholder verbatim (with
    `"`-escaping); the trailing-hint slot is empty for them — the
    placeholder itself is the hint.
    """
    ph = field.get("placeholder") or ""
    if field.get("type") == "secret":
        # Generic "..." token; preserve the hint when there is one.
        hint = ph.strip()
        if hint and hint not in {"...", '"..."'}:
            return ('"..."', hint)
        return ('"..."', "")
    if not ph:
        return ('"..."', "")
    return (f'"{clean_text_placeholder(ph)}"', "")


def render_env_line(field: dict, *, commented: bool) -> str:
    """One env-var line. `commented=True` adds the leading `# `
    that marks the var as not-required (operator must
    uncomment to set it)."""
    value, hint = render_value(field)
    prefix = "# # " if commented else "# "
    suffix_bits: list[str] = []
    if commented:
        suffix_bits.append("optional")
    if hint:
        suffix_bits.append(hint)
    suffix = f'  # {"; ".join(suffix_bits)}' if suffix_bits else ""
    return f'{prefix}{field["key"]} = {value}{suffix}'


def render(schema: dict) -> str:
    name = schema["name"]
    display = schema.get("display_name") or name
    desc = first_sentence(schema.get("description") or "")
    # Drop the "(out-of-process sidecar)" suffix — the section
    # header already documents that every block in this list is
    # out-of-process; repeating it 27 times is noise.
    if desc.endswith("(out-of-process sidecar)"):
        desc = desc[: -len("(out-of-process sidecar)")].strip()
    desc = desc.rstrip("—-").strip()
    # If the description is just a re-statement of the display
    # name, drop it entirely so we don't render `Bluesky — Bluesky`.
    if desc.lower() == display.lower():
        desc = ""
    header_text = f"{display} — {desc}" if desc else display
    lines = [f"# {header_text}"]
    # Migration warning for sidecars that replaced an in-process
    # channel. The sample needs to surface this prominently so
    # operators upgrading a pre-migration config understand why
    # their old `[channels.X]` block stopped parsing.
    if name in MIGRATED_FROM_IN_PROCESS:
        pr = MIGRATED_FROM_IN_PROCESS[name]
        lines.append(
            f"# Migrated from in-process to sidecar in #{pr}. "
            f"Old `[channels.{name}]` blocks are no longer recognised."
        )
    if name in TWO_PATH_HINTS:
        lines.append(f"# {TWO_PATH_HINTS[name]}")
    if name in PRIVACY_NOTES:
        lines.append(f"# {PRIVACY_NOTES[name]}")
    lines.extend([
        "# [[sidecar_channels]]",
        f'# name = "{name}"',
        '# command = "python3"',
        f'# args = ["-m", "librefang.sidecar.adapters.{name}"]',
        f'# channel_type = "{name}"',
        "# [sidecar_channels.env]",
    ])
    req = [f for f in schema["fields"] if f.get("required")]
    opt = [f for f in schema["fields"] if not f.get("required")]
    for f in req:
        lines.append(render_env_line(f, commented=False))
    # Show up to 4 optional vars when there are no required (so the
    # env block isn't visually empty); otherwise show up to 2
    # representative optionals.
    cap = 4 if not req else 2
    shown = 0
    for f in opt:
        if shown >= cap:
            break
        # Skip optionals that have no placeholder AND no hint —
        # they'd render as `# # X = "..."  # optional` with zero
        # signal about what to put there.
        if not f.get("placeholder") and f.get("type") != "secret":
            continue
        lines.append(render_env_line(f, commented=True))
        shown += 1
    return "\n".join(lines) + "\n"


def main(adapters: Iterable[str] = ADAPTERS) -> None:
    sys.stdout.write(
        "# ── Sidecar channel adapters (one block per channel) ─────────\n"
        "# All channel adapters now run as out-of-process Python sidecars\n"
        "# speaking newline-delimited JSON-RPC over stdio. Each block below\n"
        "# is independent — uncomment only the adapters you need. Secrets\n"
        "# belong in ~/.librefang/secrets.env, NOT here. The full env-var\n"
        "# inventory for an adapter lives in its SCHEMA — run\n"
        "#   python3 -m librefang.sidecar.adapters.<name> --describe\n"
        "# to see every supported knob including optional and advanced\n"
        "# fields. Regenerate this listing with\n"
        "#   python3 scripts/gen_sidecar_samples.py\n"
        "# from `sdk/python/` after a SCHEMA change.\n"
        "\n"
    )
    for name in adapters:
        sys.stdout.write(render(fetch(name)))
        sys.stdout.write("\n")


if __name__ == "__main__":
    main()
