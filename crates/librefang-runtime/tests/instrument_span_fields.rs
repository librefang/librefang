//! Verifies that `agent.id` and `session.id` set as `#[instrument]` fields on
//! `run_agent_loop` propagate to events emitted inside the loop. We don't call
//! `run_agent_loop` directly (it requires a kernel + memory + LLM driver);
//! instead we construct an equivalent span by hand and assert the formatter
//! sees both fields when an event fires inside it.

use std::io;
use std::sync::{Arc, Mutex};
use tracing::{info_span, span, warn, Level};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn warn_inside_agent_span_includes_agent_and_session_ids() {
    let writer = CaptureWriter::default();
    let buf = writer.0.clone();

    // Use a non-compact format so span fields appear inline. This is the
    // assertion target for this test only — the WithTraceId wrapper test in
    // librefang-cli covers the compact + suffix path.
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false);

    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let span = info_span!(
        "run_agent_loop",
        agent.id = "agent-uuid-1234",
        session.id = "session-uuid-5678",
    );
    let _entered = span.enter();
    warn!("test event from inside agent loop");

    let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        captured.contains("agent.id=\"agent-uuid-1234\"")
            || captured.contains("agent.id=agent-uuid-1234"),
        "expected agent.id in captured log, got: {captured}"
    );
    assert!(
        captured.contains("session.id=\"session-uuid-5678\"")
            || captured.contains("session.id=session-uuid-5678"),
        "expected session.id in captured log, got: {captured}"
    );
}

/// Reproduces the production daemon filter (`librefang_runtime=warn`) and
/// asserts that an INFO-level instrument span gets dropped — confirming WHY
/// `run_agent_loop` must use `level = "warn"`. If this test ever flips
/// behaviour (i.e. INFO spans survive a WARN target filter), our
/// `level = "warn"` workaround is no longer needed.
#[test]
fn info_span_is_dropped_under_warn_target_filter() {
    let writer = CaptureWriter::default();
    let buf = writer.0.clone();

    let env_filter = EnvFilter::new("warn");
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false);

    let _guard = tracing_subscriber::registry()
        .with(env_filter)
        .with(layer)
        .set_default();

    // INFO span — same as the original `#[instrument]` default.
    let info_span = info_span!(
        "run_agent_loop",
        agent.id = "agent-uuid-aaaa",
        session.id = "session-uuid-bbbb",
    );
    let _e = info_span.enter();
    warn!("event under warn filter, info span");

    let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        !captured.contains("agent.id=\"agent-uuid-aaaa\""),
        "INFO span should have been filtered out by warn-only filter, but agent.id leaked: {captured}"
    );
}

/// The actual production fix: a WARN-level span survives the WARN target
/// filter and propagates `agent.id` / `session.id` to events fired inside it.
/// This test pins the behaviour `run_agent_loop`'s `#[instrument(level = "warn", ...)]`
/// guarantees on the live daemon, where `init_tracing_stderr` installs
/// `librefang_runtime=warn` as a baseline directive.
#[test]
fn warn_span_survives_warn_target_filter_and_carries_fields() {
    let writer = CaptureWriter::default();
    let buf = writer.0.clone();

    let env_filter = EnvFilter::new("warn");
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false);

    let _guard = tracing_subscriber::registry()
        .with(env_filter)
        .with(layer)
        .set_default();

    // WARN-level span — matches the real `#[instrument(level = "warn", ...)]`.
    let warn_span = span!(
        Level::WARN,
        "run_agent_loop",
        agent.id = "agent-uuid-cccc",
        session.id = "session-uuid-dddd",
    );
    let _e = warn_span.enter();
    warn!("event under warn filter, warn span");

    let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        captured.contains("agent.id=\"agent-uuid-cccc\"")
            || captured.contains("agent.id=agent-uuid-cccc"),
        "WARN span should survive warn filter and surface agent.id, got: {captured}"
    );
    assert!(
        captured.contains("session.id=\"session-uuid-dddd\"")
            || captured.contains("session.id=session-uuid-dddd"),
        "WARN span should surface session.id, got: {captured}"
    );
}
