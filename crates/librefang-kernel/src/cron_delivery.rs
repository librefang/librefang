//! Multi-destination cron output delivery.
//!
//! A single [`CronJob`](librefang_types::scheduler::CronJob) may declare
//! zero or more [`CronDeliveryTarget`]s on its `delivery_targets` field.
//! After the job fires and produces output, the [`CronDeliveryEngine`] fans
//! out the same payload to every target concurrently. Failures in one
//! target do not abort delivery to the others — every target's outcome is
//! returned in a [`DeliveryResult`].
//!
//! The legacy single-destination `delivery` field on `CronJob` is left
//! untouched and still handled by `cron_deliver_response` in `kernel/mod.rs`.
//! The fan-out engine runs *in addition* to legacy delivery.

use async_trait::async_trait;
use futures::future::join_all;
use librefang_types::scheduler::CronDeliveryTarget;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Webhook HTTP timeout. Matches the legacy single-target cron webhook
/// delivery in `cron_deliver_response`.
const WEBHOOK_TIMEOUT_SECS: u64 = 30;

/// Per-target wall-clock timeout for fan-out delivery. Slow targets must
/// not block the others — `join_all` waits for the longest future, so each
/// `deliver_one` call is wrapped in `tokio::time::timeout` to bound how
/// long any single target can stall the overall fan-out.
const PER_TARGET_TIMEOUT_SECS: u64 = 60;

/// Per-target delivery outcome returned by [`CronDeliveryEngine::deliver`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryResult {
    /// Human-readable target description (`"channel:telegram -> chat_123"`,
    /// `"webhook:https://..."`, `"file:/tmp/out.log"`, `"email:alice@x"`).
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

/// Minimal channel-send abstraction the fan-out engine depends on.
///
/// Defined locally instead of reusing `librefang_channels::bridge::
/// ChannelBridgeHandle` because the bridge trait is keyed by `AgentId` and
/// does not expose a `(channel_type, recipient, message)` send. The kernel
/// implements this with a thin shim over `LibreFangKernel::send_channel_message`;
/// tests provide a mock.
#[async_trait]
pub trait CronChannelSender: Send + Sync {
    /// Deliver `message` via the channel adapter named `channel_type` to
    /// `recipient`. `thread_id` selects an in-channel thread/topic when the
    /// adapter supports it; `account_id` disambiguates between multiple
    /// configured accounts of the same channel (via the
    /// `<channel>:<account_id>` adapter-key suffix). Returns
    /// `Err(human_readable_reason)` on failure.
    async fn send_channel_message(
        &self,
        channel_type: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<(), String>;
}

/// Fan-out delivery engine for cron job output.
///
/// Holds a reference to a [`CronChannelSender`] (for adapter-based delivery)
/// and a shared HTTP client (for webhook delivery). Constructed once per
/// fan-out invocation; stateless across firings.
pub struct CronDeliveryEngine {
    /// Sender used to invoke channel adapters (Telegram, Slack, Email, ...).
    channel_sender: Arc<dyn CronChannelSender>,
    /// Shared HTTP client for webhook delivery.
    http: reqwest::Client,
}

impl CronDeliveryEngine {
    /// Build a new engine using the given channel sender and a fresh
    /// `reqwest::Client`. The HTTP client construction failing would mean
    /// the daemon is in an unrecoverable state (no TLS backend, no resolver,
    /// etc.), so we surface it loudly via `expect` rather than silently
    /// downgrading to `Client::default()` and producing confusing
    /// per-webhook errors at delivery time.
    pub fn new(channel_sender: Arc<dyn CronChannelSender>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(WEBHOOK_TIMEOUT_SECS))
            .build()
            .expect("HTTP client build failed for cron fan-out engine");
        Self {
            channel_sender,
            http,
        }
    }

    /// Build a new engine with an explicit HTTP client — used by tests.
    pub fn with_http_client(
        channel_sender: Arc<dyn CronChannelSender>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            channel_sender,
            http,
        }
    }

    /// Deliver `output` to every target concurrently.
    ///
    /// Returns a `Vec<DeliveryResult>` with one entry per target in the same
    /// order as the input slice. One target failing does not short-circuit
    /// the others — the job already succeeded, delivery is best-effort.
    /// Each target is also bounded by [`PER_TARGET_TIMEOUT_SECS`] so a slow
    /// adapter or a hung HTTP socket cannot stretch the whole fan-out
    /// indefinitely; a timed-out target is reported as a failure with an
    /// explicit "delivery timed out" error.
    pub async fn deliver(
        &self,
        targets: &[CronDeliveryTarget],
        job_name: &str,
        output: &str,
    ) -> Vec<DeliveryResult> {
        if targets.is_empty() {
            return Vec::new();
        }
        let timeout = Duration::from_secs(PER_TARGET_TIMEOUT_SECS);
        let futures = targets.iter().map(|t| {
            let desc = describe_target(t);
            let fut = self.deliver_one(t, job_name, output);
            async move {
                match tokio::time::timeout(timeout, fut).await {
                    Ok(res) => res,
                    Err(_) => {
                        warn!(
                            target = %desc,
                            timeout_secs = PER_TARGET_TIMEOUT_SECS,
                            "Cron fan-out: target timed out"
                        );
                        DeliveryResult::err(
                            desc,
                            format!("delivery timed out after {PER_TARGET_TIMEOUT_SECS}s"),
                        )
                    }
                }
            }
        });
        join_all(futures).await
    }

    /// Deliver to a single target. Never panics.
    async fn deliver_one(
        &self,
        target: &CronDeliveryTarget,
        job_name: &str,
        output: &str,
    ) -> DeliveryResult {
        match target {
            CronDeliveryTarget::Channel {
                channel_type,
                recipient,
                thread_id,
                account_id,
            } => {
                let desc = describe_target(target);
                match self
                    .channel_sender
                    .send_channel_message(
                        channel_type,
                        recipient,
                        output,
                        thread_id.as_deref(),
                        account_id.as_deref(),
                    )
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
            CronDeliveryTarget::Webhook { url, auth_header } => {
                let desc = format!("webhook:{url}");
                match deliver_webhook(&self.http, url, auth_header.as_deref(), job_name, output)
                    .await
                {
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
            CronDeliveryTarget::LocalFile { path, append } => {
                let desc = format!("file:{path}");
                match deliver_local_file(Path::new(path), *append, output).await {
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
            CronDeliveryTarget::Email {
                to,
                subject_template,
            } => {
                let desc = format!("email:{to}");
                let subject = render_subject(subject_template.as_deref(), job_name);
                // TODO(#3102): the existing email channel adapter takes
                // `(channel_type, recipient, message)` only — it does not
                // expose a dedicated `subject` parameter on the trait. As
                // a stop-gap we prepend the rendered subject as the first
                // line of the body so the recipient sees it; adapters that
                // already extract a leading `Subject:` header line will
                // surface it as the real RFC 5322 subject. Otherwise the
                // subject lands inside the body. A follow-up PR should add
                // a typed `subject` parameter to the email adapter and
                // route it through here so it lands in the actual headers.
                let body = format!("{subject}\n\n{output}");
                match self
                    .channel_sender
                    .send_channel_message("email", to, &body, None, None)
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

/// Stable, log-friendly description of a delivery target. Mirrors the
/// `target` field on `DeliveryResult` so timeout fallbacks and successful
/// deliveries report the same identifier in logs / API responses.
fn describe_target(target: &CronDeliveryTarget) -> String {
    match target {
        CronDeliveryTarget::Channel {
            channel_type,
            recipient,
            thread_id,
            account_id,
        } => {
            let mut s = format!("channel:{channel_type} -> {recipient}");
            if let Some(t) = thread_id.as_deref() {
                if !t.is_empty() {
                    s.push_str(&format!(" (thread={t})"));
                }
            }
            if let Some(a) = account_id.as_deref() {
                if !a.is_empty() {
                    s.push_str(&format!(" [account={a}]"));
                }
            }
            s
        }
        CronDeliveryTarget::Webhook { url, .. } => format!("webhook:{url}"),
        CronDeliveryTarget::LocalFile { path, .. } => format!("file:{path}"),
        CronDeliveryTarget::Email { to, .. } => format!("email:{to}"),
    }
}

/// Render an email subject from an optional template. `{job}` is the only
/// supported placeholder; everything else passes through unchanged.
fn render_subject(template: Option<&str>, job_name: &str) -> String {
    match template {
        Some(t) if !t.is_empty() => t.replace("{job}", job_name),
        _ => format!("Cron: {job_name}"),
    }
}

/// POST a JSON payload `{ job, output, timestamp }` to `url` and optionally
/// attach an `Authorization` header. Returns `Err(msg)` on non-2xx or
/// network failure.
async fn deliver_webhook(
    http: &reqwest::Client,
    url: &str,
    auth_header: Option<&str>,
    job_name: &str,
    output: &str,
) -> Result<(), String> {
    validate_webhook_url(url)?;
    let payload = serde_json::json!({
        "job": job_name,
        "output": output,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let mut req = http.post(url).json(&payload);
    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("webhook send failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("webhook returned HTTP {status}"));
    }
    Ok(())
}

/// Reject webhook URLs that would let an attacker pivot through the daemon
/// to local services or cloud metadata endpoints.
///
/// Cron delivery targets are reachable through the LLM tool surface, so a
/// prompt-injected agent could otherwise set `Webhook { url:
/// "http://169.254.169.254/latest/meta-data/iam/security-credentials/" }`
/// and exfiltrate cloud credentials, or call back into
/// `http://127.0.0.1:4545/api/agents` and abuse the daemon's loopback-bypass.
fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "webhook scheme must be http or https, got '{scheme}'"
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "webhook URL has no host".to_string())?;
    let host_lc = host.to_lowercase();
    // Block well-known names regardless of DNS resolution.
    if matches!(
        host_lc.as_str(),
        "localhost" | "metadata" | "metadata.google.internal" | "metadata.aws.amazon.com"
    ) {
        return Err(format!("webhook host '{host_lc}' is not allowed"));
    }
    // Literal IP addresses: refuse loopback and link-local (which covers
    // the AWS / GCP / Azure 169.254.169.254 metadata service). DNS names
    // are not resolved here — that's a known TOCTOU we accept; the
    // operator is expected to deploy the daemon with egress filtering
    // for paranoid environments.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let blocked = ip.is_loopback()
            || matches!(ip, std::net::IpAddr::V4(v4) if v4.is_link_local())
            || matches!(ip, std::net::IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80);
        if blocked {
            return Err(format!(
                "webhook host '{ip}' is loopback or link-local — refusing"
            ));
        }
    }
    Ok(())
}

/// Append or overwrite `output` at `path`. Creates parent directories when
/// missing. Returns `Err(msg)` on any I/O failure.
///
/// SECURITY: this layer trusts the path it is handed. The real safety net
/// against `LocalFile { path: "../../etc/passwd" }` and absolute paths
/// like `/etc/passwd` lives in `CronJob::validate_delivery_targets()`,
/// which rejects untrusted input before it ever reaches the scheduler.
/// We keep a defence-in-depth `..` check here so that a future code path
/// which forgets to validate can't trivially write outside the workspace,
/// but absolute paths are accepted because tests legitimately use
/// `tempfile::tempdir()` (which yields absolute paths under `/tmp` or
/// `/var/folders`) and rejecting them here would block all unit tests.
async fn deliver_local_file(path: &Path, append: bool, output: &str) -> Result<(), String> {
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "path traversal rejected: '..' component in {}",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create parent dir failed: {e}"))?;
        }
    }
    if append {
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
        // Newline separator between runs makes tailing nicer.
        f.write_all(b"\n")
            .await
            .map_err(|e| format!("write newline failed: {e}"))?;
    } else {
        tokio::fs::write(path, output.as_bytes())
            .await
            .map_err(|e| format!("write failed: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Captured arguments of a single `send_channel_message` invocation.
    /// The trailing `Option<String>` slots track the new `thread_id` and
    /// `account_id` plumbing so tests can assert they round-trip from
    /// `CronDeliveryTarget::Channel` through the engine.
    type SenderCall = (String, String, String, Option<String>, Option<String>);

    /// Mock channel sender that records every call. Optionally fails for
    /// specific channel names.
    struct MockSender {
        calls: Mutex<Vec<SenderCall>>,
        fail_on_channel: Option<String>,
    }

    impl MockSender {
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

        fn calls(&self) -> Vec<SenderCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CronChannelSender for MockSender {
        async fn send_channel_message(
            &self,
            channel_type: &str,
            recipient: &str,
            message: &str,
            thread_id: Option<&str>,
            account_id: Option<&str>,
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push((
                channel_type.to_string(),
                recipient.to_string(),
                message.to_string(),
                thread_id.map(str::to_string),
                account_id.map(str::to_string),
            ));
            if let Some(ref failing) = self.fail_on_channel {
                if failing == channel_type {
                    return Err(format!("mock: forced failure on '{channel_type}'"));
                }
            }
            Ok(())
        }
    }

    fn test_engine(sender: Arc<MockSender>) -> CronDeliveryEngine {
        CronDeliveryEngine::new(sender)
    }

    // -- LocalFile: overwrite ------------------------------------------------

    #[tokio::test]
    async fn localfile_overwrite_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("out.txt");
        let target = CronDeliveryTarget::LocalFile {
            path: path.to_string_lossy().to_string(),
            append: false,
        };

        let engine = test_engine(MockSender::new());
        let results = engine.deliver(&[target], "job-x", "hello world").await;

        assert_eq!(results.len(), 1);
        assert!(results[0].success, "error: {:?}", results[0].error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn localfile_overwrite_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("replace.txt");
        std::fs::write(&path, "OLD CONTENT").unwrap();

        let target = CronDeliveryTarget::LocalFile {
            path: path.to_string_lossy().to_string(),
            append: false,
        };
        let engine = test_engine(MockSender::new());
        let results = engine.deliver(&[target], "job-x", "NEW").await;

        assert!(results[0].success);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "NEW");
    }

    // -- LocalFile: append ---------------------------------------------------

    #[tokio::test]
    async fn localfile_append_adds_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.txt");
        let target = CronDeliveryTarget::LocalFile {
            path: path.to_string_lossy().to_string(),
            append: true,
        };
        let engine = test_engine(MockSender::new());

        engine
            .deliver(std::slice::from_ref(&target), "job", "first")
            .await;
        engine.deliver(&[target], "job", "second").await;

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("first") && content.contains("second"),
            "expected both lines in appended file, got: {content:?}"
        );
    }

    #[tokio::test]
    async fn localfile_creates_missing_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/deep/out.log");
        let target = CronDeliveryTarget::LocalFile {
            path: path.to_string_lossy().to_string(),
            append: true,
        };
        let engine = test_engine(MockSender::new());
        let results = engine.deliver(&[target], "job", "payload").await;

        assert!(results[0].success, "error: {:?}", results[0].error);
        assert!(path.exists(), "nested file should have been created");
    }

    // -- Webhook -------------------------------------------------------------

    #[tokio::test]
    async fn webhook_sends_payload() {
        let (port, rx) = spawn_mock_http_server(200, "OK").await;
        let url = format!("http://127.0.0.1:{port}/hook");

        let target = CronDeliveryTarget::Webhook {
            url: url.clone(),
            auth_header: Some("Bearer test-token".to_string()),
        };
        let engine = test_engine(MockSender::new());
        let results = engine
            .deliver(&[target], "daily-report", "result body")
            .await;

        assert!(results[0].success, "error: {:?}", results[0].error);

        let captured = rx.await.expect("mock server never received a request");
        assert!(
            captured.body.contains("\"job\":\"daily-report\""),
            "payload missing job name, got: {}",
            captured.body
        );
        assert!(
            captured.body.contains("\"output\":\"result body\""),
            "payload missing output, got: {}",
            captured.body
        );
        assert!(
            captured.body.contains("\"timestamp\""),
            "payload missing timestamp, got: {}",
            captured.body
        );
        assert!(
            captured
                .headers
                .iter()
                .any(|h| h.eq_ignore_ascii_case("authorization: Bearer test-token")),
            "missing auth header, got: {:?}",
            captured.headers
        );
    }

    #[tokio::test]
    async fn webhook_reports_non_2xx() {
        let (port, _rx) = spawn_mock_http_server(500, "Internal Server Error").await;
        let url = format!("http://127.0.0.1:{port}/hook");

        let target = CronDeliveryTarget::Webhook {
            url,
            auth_header: None,
        };
        let engine = test_engine(MockSender::new());
        let results = engine.deliver(&[target], "job", "output").await;

        assert!(!results[0].success);
        let err = results[0].error.as_deref().unwrap_or("");
        assert!(err.contains("500"), "expected 500 in error, got: {err}");
    }

    // -- Channel target ------------------------------------------------------

    #[tokio::test]
    async fn channel_target_invokes_sender() {
        let sender = MockSender::new();
        let engine = test_engine(sender.clone());
        let target = CronDeliveryTarget::Channel {
            channel_type: "slack".to_string(),
            recipient: "C12345".to_string(),
            thread_id: None,
            account_id: None,
        };
        let results = engine.deliver(&[target], "alerts", "fire").await;
        assert!(results[0].success, "error: {:?}", results[0].error);
        let calls = sender.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "slack");
        assert_eq!(calls[0].1, "C12345");
        assert_eq!(calls[0].2, "fire");
        assert!(calls[0].3.is_none(), "thread_id should be None when unset");
        assert!(calls[0].4.is_none(), "account_id should be None when unset");
    }

    #[tokio::test]
    async fn channel_target_passes_thread_and_account() {
        // Regression: thread_id and account_id must reach the channel
        // sender so multi-account / threaded adapters can route correctly.
        let sender = MockSender::new();
        let engine = test_engine(sender.clone());
        let target = CronDeliveryTarget::Channel {
            channel_type: "slack".to_string(),
            recipient: "C12345".to_string(),
            thread_id: Some("1700000000.000100".to_string()),
            account_id: Some("workspace-b".to_string()),
        };
        let results = engine.deliver(&[target], "alerts", "fire").await;
        assert!(results[0].success, "error: {:?}", results[0].error);
        let calls = sender.calls();
        assert_eq!(calls[0].3.as_deref(), Some("1700000000.000100"));
        assert_eq!(calls[0].4.as_deref(), Some("workspace-b"));
        // The descriptor should include the routing hints so log scrapers
        // can correlate failures with the configured target.
        assert!(
            results[0].target.contains("thread="),
            "expected thread hint in descriptor, got {:?}",
            results[0].target
        );
        assert!(
            results[0].target.contains("account="),
            "expected account hint in descriptor, got {:?}",
            results[0].target
        );
    }

    // -- Email target --------------------------------------------------------

    #[tokio::test]
    async fn email_target_routes_through_email_channel() {
        let sender = MockSender::new();
        let engine = test_engine(sender.clone());
        let target = CronDeliveryTarget::Email {
            to: "alice@example.com".to_string(),
            subject_template: Some("Daily: {job}".to_string()),
        };
        let results = engine.deliver(&[target], "report", "the body").await;
        assert!(results[0].success, "error: {:?}", results[0].error);
        let calls = sender.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "email");
        assert_eq!(calls[0].1, "alice@example.com");
        assert!(calls[0].2.starts_with("Daily: report"));
        assert!(calls[0].2.contains("the body"));
    }

    // -- Mixed success/failure (failure isolation) --------------------------

    #[tokio::test]
    async fn mixed_targets_one_success_one_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let ok_path = tmp.path().join("ok.txt");

        let targets = vec![
            // Will succeed (file write).
            CronDeliveryTarget::LocalFile {
                path: ok_path.to_string_lossy().to_string(),
                append: false,
            },
            // Will fail (mock sender rejects 'slack').
            CronDeliveryTarget::Channel {
                channel_type: "slack".to_string(),
                recipient: "C1".to_string(),
                thread_id: None,
                account_id: None,
            },
        ];

        let sender = MockSender::failing_on("slack");
        let engine = test_engine(sender);
        let results = engine.deliver(&targets, "job", "payload").await;

        assert_eq!(results.len(), 2);
        assert!(
            results[0].success,
            "file delivery should succeed: {:?}",
            results[0].error
        );
        assert!(
            !results[1].success,
            "channel delivery should fail, but got success"
        );
        assert!(results[1]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("forced failure"));

        // File was still written even though the other target failed —
        // failure isolation invariant.
        assert_eq!(std::fs::read_to_string(&ok_path).unwrap(), "payload");
    }

    #[tokio::test]
    async fn empty_targets_returns_empty_vec() {
        let engine = test_engine(MockSender::new());
        let results = engine.deliver(&[], "job", "x").await;
        assert!(results.is_empty());
    }

    /// A blocking sender used to verify the per-target timeout actually
    /// short-circuits a stuck delivery.
    struct StallSender;
    #[async_trait]
    impl CronChannelSender for StallSender {
        async fn send_channel_message(
            &self,
            _channel_type: &str,
            _recipient: &str,
            _message: &str,
            _thread_id: Option<&str>,
            _account_id: Option<&str>,
        ) -> Result<(), String> {
            // Sleep well past PER_TARGET_TIMEOUT_SECS using tokio time so
            // `tokio::time::pause` (start_paused) can advance the clock
            // instantly without making the test wall-clock-slow.
            tokio::time::sleep(Duration::from_secs(PER_TARGET_TIMEOUT_SECS * 10)).await;
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn per_target_timeout_isolates_stuck_target() {
        // A stuck target must not block sibling targets and must surface as
        // a timeout-flavoured failure rather than hanging the whole fan-out.
        let tmp = tempfile::tempdir().unwrap();
        let ok_path = tmp.path().join("ok.txt");
        let stall = Arc::new(StallSender);
        let engine = CronDeliveryEngine::new(stall);

        let targets = vec![
            CronDeliveryTarget::LocalFile {
                path: ok_path.to_string_lossy().to_string(),
                append: false,
            },
            CronDeliveryTarget::Channel {
                channel_type: "stuck".to_string(),
                recipient: "rcp".to_string(),
                thread_id: None,
                account_id: None,
            },
        ];
        let results = engine.deliver(&targets, "job", "payload").await;

        assert_eq!(results.len(), 2);
        assert!(results[0].success, "file write should still succeed");
        assert!(
            !results[1].success,
            "stuck channel must be reported as failure"
        );
        let err = results[1].error.as_deref().unwrap_or("");
        assert!(
            err.contains("timed out"),
            "expected timeout error, got: {err:?}"
        );
        // File side-effect proves the file delivery completed even though
        // the channel target was hung.
        assert_eq!(std::fs::read_to_string(&ok_path).unwrap(), "payload");
    }

    // -- Subject rendering ---------------------------------------------------

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

    // -- Minimal HTTP mock ---------------------------------------------------

    struct CapturedRequest {
        headers: Vec<String>,
        body: String,
    }

    /// Spawn a tiny TCP server that serves exactly one request, parses the
    /// HTTP/1.1 request line + headers + body, then responds with the given
    /// status code and reason phrase. Returns `(port, oneshot_rx)` where the
    /// oneshot resolves once the request has been received.
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

            // Read until we have full headers and the declared body.
            let mut buf = Vec::with_capacity(4096);
            let mut tmp = [0u8; 1024];
            let mut headers_end = None;
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
                        // Parse Content-Length.
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

            // Split into headers + body.
            let head_end = headers_end.unwrap_or(buf.len());
            let head_str = String::from_utf8_lossy(&buf[..head_end.saturating_sub(4)]).to_string();
            let body_bytes = if head_end < buf.len() {
                &buf[head_end..]
            } else {
                &[][..]
            };
            let body = String::from_utf8_lossy(body_bytes).to_string();
            let headers: Vec<String> = head_str.lines().skip(1).map(|l| l.to_string()).collect();

            // Send response.
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
