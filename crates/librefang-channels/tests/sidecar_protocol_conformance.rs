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
    SidecarCommand, SidecarEvent, SidecarInteractiveParams, SidecarReactionParams,
    SidecarSendParams, SidecarStreamDeltaParams, SidecarStreamEndParams, SidecarStreamStartParams,
    SidecarTypingCmdParams,
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
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    out.sort();
    out
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

/// Consumer side: every corpus event deserializes into the
/// `SidecarEvent` variant named by its `method`.
#[test]
fn events_deserialize_into_expected_variant() {
    for name in list_json("events") {
        let v = read_corpus(&format!("events/{name}"));
        let method = v["method"].as_str().unwrap().to_string();
        let raw = serde_json::to_string(&v).unwrap();
        let ev: SidecarEvent = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("events/{name}: deserialize: {e}"));
        let ok = matches!(
            (&ev, method.as_str()),
            (SidecarEvent::Message { .. }, "message")
                | (SidecarEvent::Ready { .. }, "ready")
                | (SidecarEvent::Error { .. }, "error")
                | (SidecarEvent::Typing { .. }, "typing")
        );
        assert!(ok, "events/{name}: parsed variant != method {method:?}");
    }
}

/// Spot-check that `ready` parses in both the full and bare legacy
/// forms (the SDK never emits the bare form; Rust must still accept
/// it — that backward-compat guarantee is corpus-pinned here).
#[test]
fn ready_full_and_minimal_both_parse() {
    let full = read_corpus("events/ready_full.json");
    if let SidecarEvent::Ready { params } = serde_json::from_value(full).unwrap() {
        assert_eq!(
            params.capabilities,
            vec!["typing", "reaction", "interactive", "thread", "streaming"]
        );
        assert_eq!(params.account_id.as_deref(), Some("bot-1"));
        assert_eq!(params.protocol_version, Some(1));
    } else {
        panic!("ready_full did not parse as Ready");
    }

    let minimal = read_corpus("events/ready_minimal.json");
    match serde_json::from_value::<SidecarEvent>(minimal).unwrap() {
        SidecarEvent::Ready { params } => {
            assert!(params.capabilities.is_empty());
            assert!(params.protocol_version.is_none());
        }
        _ => panic!("ready_minimal did not parse as Ready"),
    }
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
