//! Rust SDK side of the sidecar protocol conformance suite.
//!
//! The shared corpus (`conformance/sidecar/corpus/`) is the single oracle.
//! This crate (the Rust *adapter* SDK) is — like the Python SDK — both a **producer** of events and a **consumer** of commands, so we assert in both directions:
//!
//! * Events: builder output == corpus JSON value (structural).
//! * Commands: `parse_command(corpus)` == expected typed `Command`.
//!
//! This mirrors `sdk/python/tests/test_sidecar_conformance.py`.
//! The Rust supervisor's pair (`crates/librefang-channels/tests/sidecar_protocol_conformance.rs`) asserts the OTHER direction for each frame kind.
//!
//! Equality is structural JSON value equality, not byte equality — see `conformance/sidecar/README.md`.

use librefang_sidecar::protocol::{events, parse_command, ChannelUser, Command, Content};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    // sdk/rust/librefang-sidecar/Cargo.toml → repo root is 3 up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/sidecar/corpus")
}

/// Tests in this file are integration tests against the SHARED corpus that lives at the LibreFang repo root.
/// When the crate is consumed outside that repo (a `cargo publish` package, a docs.rs build, a downstream vendor that ran `cargo test -p librefang-sidecar`), the corpus path does not exist and every test below would otherwise panic with "corpus dir missing".
/// This helper returns true when the corpus IS present (in-repo) and false otherwise; tests skip-with-stderr-message in the absent case instead of failing.
fn corpus_available() -> bool {
    corpus_dir().is_dir()
}

macro_rules! require_corpus {
    () => {
        if !corpus_available() {
            eprintln!(
                "[librefang-sidecar conformance] corpus not present at {} — skipping (expected when the crate is consumed outside the librefang repo).",
                corpus_dir().display()
            );
            return;
        }
    };
}

fn read_corpus(rel: &str) -> Value {
    let path = corpus_dir().join(rel);
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read corpus {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse corpus {}: {e}", path.display()))
}

fn list_json(subdir: &str) -> HashSet<String> {
    let dir = corpus_dir().join(subdir);
    fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect()
}

// ── Setup sanity ────────────────────────────────────────────────────

#[test]
fn corpus_present() {
    // Skip cleanly when the corpus is absent (out-of-repo consumer).
    // In-repo this still asserts the structural invariant that both subdirs are populated.
    require_corpus!();
    assert!(!list_json("events").is_empty(), "no event fixtures");
    assert!(!list_json("commands").is_empty(), "no command fixtures");
}

// ── Producer side: events ───────────────────────────────────────────

/// `events/ready_minimal.json` is the bare legacy `{"method":"ready"}`
/// form. The SDK never *emits* it (the builder always writes full
/// params), it exists so the daemon's *consumer* side stays
/// backward-compatible. Documented producer-side skip — same as the
/// Python SDK's `EVENT_PRODUCER_SKIP`.
fn event_producer_skip() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("ready_minimal.json".to_string());
    s
}

fn build_event(name: &str) -> Value {
    match name {
        "ready_full.json" => events::ready(
            vec![
                "typing".into(),
                "reaction".into(),
                "interactive".into(),
                "thread".into(),
                "streaming".into(),
            ],
            Some("bot-1".into()),
            false,
            Vec::new(),
            Vec::new(),
            Some(1),
        ),
        "message_text.json" => librefang_sidecar::MessageBuilder::new("42", "Alice")
            .content(Content::text("hello"))
            .channel_id("-100123")
            .platform("telegram")
            .build(),
        "message_minimal.json" => librefang_sidecar::MessageBuilder::new("1", "Bob")
            .text("hi")
            .build(),
        "error.json" => events::error("boom"),
        "typing.json" => events::typing("u", "n", true),
        "qr_ready.json" => events::qr_ready(
            "wxp://f2f0YGcQ-xxxxxxxxxxxxxxxxxxxx",
            Some("https://login.example/qr?code=abc".into()),
            Some("Scan within 5 minutes".into()),
            Some("2026-05-28T20:00:00Z".into()),
        ),
        "qr_status.json" => events::qr_status("confirmed", Some("Login confirmed".into())),
        other => panic!("unmapped event fixture: {other}"),
    }
}

#[test]
fn every_event_fixture_is_covered() {
    require_corpus!();
    // A fixture with neither a producer assertion nor a documented skip is not conformance — mirror the Python coverage guard.
    // (Non-empty corpus check lives in `corpus_present` so we don't redundantly assert it here.)
    let actual = list_json("events");
    let asserted: HashSet<String> = [
        "ready_full.json",
        "message_text.json",
        "message_minimal.json",
        "error.json",
        "typing.json",
        "qr_ready.json",
        "qr_status.json",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let union: HashSet<String> = asserted.union(&event_producer_skip()).cloned().collect();
    assert_eq!(
        union, actual,
        "event corpus is not fully covered by asserts ∪ skip"
    );
}

#[test]
fn event_builders_match_corpus() {
    require_corpus!();
    let skip = event_producer_skip();
    for name in list_json("events") {
        if skip.contains(&name) {
            continue;
        }
        let expected = read_corpus(&format!("events/{name}"));
        let actual = build_event(&name);
        assert_eq!(
            actual, expected,
            "event builder output for {name} does not match corpus"
        );
    }
}

// ── Consumer side: commands ─────────────────────────────────────────

fn parse_cmd(name: &str) -> Command {
    let v = read_corpus(&format!("commands/{name}"));
    let raw = serde_json::to_string(&v).unwrap();
    parse_command(&raw).unwrap_or_else(|e| panic!("parse_command({name}): {e}"))
}

#[test]
fn every_command_fixture_is_covered() {
    require_corpus!();
    // Non-empty check is in `corpus_present`; here we only assert coverage.
    let actual = list_json("commands");
    let asserted: HashSet<String> = [
        "send_full.json",
        "send_minimal.json",
        "ready_ack.json",
        "shutdown.json",
        "heartbeat.json",
        "typing.json",
        "reaction.json",
        "interactive.json",
        "stream_start.json",
        "stream_start_threaded.json",
        "stream_delta.json",
        "stream_end.json",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        asserted, actual,
        "command corpus is not fully covered by parse asserts"
    );
}

#[test]
fn command_send_full() {
    require_corpus!();
    match parse_cmd("send_full.json") {
        Command::Send(s) => {
            assert_eq!(s.channel_id, "c1");
            assert_eq!(s.text, "hello");
            assert_eq!(s.thread_id.as_deref(), Some("t1"));
            assert_eq!(s.content, Some(Content::text("hello")));
            assert_eq!(
                s.user,
                ChannelUser {
                    platform_id: "c1".into(),
                    display_name: "Alice".into(),
                    librefang_user: None,
                }
            );
        }
        other => panic!("expected Send, got {other:?}"),
    }
}

#[test]
fn command_send_minimal() {
    require_corpus!();
    match parse_cmd("send_minimal.json") {
        Command::Send(s) => {
            assert_eq!(s.channel_id, "c1");
            assert_eq!(s.text, "hi");
            // The minimal corpus omits content + thread_id, so we
            // expect None on both — Serde defaults to None when the
            // key is absent.
            assert!(s.content.is_none());
            assert!(s.thread_id.is_none());
        }
        other => panic!("expected Send, got {other:?}"),
    }
}

#[test]
fn command_parameterless() {
    require_corpus!();
    assert_eq!(parse_cmd("ready_ack.json"), Command::ReadyAck);
    assert_eq!(parse_cmd("shutdown.json"), Command::Shutdown);
    assert_eq!(parse_cmd("heartbeat.json"), Command::Heartbeat);
}

#[test]
fn command_typing() {
    require_corpus!();
    match parse_cmd("typing.json") {
        Command::Typing(t) => assert_eq!(t.channel_id, "c1"),
        other => panic!("expected Typing, got {other:?}"),
    }
}

#[test]
fn command_reaction() {
    require_corpus!();
    match parse_cmd("reaction.json") {
        Command::Reaction(r) => {
            assert_eq!(r.channel_id, "c1");
            assert_eq!(r.message_id, "55");
            assert_eq!(r.reaction, "👍");
        }
        other => panic!("expected Reaction, got {other:?}"),
    }
}

#[test]
fn command_interactive() {
    require_corpus!();
    match parse_cmd("interactive.json") {
        Command::Interactive(i) => {
            assert_eq!(i.channel_id, "c1");
            assert_eq!(i.message.text, "pick");
            assert_eq!(i.message.buttons.len(), 1);
            let row = &i.message.buttons[0];
            assert_eq!(row.len(), 2);
            assert_eq!(row[0].label, "Yes");
            assert_eq!(row[0].action, "y");
            assert!(row[0].url.is_none());
            assert_eq!(row[1].label, "Docs");
            assert_eq!(row[1].url.as_deref(), Some("https://x"));
        }
        other => panic!("expected Interactive, got {other:?}"),
    }
}

#[test]
fn command_stream_start_default_thread() {
    require_corpus!();
    match parse_cmd("stream_start.json") {
        Command::StreamStart(s) => {
            assert_eq!(s.channel_id, "c1");
            assert_eq!(s.stream_id, "s1");
            assert!(s.thread_id.is_none());
        }
        other => panic!("expected StreamStart, got {other:?}"),
    }
}

#[test]
fn command_stream_start_threaded() {
    require_corpus!();
    match parse_cmd("stream_start_threaded.json") {
        Command::StreamStart(s) => {
            assert_eq!(s.thread_id.as_deref(), Some("t1"));
        }
        other => panic!("expected StreamStart, got {other:?}"),
    }
}

#[test]
fn command_stream_delta_and_end() {
    require_corpus!();
    match parse_cmd("stream_delta.json") {
        Command::StreamDelta(d) => {
            assert_eq!(d.stream_id, "s1");
            assert_eq!(d.text, "Hel");
        }
        other => panic!("expected StreamDelta, got {other:?}"),
    }
    match parse_cmd("stream_end.json") {
        Command::StreamEnd(e) => assert_eq!(e.stream_id, "s1"),
        other => panic!("expected StreamEnd, got {other:?}"),
    }
}
