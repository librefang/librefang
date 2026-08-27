//! Rust side of the sidecar protocol conformance suite.
//!
//! The shared corpus (`conformance/sidecar/corpus/`) is the single
//! oracle for the protocol's two implementations — this crate's
//! `SidecarEvent`/`SidecarCommand` and the Python SDK's
//! `protocol.py`. The Python half lives in
//! `sdk/python/tests/test_sidecar_conformance.py` and asserts against
//! the same files; drift on either side fails its own conformance run.
//!
//! Directionality (see `conformance/sidecar/README.md`):
//! * **events** are produced by adapters and *consumed* here — Rust is
//!   the deserializer, so we assert every corpus event parses into the
//!   expected `SidecarEvent` variant.
//! * **commands** are *produced* here — Rust is the serializer, so we
//!   assert each `SidecarCommand` serializes to the corpus JSON value.
//!
//! Equality is structural JSON value equality, not byte equality.

use librefang_channels::sidecar::{
    classify_protocol_version, ProtocolSkew, SidecarCommand, SidecarEvent,
    SidecarInteractiveParams, SidecarReactionParams, SidecarSendParams, SidecarStreamDeltaParams,
    SidecarStreamEndParams, SidecarStreamStartParams, SidecarTypingCmdParams,
    SIDECAR_PROTOCOL_VERSION,
};
use librefang_channels::types::{
    ChannelContent, ChannelUser, InteractiveButton, InteractiveMessage,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/sidecar/corpus")
}

fn read_corpus(rel: &str) -> Value {
    let path = corpus_dir().join(rel);
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read corpus {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse corpus {}: {e}", path.display()))
}

fn list_json(subdir: &str) -> Vec<String> {
    let dir = corpus_dir().join(subdir);
    let mut out: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("read entry in {}: {e}", dir.display()))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|n| n.ends_with(".json"))
        .collect();
    out.sort();
    out
}

#[test]
fn corpus_directory_exists() {
    let dir = corpus_dir();
    assert!(
        dir.is_dir(),
        "sidecar conformance corpus directory is missing: {}",
        dir.display()
    );
}

/// Every corpus file is a JSON object with a string `method`.
#[test]
fn corpus_files_are_well_formed() {
    for sub in ["events", "commands"] {
        let names = list_json(sub);
        assert!(!names.is_empty(), "no corpus files under {sub}/");
        for name in names {
            let v = read_corpus(&format!("{sub}/{name}"));
            let obj = v
                .as_object()
                .unwrap_or_else(|| panic!("{sub}/{name}: not a JSON object"));
            assert!(
                obj.get("method").and_then(Value::as_str).is_some(),
                "{sub}/{name}: missing string `method`"
            );
        }
    }
}

fn optional_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn value_or(params: &Value, key: &str, default: Value) -> Value {
    params.get(key).cloned().unwrap_or(default)
}

fn assert_event_matches_fixture(name: &str, fixture: &Value, event: &SidecarEvent) {
    let method = fixture["method"]
        .as_str()
        .unwrap_or_else(|| panic!("events/{name}: missing method"));
    let params = fixture.get("params").unwrap_or(&Value::Null);

    match event {
        SidecarEvent::Message { params: actual } => {
            assert_eq!(method, "message", "events/{name}: variant mismatch");
            assert_eq!(actual.user_id, params["user_id"]);
            assert_eq!(actual.user_name, params["user_name"]);
            assert_eq!(actual.text.as_deref(), optional_str(params, "text"));
            assert_eq!(
                actual.channel_id.as_deref(),
                optional_str(params, "channel_id")
            );
            assert_eq!(actual.platform.as_deref(), optional_str(params, "platform"));
            assert_eq!(
                actual.message_id.as_deref(),
                optional_str(params, "message_id")
            );
            assert_eq!(
                serde_json::to_value(&actual.content).unwrap(),
                value_or(params, "content", Value::Null)
            );
            assert_eq!(actual.username.as_deref(), optional_str(params, "username"));
            assert_eq!(
                actual.librefang_user.as_deref(),
                optional_str(params, "librefang_user")
            );
            assert_eq!(
                actual.is_group,
                params
                    .get("is_group")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            );
            assert_eq!(
                actual.thread_id.as_deref(),
                optional_str(params, "thread_id")
            );
            assert_eq!(
                serde_json::to_value(&actual.group_members).unwrap(),
                value_or(params, "group_members", serde_json::json!([]))
            );
            assert_eq!(
                serde_json::to_value(&actual.group_participants).unwrap(),
                value_or(params, "group_participants", serde_json::json!([]))
            );
            assert_eq!(
                serde_json::to_value(&actual.metadata).unwrap(),
                value_or(params, "metadata", serde_json::json!({}))
            );
        }
        SidecarEvent::Ready { params: actual } => {
            assert_eq!(method, "ready", "events/{name}: variant mismatch");
            assert_eq!(
                serde_json::to_value(&actual.capabilities).unwrap(),
                value_or(params, "capabilities", serde_json::json!([]))
            );
            assert_eq!(
                actual.account_id.as_deref(),
                optional_str(params, "account_id")
            );
            assert_eq!(
                actual.suppress_error_responses,
                params
                    .get("suppress_error_responses")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            );
            assert_eq!(
                serde_json::to_value(&actual.notification_recipients).unwrap(),
                value_or(params, "notification_recipients", serde_json::json!([]))
            );
            assert_eq!(
                serde_json::to_value(&actual.header_rules).unwrap(),
                value_or(params, "header_rules", serde_json::json!([]))
            );
            assert_eq!(
                actual.protocol_version,
                params
                    .get("protocol_version")
                    .and_then(Value::as_u64)
                    .map(|version| version as u32)
            );
        }
        SidecarEvent::Error { params: actual } => {
            assert_eq!(method, "error", "events/{name}: variant mismatch");
            assert_eq!(actual.message, params["message"]);
        }
        SidecarEvent::Typing { params: actual } => {
            assert_eq!(method, "typing", "events/{name}: variant mismatch");
            assert_eq!(actual.user_id, params["user_id"]);
            assert_eq!(actual.user_name, params["user_name"]);
            assert_eq!(actual.is_typing, params["is_typing"]);
        }
        SidecarEvent::QrReady { params: actual } => {
            assert_eq!(method, "qr_ready", "events/{name}: variant mismatch");
            assert_eq!(actual.qr_code, params["qr_code"]);
            assert_eq!(actual.qr_url.as_deref(), optional_str(params, "qr_url"));
            assert_eq!(actual.message.as_deref(), optional_str(params, "message"));
            assert_eq!(
                serde_json::to_value(actual.expires_at).unwrap(),
                value_or(params, "expires_at", Value::Null)
            );
        }
        SidecarEvent::QrStatus { params: actual } => {
            assert_eq!(method, "qr_status", "events/{name}: variant mismatch");
            assert_eq!(actual.status, params["status"]);
            assert_eq!(actual.message.as_deref(), optional_str(params, "message"));
        }
    }
}

/// Consumer side: every event variant has a fixture and every fixture deserializes into the exact fields it declares.
#[test]
fn events_deserialize_into_expected_variant() {
    let expected = [
        "error.json",
        "message_command.json",
        "message_minimal.json",
        "message_text.json",
        "qr_ready.json",
        "qr_status.json",
        "ready_full.json",
        "ready_minimal.json",
        "typing.json",
    ];
    assert_eq!(
        list_json("events"),
        expected,
        "event corpus files and asserted variants diverged"
    );

    for name in expected {
        let v = read_corpus(&format!("events/{name}"));
        let raw = serde_json::to_string(&v).unwrap();
        let ev: SidecarEvent = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("events/{name}: deserialize: {e}"));
        assert_event_matches_fixture(name, &v, &ev);
    }
}

/// Spot-check that `ready` parses in both the full and bare legacy
/// forms (the SDK never emits the bare form; Rust must still accept
/// it — that backward-compat guarantee is corpus-pinned here).
#[test]
fn ready_full_and_minimal_both_parse() {
    let full = read_corpus("events/ready_full.json");
    if let SidecarEvent::Ready { params } = serde_json::from_value(full.clone()).unwrap() {
        assert_eq!(
            serde_json::to_value(params.capabilities).unwrap(),
            full["params"]["capabilities"]
        );
        assert_eq!(params.account_id.as_deref(), Some("bot-1"));
        // Compared against the constant, not a literal: the corpus number and the daemon's number are the same contract, and `tests/sidecar_version_contract.rs` pins the rest of the mirrors.
        assert_eq!(params.protocol_version, Some(SIDECAR_PROTOCOL_VERSION));
        assert_eq!(
            classify_protocol_version(params.protocol_version),
            ProtocolSkew::Match
        );
    } else {
        panic!("ready_full did not parse as Ready");
    }

    let minimal = read_corpus("events/ready_minimal.json");
    match serde_json::from_value::<SidecarEvent>(minimal).unwrap() {
        SidecarEvent::Ready { params } => {
            assert!(params.capabilities.is_empty());
            assert!(params.protocol_version.is_none());
            // Accepting the bare frame is not the same as treating it as current: an adapter that declares nothing is `Unspecified`, and the supervisor warns rather than assuming it speaks v1.
            assert_eq!(
                classify_protocol_version(params.protocol_version),
                ProtocolSkew::Unspecified
            );
        }
        _ => panic!("ready_minimal did not parse as Ready"),
    }
}

/// Consumer side for the frame a Telegram slash command travels in.
///
/// `Content::Command` was the only frozen-core content shape with no corpus fixture, so the supervisor's ability to read a slash command off the wire was never pinned — the exact path #7140 reported as inert.
/// `content` deserializes as a whole: an adapter emitting a shape this `ChannelContent` cannot model fails `SidecarMessageParams` outright and the supervisor drops the entire `message` event, command and all.
#[test]
fn message_command_deserializes_into_channel_content_command() {
    let v = read_corpus("events/message_command.json");
    let ev: SidecarEvent = serde_json::from_value(v).expect("message_command must deserialize");
    let SidecarEvent::Message { params } = ev else {
        panic!("message_command did not parse as Message");
    };
    match params.content {
        Some(ChannelContent::Command { name, args }) => {
            assert_eq!(name, "agent");
            assert_eq!(args, vec!["researcher".to_string()]);
        }
        other => panic!("expected ChannelContent::Command, got {other:?}"),
    }
    // No `text` mirror: only plain-text content has a lossless flattening, so a supervisor that reads `text` and ignores `content` sees nothing here.
    // That is the whole reason the command path depends on `content` parsing.
    assert_eq!(params.text, None);
    assert_eq!(params.message_id.as_deref(), Some("8891"));
}

fn user(platform_id: &str, display_name: &str) -> ChannelUser {
    ChannelUser {
        platform_id: platform_id.to_string(),
        display_name: display_name.to_string(),
        librefang_user: None,
    }
}

/// Producer side: each `SidecarCommand` serializes to *exactly* the
/// corpus frame (structural JSON value equality).
#[test]
fn commands_serialize_to_corpus() {
    let cases: Vec<(&str, SidecarCommand)> = vec![
        (
            "send_full.json",
            SidecarCommand::Send {
                params: SidecarSendParams {
                    channel_id: "c1".into(),
                    text: "hello".into(),
                    content: Some(ChannelContent::Text("hello".into())),
                    thread_id: Some("t1".into()),
                    user: user("c1", "Alice"),
                },
            },
        ),
        (
            "send_minimal.json",
            SidecarCommand::Send {
                params: SidecarSendParams {
                    channel_id: "c1".into(),
                    text: "hi".into(),
                    content: None,
                    thread_id: None,
                    user: user("c1", "Bob"),
                },
            },
        ),
        ("ready_ack.json", SidecarCommand::ReadyAck),
        ("shutdown.json", SidecarCommand::Shutdown),
        ("heartbeat.json", SidecarCommand::Heartbeat),
        (
            "typing.json",
            SidecarCommand::Typing {
                params: SidecarTypingCmdParams {
                    channel_id: "c1".into(),
                },
            },
        ),
        (
            "reaction.json",
            SidecarCommand::Reaction {
                params: SidecarReactionParams {
                    channel_id: "c1".into(),
                    message_id: "55".into(),
                    reaction: "👍".into(),
                    // Empty `phase` / `None` `tool_name` are skipped on the
                    // wire, so the enriched struct still serializes to the
                    // legacy emoji-only corpus frame (#6451).
                    phase: String::new(),
                    tool_name: None,
                },
            },
        ),
        (
            "interactive.json",
            SidecarCommand::Interactive {
                params: SidecarInteractiveParams {
                    channel_id: "c1".into(),
                    message: InteractiveMessage {
                        text: "pick".into(),
                        buttons: vec![vec![
                            InteractiveButton {
                                label: "Yes".into(),
                                action: "y".into(),
                                style: None,
                                url: None,
                            },
                            InteractiveButton {
                                label: "Docs".into(),
                                action: "d".into(),
                                style: None,
                                url: Some("https://x".into()),
                            },
                        ]],
                    },
                },
            },
        ),
        (
            "stream_start.json",
            SidecarCommand::StreamStart {
                params: SidecarStreamStartParams {
                    channel_id: "c1".into(),
                    stream_id: "s1".into(),
                    thread_id: None,
                },
            },
        ),
        (
            "stream_start_threaded.json",
            SidecarCommand::StreamStart {
                params: SidecarStreamStartParams {
                    channel_id: "c1".into(),
                    stream_id: "s1".into(),
                    thread_id: Some("t1".into()),
                },
            },
        ),
        (
            "stream_delta.json",
            SidecarCommand::StreamDelta {
                params: SidecarStreamDeltaParams {
                    stream_id: "s1".into(),
                    text: "Hel".into(),
                },
            },
        ),
        (
            "stream_end.json",
            SidecarCommand::StreamEnd {
                params: SidecarStreamEndParams {
                    stream_id: "s1".into(),
                },
            },
        ),
    ];

    // Every command corpus file must have a case here — a fixture with
    // no producer assertion is not conformance.
    let mut covered: Vec<String> = cases.iter().map(|(n, _)| n.to_string()).collect();
    covered.sort();
    assert_eq!(
        covered,
        list_json("commands"),
        "command corpus files and asserted cases diverged"
    );

    for (name, cmd) in cases {
        let got = serde_json::to_value(&cmd).unwrap();
        let want = read_corpus(&format!("commands/{name}"));
        assert_eq!(got, want, "commands/{name}: serialize != corpus");
    }
}

/// #6451: a `tool_use` reaction serializes the enriched `phase` +
/// `tool_name` fields (so a reaction consumer can render a live step
/// list), while an empty `phase` / absent `tool_name` are dropped from
/// the wire (backward-compatible with the emoji-only frame).
#[test]
fn reaction_serializes_phase_and_tool_name_when_present() {
    let cmd = SidecarCommand::Reaction {
        params: SidecarReactionParams {
            channel_id: "c1".into(),
            message_id: "55".into(),
            reaction: "\u{2699}\u{FE0F}".into(),
            phase: "tool_use".into(),
            tool_name: Some("web_fetch".into()),
        },
    };
    let got = serde_json::to_value(&cmd).unwrap();
    assert_eq!(
        got,
        serde_json::json!({
            "method": "reaction",
            "params": {
                "channel_id": "c1",
                "message_id": "55",
                "reaction": "\u{2699}\u{FE0F}",
                "phase": "tool_use",
                "tool_name": "web_fetch",
            }
        })
    );

    // A non-tool phase drops `tool_name` but keeps `phase`.
    let thinking = SidecarCommand::Reaction {
        params: SidecarReactionParams {
            channel_id: "c1".into(),
            message_id: "55".into(),
            reaction: "\u{1F914}".into(),
            phase: "thinking".into(),
            tool_name: None,
        },
    };
    let got = serde_json::to_value(&thinking).unwrap();
    assert_eq!(got["params"].get("tool_name"), None);
    assert_eq!(got["params"]["phase"], "thinking");
}
