"""Wire protocol for LibreFang sidecar channel adapters.

Newline-delimited JSON over stdio:

* **Events** (adapter -> LibreFang, written to **stdout**): ``ready``,
  ``message``, ``error``, ``typing``.
* **Commands** (LibreFang -> adapter, read from **stdin**): ``send``,
  ``ready_ack``, ``typing``, ``reaction``, ``interactive``,
  ``stream_start``, ``stream_delta``, ``stream_end``, ``heartbeat``,
  ``shutdown``.

The event/command envelope targets the **post-#5219** sidecar protocol
(the P0–P3 channel-parity work). It is NOT the protocol on ``main``:
``main``'s Rust ``SidecarEvent`` is the minimal ``message`` / ``ready``
(unit, no params) / ``error`` set with ``SidecarMessageParams`` =
``{user_id, user_name, text, channel_id, platform}`` (no ``content``),
and ``SidecarCommand`` is only ``send {channel_id, text}`` /
``shutdown``. An adapter written with this SDK is fully wire-correct
only once #5219 lands, but it **degrades cleanly on the current
``main``**: a pre-#5219 daemon never sends ``ready_ack``, so the
handshake loop stops re-announcing after ``ready_max_attempts`` (see
:mod:`librefang.sidecar.runtime`) rather than flooding stdout; and
:func:`message` mirrors plain-text ``content`` into the legacy
``text`` field so a ``content``-only text message is still delivered
non-empty (non-text ``content`` has no lossless ``text`` form and is
seen only by a #5219+ daemon). ``content`` is the externally-tagged
``ChannelContent`` enum (e.g. ``{"Text": "hi"}``, ``{"Image": {...}}``)
— this part IS byte-for-byte the existing
``crate::types::ChannelContent``. Build it with :class:`Content`.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Union

# --------------------------------------------------------------------------
# ChannelContent — externally tagged, mirrors crate::types::ChannelContent
# --------------------------------------------------------------------------


class Content:
    """Builders for every ``ChannelContent`` variant.

    Each returns the externally-tagged dict the wire expects. Optional
    fields are emitted as ``null`` only where the Rust side keeps them
    (it uses ``#[serde(default)]`` / ``skip_serializing_if`` per field);
    sending the explicit key is always accepted.
    """

    @staticmethod
    def text(s: str) -> Dict[str, Any]:
        return {"Text": s}

    @staticmethod
    def image(url: str, caption: Optional[str] = None,
              mime_type: Optional[str] = None) -> Dict[str, Any]:
        return {"Image": {"url": url, "caption": caption,
                          "mime_type": mime_type}}

    @staticmethod
    def file(url: str, filename: str) -> Dict[str, Any]:
        return {"File": {"url": url, "filename": filename}}

    @staticmethod
    def file_data(data: bytes, filename: str,
                  mime_type: str) -> Dict[str, Any]:
        """Inline bytes. Mirrors Rust ``FileData{data: Vec<u8>}``: serde
        serialises the bytes as a JSON number array (there is no base64
        shim — see P1). Prefer :meth:`file` with a URL for large
        payloads."""
        return {"FileData": {"data": list(data), "filename": filename,
                             "mime_type": mime_type}}

    @staticmethod
    def voice(url: str, caption: Optional[str] = None,
              duration_seconds: int = 0) -> Dict[str, Any]:
        return {"Voice": {"url": url, "caption": caption,
                          "duration_seconds": duration_seconds}}

    @staticmethod
    def video(url: str, caption: Optional[str] = None,
              duration_seconds: int = 0,
              filename: Optional[str] = None) -> Dict[str, Any]:
        return {"Video": {"url": url, "caption": caption,
                          "duration_seconds": duration_seconds,
                          "filename": filename}}

    @staticmethod
    def location(lat: float, lon: float) -> Dict[str, Any]:
        return {"Location": {"lat": lat, "lon": lon}}

    @staticmethod
    def command(name: str, args: List[str]) -> Dict[str, Any]:
        return {"Command": {"name": name, "args": list(args)}}

    @staticmethod
    def interactive(text: str,
                    buttons: List[List[Dict[str, Any]]]) -> Dict[str, Any]:
        return {"Interactive": {"text": text, "buttons": buttons}}

    @staticmethod
    def button_callback(action: str,
                        message_text: Optional[str] = None) -> Dict[str, Any]:
        return {"ButtonCallback": {"action": action,
                                   "message_text": message_text}}

    @staticmethod
    def delete_message(message_id: str) -> Dict[str, Any]:
        return {"DeleteMessage": {"message_id": message_id}}

    @staticmethod
    def edit_interactive(message_id: str, text: str,
                         buttons: List[List[Dict[str, Any]]]
                         ) -> Dict[str, Any]:
        return {"EditInteractive": {"message_id": message_id, "text": text,
                                    "buttons": buttons}}

    @staticmethod
    def audio(url: str, caption: Optional[str] = None,
              duration_seconds: int = 0, title: Optional[str] = None,
              performer: Optional[str] = None) -> Dict[str, Any]:
        payload: Dict[str, Any] = {"url": url, "caption": caption,
                                   "duration_seconds": duration_seconds}
        if title is not None:
            payload["title"] = title
        if performer is not None:
            payload["performer"] = performer
        return {"Audio": payload}

    @staticmethod
    def animation(url: str, caption: Optional[str] = None,
                  duration_seconds: int = 0) -> Dict[str, Any]:
        return {"Animation": {"url": url, "caption": caption,
                              "duration_seconds": duration_seconds}}

    @staticmethod
    def sticker(file_id: str) -> Dict[str, Any]:
        return {"Sticker": {"file_id": file_id}}

    @staticmethod
    def media_group(items: List[Dict[str, Any]]) -> Dict[str, Any]:
        return {"MediaGroup": {"items": items}}

    @staticmethod
    def poll(question: str, options: List[str], is_quiz: bool = False,
             correct_option_id: Optional[int] = None,
             explanation: Optional[str] = None) -> Dict[str, Any]:
        payload: Dict[str, Any] = {"question": question,
                                   "options": list(options),
                                   "is_quiz": is_quiz}
        if correct_option_id is not None:
            payload["correct_option_id"] = correct_option_id
        if explanation is not None:
            payload["explanation"] = explanation
        return {"Poll": payload}

    @staticmethod
    def poll_answer(poll_id: str, option_ids: List[int]) -> Dict[str, Any]:
        return {"PollAnswer": {"poll_id": poll_id,
                               "option_ids": list(option_ids)}}

    @staticmethod
    def button(label: str, action: str, style: Optional[str] = None,
               url: Optional[str] = None) -> Dict[str, Any]:
        """An ``InteractiveButton`` for use in :meth:`interactive`."""
        b: Dict[str, Any] = {"label": label, "action": action}
        if style is not None:
            b["style"] = style
        if url is not None:
            b["url"] = url
        return b


# --------------------------------------------------------------------------
# Outbound events (adapter -> LibreFang, stdout)
# --------------------------------------------------------------------------


def ready(capabilities: Optional[List[str]] = None,
          account_id: Optional[str] = None,
          suppress_error_responses: bool = False,
          notification_recipients: Optional[List[Dict[str, Any]]] = None,
          header_rules: Optional[List[Any]] = None,
          protocol_version: Optional[int] = None) -> Dict[str, Any]:
    """Build a ``ready`` event declaring the adapter's capabilities."""
    return {
        "method": "ready",
        "params": {
            "capabilities": list(capabilities or []),
            "account_id": account_id,
            "suppress_error_responses": suppress_error_responses,
            "notification_recipients": notification_recipients or [],
            "header_rules": header_rules or [],
            "protocol_version": protocol_version,
        },
    }


def message(user_id: str, user_name: str, *, text: Optional[str] = None,
            content: Optional[Dict[str, Any]] = None,
            message_id: Optional[str] = None,
            channel_id: Optional[str] = None,
            platform: Optional[str] = None,
            username: Optional[str] = None,
            librefang_user: Optional[str] = None,
            is_group: bool = False, thread_id: Optional[str] = None,
            group_members: Optional[List[Dict[str, Any]]] = None,
            group_participants: Optional[List[Dict[str, Any]]] = None,
            metadata: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    """Build a ``message`` event. ``content`` (a :class:`Content`
    result) supersedes ``text``; legacy text-only adapters may pass
    only ``text``. ``message_id`` is the platform's *native* id —
    supply it so reactions/edits target the real message instead of a
    server-generated UUID (the #5219+ daemon stores it as
    ``platform_message_id``)."""
    params: Dict[str, Any] = {"user_id": user_id, "user_name": user_name}
    if text is not None:
        params["text"] = text
    if message_id is not None:
        params["message_id"] = message_id
    if content is not None:
        params["content"] = content
        # Back-compat: a pre-#5219 daemon ignores `content` and reads
        # only `text`, so a content-only plain-text message would be
        # delivered empty. Mirror a `Content.text(...)` into `text`
        # (post-#5219 the daemon prefers `content`, so this is
        # harmless redundancy). Non-text content can't be flattened
        # losslessly and is left for the #5219+ daemon.
        if (text is None and isinstance(content, dict)
                and isinstance(content.get("Text"), str)):
            params["text"] = content["Text"]
    if channel_id is not None:
        params["channel_id"] = channel_id
    if platform is not None:
        params["platform"] = platform
    if username is not None:
        params["username"] = username
    if librefang_user is not None:
        params["librefang_user"] = librefang_user
    if is_group:
        params["is_group"] = True
    if thread_id is not None:
        params["thread_id"] = thread_id
    if group_members:
        params["group_members"] = group_members
    if group_participants:
        params["group_participants"] = group_participants
    if metadata:
        params["metadata"] = metadata
    return {"method": "message", "params": params}


def error(msg: str) -> Dict[str, Any]:
    return {"method": "error", "params": {"message": msg}}


def typing_event(user_id: str, user_name: str,
                 is_typing: bool) -> Dict[str, Any]:
    return {"method": "typing",
            "params": {"user_id": user_id, "user_name": user_name,
                       "is_typing": is_typing}}


def qr_ready(qr_code: str, *, qr_url: Optional[str] = None,
             message: Optional[str] = None,
             expires_at: Optional[str] = None) -> Dict[str, Any]:
    """Build a ``qr_ready`` event so the daemon can cache the QR for
    the dashboard's ``GET /api/channels/{name}/qr`` projection.

    Emit ONCE per QR-login session, immediately after the platform
    returns a scannable payload. Follow up with :func:`qr_status`
    transitions as the user scans / confirms / the code expires.

    ``qr_code`` is the raw payload the user's phone scanner reads.
    ``qr_url`` is an optional pre-formed deep-link the platform's app
    recognises; when absent the dashboard renders ``qr_code`` to a
    canvas itself. ``expires_at`` is an ISO 8601 timestamp; supply it
    when the platform tells you the window (e.g. WeChat's 5 min)."""
    params: Dict[str, Any] = {"qr_code": qr_code}
    if qr_url is not None:
        params["qr_url"] = qr_url
    if message is not None:
        params["message"] = message
    if expires_at is not None:
        params["expires_at"] = expires_at
    return {"method": "qr_ready", "params": params}


def qr_status(status: str, *,
              message: Optional[str] = None) -> Dict[str, Any]:
    """Build a ``qr_status`` event reporting a QR-login transition.

    ``status`` is one of ``"pending"``, ``"scanning"``, ``"confirmed"``,
    ``"expired"``, ``"failed"``. The daemon coerces unknown values to
    ``"failed"`` so a misspelled status can't strand the dashboard in
    ``"pending"`` forever. ``message`` is operator-visible — populate
    it on ``"expired"`` / ``"failed"`` with the reason, on
    ``"confirmed"`` with any follow-up hint (e.g. \"set
    WECHAT_BOT_TOKEN in secrets.env to skip QR next time\").

    The event intentionally does NOT carry the captured credential.
    The current sidecar-configure endpoint is a full-form upsert that
    would wipe any other already-set schema fields on a partial save,
    so auto-persist via the dashboard is unsafe until a narrow
    ``/api/channels/sidecar/{name}/secrets`` endpoint exists. Until
    then, sidecars log the token at DEBUG and the operator copies it
    into ``secrets.env`` by hand."""
    params: Dict[str, Any] = {"status": status}
    if message is not None:
        params["message"] = message
    return {"method": "qr_status", "params": params}


# --------------------------------------------------------------------------
# Inbound commands (LibreFang -> adapter, stdin)
# --------------------------------------------------------------------------


@dataclass
class Send:
    channel_id: str
    text: str
    content: Optional[Dict[str, Any]]
    thread_id: Optional[str]
    user: Dict[str, Any]


@dataclass
class ReadyAck:
    pass


@dataclass
class Shutdown:
    pass


@dataclass
class TypingCmd:
    channel_id: str


@dataclass
class Reaction:
    channel_id: str
    message_id: str
    reaction: str


@dataclass
class Interactive:
    channel_id: str
    message: Dict[str, Any]


@dataclass
class StreamStart:
    channel_id: str
    stream_id: str
    #: Thread to stream the reply into; ``None`` for a top-level reply.
    #: The #5219+ daemon sends this for threaded streamed replies.
    thread_id: Optional[str] = None


@dataclass
class StreamDelta:
    stream_id: str
    text: str


@dataclass
class StreamEnd:
    stream_id: str


@dataclass
class Heartbeat:
    pass


@dataclass
class UnknownCommand:
    """Forward-compat: a command method this SDK version doesn't model.
    Adapters must tolerate these rather than crash (the daemon relies on
    that symmetry — e.g. it sends ``ready_ack`` to older adapters)."""
    method: str
    raw: Dict[str, Any]


Command = Union[
    Send, ReadyAck, Shutdown, TypingCmd, Reaction, Interactive,
    StreamStart, StreamDelta, StreamEnd, Heartbeat, UnknownCommand,
]


def parse_command(line: str) -> Command:
    """Parse one stdin line into a typed command. Raises
    ``json.JSONDecodeError`` on malformed JSON *and* on syntactically
    valid JSON that is not an object (e.g. a bare number/array) — the
    reader treats both the same: emit a protocol error and continue,
    rather than letting an ``AttributeError`` escape and wedge the
    adapter. Unknown methods become :class:`UnknownCommand`."""
    obj = json.loads(line)
    if not isinstance(obj, dict):
        raise json.JSONDecodeError(
            "expected a JSON object", line, 0)
    method = obj.get("method")
    p = obj.get("params") or {}
    if method == "send":
        return Send(p.get("channel_id", ""), p.get("text", ""),
                    p.get("content"), p.get("thread_id"),
                    p.get("user", {}))
    if method == "ready_ack":
        return ReadyAck()
    if method == "shutdown":
        return Shutdown()
    if method == "typing":
        return TypingCmd(p.get("channel_id", ""))
    if method == "reaction":
        return Reaction(p.get("channel_id", ""), p.get("message_id", ""),
                        p.get("reaction", ""))
    if method == "interactive":
        return Interactive(p.get("channel_id", ""), p.get("message", {}))
    if method == "stream_start":
        return StreamStart(p.get("channel_id", ""), p.get("stream_id", ""),
                           p.get("thread_id"))
    if method == "stream_delta":
        return StreamDelta(p.get("stream_id", ""), p.get("text", ""))
    if method == "stream_end":
        return StreamEnd(p.get("stream_id", ""))
    if method == "heartbeat":
        return Heartbeat()
    return UnknownCommand(method or "", obj)


# ---------------------------------------------------------------------------
# Self-description schema — emitted by `python -m <adapter> --describe`.
# Mirrors the FieldType enum in librefang-api's CHANNEL_REGISTRY so the
# dashboard can render either kind of channel with one form component.
# ---------------------------------------------------------------------------

_ALLOWED_FIELD_TYPES = {"text", "secret", "number", "list", "bool", "select"}


class Field:
    """One configurable field for a sidecar adapter.

    `type=secret` is routed to ~/.librefang/secrets.env on save (never
    written to config.toml). Every other type is stored in the
    [sidecar_channels.env] table of config.toml — these are the env
    vars the child process reads via os.environ on startup.
    """

    __slots__ = ("key", "label", "type", "required", "placeholder",
                 "advanced", "options")

    def __init__(self, key, label, type, *, required=False,
                 placeholder="", advanced=False, options=None):
        if type not in _ALLOWED_FIELD_TYPES:
            raise ValueError(
                f"unknown field type {type!r}; "
                f"allowed: {sorted(_ALLOWED_FIELD_TYPES)}"
            )
        self.key = key
        self.label = label
        self.type = type
        self.required = required
        self.placeholder = placeholder
        self.advanced = advanced
        self.options = options  # for type=select

    def to_dict(self):
        d = {
            "key": self.key,
            "label": self.label,
            "type": self.type,
            "required": self.required,
            "placeholder": self.placeholder,
            "advanced": self.advanced,
        }
        if self.options is not None:
            d["options"] = list(self.options)
        return d


class Schema:
    """Self-description payload emitted by `<adapter> --describe`."""

    __slots__ = ("name", "display_name", "description", "fields")

    def __init__(self, name, display_name, description, fields):
        self.name = name
        self.display_name = display_name
        self.description = description
        self.fields = list(fields)

    def to_dict(self):
        return {
            "name": self.name,
            "display_name": self.display_name,
            "description": self.description,
            "fields": [f.to_dict() for f in self.fields],
        }
