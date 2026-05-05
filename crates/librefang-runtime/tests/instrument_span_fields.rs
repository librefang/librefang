//! Verifies that `agent.id` and `session.id` set as `#[instrument]` fields on
//! `run_agent_loop` propagate to events emitted inside the loop. We don't call
//! `run_agent_loop` directly (it requires a kernel + memory + LLM driver);
//! instead we construct an equivalent span by hand and assert the formatter
//! sees both fields when an event fires inside it.

use std::io;
use std::sync::{Arc, Mutex};
use tracing::{info_span, warn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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
