//! Multi-destination cron output delivery.
//!
//! A single [`CronJob`](librefang_types::scheduler::CronJob) may declare zero
//! or more [`CronDeliveryTarget`]s on its `delivery_targets` field. After the
//! job fires and produces output, the [`CronDeliveryEngine`] fans out the
//! same payload to every target concurrently. Failures in one target do not
//! abort delivery to the others — every target's outcome is captured in a
//! [`DeliveryResult`].
//!
//! This is the LibreFang port of the OpenFang multi-destination cron pattern
//! (see openfang commit `3db5d3a`): one job → N destinations
//! (channel / webhook / file / email).

use async_trait::async_trait;
use futures::future::join_all;
use librefang_types::scheduler::CronDeliveryTarget;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Webhook HTTP timeout. Matches the legacy single-target cron webhook
/// (`cron_deliver_response` in `kernel/mod.rs`).
const WEBHOOK_TIMEOUT_SECS: u64 = 30;

/// Per-target delivery outcome returned by [`CronDeliveryEngine::deliver`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryResult {
    /// Human-readable target description (e.g.
    /// `"channel:telegram -> chat_123"`, `"webhook:https://..."`,
    /// `"file:/tmp/out.log"`, `"email:alice@x"`).
    pub target: String,
    /// Whether delivery succeeded.
    pub success: bool,
    /// Error message if `success` is `false`.
    pub error: Option<String>,
}

impl DeliveryResult {
    fn ok(target: String) -> Self {
        Self {
            target,
            success: true,
            error: None,
        }
    }

    fn err(target: String, msg: String) -> Self {
        Self {
            target,
            success: false,
            error: Some(msg),
        }
    }
}

/// Channel dispatcher used by the engine to invoke channel adapters
/// (telegram, slack, email, …) without depending on the full `KernelHandle`
/// surface. The kernel implements it by delegating to its existing
/// `send_channel_message` method.
///
/// Defined here (not in `librefang-channels`) because the engine is owned
/// by the kernel and only needs this single method.
#[async_trait]
pub trait CronChannelDispatcher: Send + Sync {
    /// Send `message` via the named adapter to `recipient`. Returns
    /// `Err(reason)` on failure.
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
    ) -> Result<(), String>;
}

/// Fan-out delivery engine for cron job output.
///
/// Holds a reference to a `CronChannelDispatcher` (used for channel- and
/// email-style delivery) and a shared `reqwest::Client` (used for webhook
/// delivery). Constructed once per kernel and reused across every cron
/// firing.
pub struct CronDeliveryEngine {
    dispatcher: Arc<dyn CronChannelDispatcher>,
    http: reqwest::Client,
}

impl CronDeliveryEngine {
    /// Build a new engine using the given dispatcher and a fresh
    /// `reqwest::Client` with a 30s timeout. Falls back to the default
    /// client if the builder fails (effectively never on supported
    /// platforms).
    pub fn new(dispatcher: Arc<dyn CronChannelDispatcher>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(WEBHOOK_TIMEOUT_SECS))
            .build()
            .unwrap_or_default();
        Self { dispatcher, http }
    }

    /// Build a new engine with an explicit HTTP client — used by tests so
    /// the client can point at a mock server.
    pub fn with_http_client(
        dispatcher: Arc<dyn CronChannelDispatcher>,
        http: reqwest::Client,
    ) -> Self {
        Self { dispatcher, http }
    }

    /// Deliver `output` (and `job_id` / `agent_id` metadata) to every target
    /// concurrently.
    ///
    /// Returns one `DeliveryResult` per target, in the same order as the
    /// input slice. One target failing does not short-circuit the others —
    /// the underlying job already succeeded, fan-out is best-effort.
    pub async fn deliver(
        &self,
        targets: &[CronDeliveryTarget],
        job_id: &str,
        agent_id: &str,
        job_name: &str,
        output: &str,
    ) -> Vec<DeliveryResult> {
        if targets.is_empty() {
            return Vec::new();
        }
        let futures = targets
            .iter()
            .map(|t| self.deliver_one(t, job_id, agent_id, job_name, output));
        join_all(futures).await
    }

    /// Deliver to a single target. Never panics.
    async fn deliver_one(
        &self,
        target: &CronDeliveryTarget,
        job_id: &str,
        agent_id: &str,
        job_name: &str,
        output: &str,
    ) -> DeliveryResult {
        match target {
            CronDeliveryTarget::Channel { channel, to } => {
                let desc = format!("channel:{channel} -> {to}");
                match self
                    .dispatcher
                    .send_channel_message(channel, to, output)
                    .await
                {
                    Ok(()) => {
                        debug!(target = %desc, "Cron fan-out: channel delivery ok");
                        DeliveryResult::ok(desc)
                    }
                    Err(e) => {
                        warn!(target = %desc, error = %e, "Cron fan-out: channel delivery failed");
                        DeliveryResult::err(desc, e)
                    }
                }
            }
            CronDeliveryTarget::Webhook { url } => {
                let desc = format!("webhook:{url}");
                match deliver_webhook(&self.http, url, job_id, agent_id, job_name, output).await {
                    Ok(()) => {
                        debug!(target = %desc, "Cron fan-out: webhook delivery ok");
                        DeliveryResult::ok(desc)
                    }
                    Err(e) => {
                        warn!(target = %desc, error = %e, "Cron fan-out: webhook delivery failed");
                        DeliveryResult::err(desc, e)
                    }
                }
            }
            CronDeliveryTarget::File { path } => {
                let desc = format!("file:{path}");
                match deliver_file(Path::new(path), output).await {
                    Ok(()) => {
                        debug!(target = %desc, "Cron fan-out: file write ok");
                        DeliveryResult::ok(desc)
                    }
                    Err(e) => {
                        warn!(target = %desc, error = %e, "Cron fan-out: file write failed");
                        DeliveryResult::err(desc, e)
                    }
                }
            }
            CronDeliveryTarget::Email { to, subject } => {
                let desc = format!("email:{to}");
                let rendered = render_subject(subject.as_deref(), job_name);
                let body = format!("{rendered}\n\n{output}");
                // Route via the existing email channel adapter. If no email
                // adapter is configured the dispatcher returns Err which we
                // surface as a failed `DeliveryResult` (no silent success).
                match self
                    .dispatcher
                    .send_channel_message("email", to, &body)
                    .await
                {
                    Ok(()) => {
                        debug!(target = %desc, "Cron fan-out: email delivery ok");
                        DeliveryResult::ok(desc)
                    }
                    Err(e) => {
                        warn!(target = %desc, error = %e, "Cron fan-out: email delivery failed");
                        DeliveryResult::err(desc, e)
                    }
                }
            }
        }
    }
}

/// Render an email subject from an optional template. `{job}` is the only
/// supported placeholder; an empty/`None` template falls back to
/// `"Cron: <job_name>"`.
fn render_subject(template: Option<&str>, job_name: &str) -> String {
    match template {
        Some(t) if !t.is_empty() => t.replace("{job}", job_name),
        _ => format!("Cron: {job_name}"),
    }
}

/// POST a JSON payload `{job_id, agent_id, content}` to `url`. The 30s
/// timeout comes from the shared `reqwest::Client` configured in
/// [`CronDeliveryEngine::new`]. Returns `Err(msg)` on network failure or
/// non-2xx status.
async fn deliver_webhook(
    http: &reqwest::Client,
    url: &str,
    job_id: &str,
    agent_id: &str,
    job_name: &str,
    output: &str,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "job_id": job_id,
        "agent_id": agent_id,
        "job": job_name,
        "content": output,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let resp = http
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("webhook send failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("webhook returned HTTP {status}"));
    }
    Ok(())
}

/// Append `output` (followed by a newline) to `path`, creating parent
/// directories as needed. Rejects any path that contains a `..` component
/// as defence-in-depth on top of the validation already performed in
/// `CronDeliveryTarget::validate`.
async fn deliver_file(path: &Path, output: &str) -> Result<(), String> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path must not contain '..' components".into());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create parent dir failed: {e}"))?;
        }
    }
    use tokio::io::AsyncWriteExt;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("open failed: {e}"))?;
    f.write_all(output.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    // Newline separator between successive runs makes tailing nicer.
    f.write_all(b"\n")
        .await
        .map_err(|e| format!("write newline failed: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records every dispatch call. Optionally fails when the channel name
    /// matches `fail_on_channel`.
    struct MockDispatcher {
        calls: Mutex<Vec<(String, String, String)>>,
        fail_on_channel: Option<String>,
    }

    impl MockDispatcher {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                fail_on_channel: None,
            })
        }

        fn failing_on(channel: &str) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                fail_on_channel: Some(channel.to_string()),
            })
        }

        fn calls(&self) -> Vec<(String, String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CronChannelDispatcher for MockDispatcher {
        async fn send_channel_message(
            &self,
            channel: &str,
            recipient: &str,
            message: &str,
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push((
                channel.to_string(),
                recipient.to_string(),
                message.to_string(),
            ));
            if let Some(ref f) = self.fail_on_channel {
                if f == channel {
                    return Err(format!("mock: forced failure on '{channel}'"));
                }
            }
            Ok(())
        }
    }

    fn engine_with(dispatcher: Arc<MockDispatcher>) -> CronDeliveryEngine {
        CronDeliveryEngine::new(dispatcher)
    }

    // -- Empty targets -------------------------------------------------------

    #[tokio::test]
    async fn empty_targets_returns_empty_vec() {
        let engine = engine_with(MockDispatcher::new());
        let results = engine.deliver(&[], "j", "a", "name", "x").await;
        assert!(results.is_empty());
    }

    // -- Channel: success ----------------------------------------------------

    #[tokio::test]
    async fn channel_target_invokes_dispatcher() {
        let mock = MockDispatcher::new();
        let engine = engine_with(mock.clone());
        let target = CronDeliveryTarget::Channel {
            channel: "slack".into(),
            to: "C12345".into(),
        };
        let results = engine
            .deliver(&[target], "job-1", "agent-1", "alerts", "fire")
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success, "error: {:?}", results[0].error);
        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "slack");
        assert_eq!(calls[0].1, "C12345");
        assert_eq!(calls[0].2, "fire");
    }

    // -- Channel: failure ----------------------------------------------------

    #[tokio::test]
    async fn channel_target_failure_is_reported() {
        let mock = MockDispatcher::failing_on("slack");
        let engine = engine_with(mock);
        let target = CronDeliveryTarget::Channel {
            channel: "slack".into(),
            to: "C1".into(),
        };
        let results = engine
            .deliver(&[target], "job", "agent", "name", "payload")
            .await;
        assert!(!results[0].success);
        assert!(results[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("forced failure"));
    }

    // -- File: append + creates parents -------------------------------------

    #[tokio::test]
    async fn file_target_creates_and_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/deep/out.log");
        let target = CronDeliveryTarget::File {
            path: path.to_string_lossy().to_string(),
        };
        let engine = engine_with(MockDispatcher::new());
        let r1 = engine
            .deliver(std::slice::from_ref(&target), "j", "a", "name", "first")
            .await;
        let r2 = engine.deliver(&[target], "j", "a", "name", "second").await;
        assert!(r1[0].success, "{:?}", r1[0].error);
        assert!(r2[0].success, "{:?}", r2[0].error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("first") && content.contains("second"),
            "got: {content:?}"
        );
    }

    // -- File: parent-dir traversal rejected --------------------------------

    #[tokio::test]
    async fn file_target_rejects_parent_dir_components() {
        let target = CronDeliveryTarget::File {
            path: "logs/../../escape.txt".into(),
        };
        let engine = engine_with(MockDispatcher::new());
        let results = engine.deliver(&[target], "j", "a", "name", "x").await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].success, "must reject '..' paths");
        assert!(results[0].error.as_deref().unwrap_or("").contains(".."));
    }

    // -- Webhook: success + payload shape -----------------------------------

    #[tokio::test]
    async fn webhook_sends_payload_with_metadata() {
        let (port, rx) = spawn_mock_http_server(200, "OK").await;
        let url = format!("http://127.0.0.1:{port}/hook");
        let target = CronDeliveryTarget::Webhook { url };
        let engine = engine_with(MockDispatcher::new());
        let results = engine
            .deliver(&[target], "job-7", "agent-9", "daily", "result body")
            .await;
        assert!(results[0].success, "error: {:?}", results[0].error);
        let captured = rx.await.expect("mock server never received a request");
        assert!(
            captured.body.contains("\"job_id\":\"job-7\""),
            "missing job_id, got: {}",
            captured.body
        );
        assert!(
            captured.body.contains("\"agent_id\":\"agent-9\""),
            "missing agent_id, got: {}",
            captured.body
        );
        assert!(
            captured.body.contains("\"content\":\"result body\""),
            "missing content, got: {}",
            captured.body
        );
    }

    // -- Webhook: non-2xx ----------------------------------------------------

    #[tokio::test]
    async fn webhook_reports_non_2xx_status() {
        let (port, _rx) = spawn_mock_http_server(500, "Internal Server Error").await;
        let url = format!("http://127.0.0.1:{port}/hook");
        let target = CronDeliveryTarget::Webhook { url };
        let engine = engine_with(MockDispatcher::new());
        let results = engine.deliver(&[target], "j", "a", "name", "x").await;
        assert!(!results[0].success);
        assert!(results[0].error.as_deref().unwrap_or("").contains("500"));
    }

    // -- Email: routes through dispatcher with rendered subject -------------

    #[tokio::test]
    async fn email_target_routes_via_dispatcher() {
        let mock = MockDispatcher::new();
        let engine = engine_with(mock.clone());
        let target = CronDeliveryTarget::Email {
            to: "alice@example.com".into(),
            subject: Some("Report: {job}".into()),
        };
        let results = engine
            .deliver(&[target], "j", "a", "weekly", "the body")
            .await;
        assert!(results[0].success, "{:?}", results[0].error);
        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "email");
        assert_eq!(calls[0].1, "alice@example.com");
        assert!(
            calls[0].2.starts_with("Report: weekly"),
            "subject not rendered into body: {}",
            calls[0].2
        );
        assert!(
            calls[0].2.contains("the body"),
            "body missing in dispatch: {}",
            calls[0].2
        );
    }

    // -- Mixed success / failure --------------------------------------------

    #[tokio::test]
    async fn mixed_targets_partial_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let ok_path = tmp.path().join("ok.txt");
        let targets = vec![
            // Will succeed.
            CronDeliveryTarget::File {
                path: ok_path.to_string_lossy().to_string(),
            },
            // Will fail (mock dispatcher rejects 'slack').
            CronDeliveryTarget::Channel {
                channel: "slack".into(),
                to: "C1".into(),
            },
        ];
        let engine = engine_with(MockDispatcher::failing_on("slack"));
        let results = engine
            .deliver(&targets, "job", "agent", "name", "payload")
            .await;
        assert_eq!(results.len(), 2);
        assert!(results[0].success, "file delivery should succeed");
        assert!(!results[1].success, "channel delivery should fail");
        // File was still written even though the other target failed.
        let content = std::fs::read_to_string(&ok_path).unwrap();
        assert!(content.contains("payload"));
    }

    // -- render_subject helper ----------------------------------------------

    #[test]
    fn render_subject_substitutes_placeholder() {
        assert_eq!(render_subject(Some("Cron: {job}"), "daily"), "Cron: daily");
        assert_eq!(
            render_subject(Some("no placeholder"), "x"),
            "no placeholder"
        );
        assert_eq!(render_subject(None, "daily"), "Cron: daily");
        assert_eq!(render_subject(Some(""), "daily"), "Cron: daily");
    }

    // -- Minimal HTTP mock --------------------------------------------------
    // Adapted from the openfang reference impl at commit 3db5d3a so we can
    // run webhook tests without a heavy hyper/httpmock dependency.

    struct CapturedRequest {
        #[allow(dead_code)]
        headers: Vec<String>,
        body: String,
    }

    async fn spawn_mock_http_server(
        status: u16,
        reason: &'static str,
    ) -> (u16, tokio::sync::oneshot::Receiver<CapturedRequest>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => return,
            };

            let mut buf = Vec::with_capacity(4096);
            let mut tmp = [0u8; 1024];
            let mut headers_end: Option<usize> = None;
            let mut content_length: Option<usize> = None;
            loop {
                let n = match stream.read(&mut tmp).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => return,
                };
                buf.extend_from_slice(&tmp[..n]);
                if headers_end.is_none() {
                    if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                        headers_end = Some(pos + 4);
                        let head_str = String::from_utf8_lossy(&buf[..pos]);
                        for line in head_str.lines() {
                            if let Some(v) = line.strip_prefix("Content-Length: ") {
                                content_length = v.trim().parse::<usize>().ok();
                            } else if let Some(v) = line.strip_prefix("content-length: ") {
                                content_length = v.trim().parse::<usize>().ok();
                            }
                        }
                    }
                }
                if let (Some(end), Some(cl)) = (headers_end, content_length) {
                    if buf.len() >= end + cl {
                        break;
                    }
                }
                if headers_end.is_some() && content_length.is_none() {
                    break;
                }
            }

            let head_end = headers_end.unwrap_or(buf.len());
            let head_str = String::from_utf8_lossy(&buf[..head_end.saturating_sub(4)]).to_string();
            let body_bytes = if head_end < buf.len() {
                &buf[head_end..]
            } else {
                &[][..]
            };
            let body = String::from_utf8_lossy(body_bytes).to_string();
            let headers: Vec<String> = head_str.lines().skip(1).map(|l| l.to_string()).collect();

            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;

            let _ = tx.send(CapturedRequest { headers, body });
        });

        (port, rx)
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }
}
