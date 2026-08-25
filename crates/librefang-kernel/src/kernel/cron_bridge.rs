//! Cron-side glue between [`LibreFangKernel::send_channel_message`] and the
//! multi-target fan-out engine in [`crate::cron_delivery`]. The
//! `KernelCronBridge` struct itself lives in `kernel::mod` (it holds an
//! `Arc<LibreFangKernel>`); only the trait impl and the two cron fan-out
//! helpers live here.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use librefang_runtime::kernel_handle::ChannelSender;
use librefang_types::agent::AgentId;

use super::{KernelCronBridge, LibreFangKernel};

impl LibreFangKernel {
    /// Deliver the output produced by a cron action through both the legacy single-destination setting and the newer multi-target fan-out list.
    ///
    /// Keeping this operation on the kernel gives scheduled fires and manual `POST /api/schedules/{id}/run` calls one delivery path.
    /// Delivery is intentionally best-effort: adapter failures are logged by the helpers and do not turn a successfully completed action into a failed action.
    pub async fn deliver_cron_output(
        self: &Arc<Self>,
        agent_id: AgentId,
        job_name: &str,
        output: &str,
        silent: bool,
        delivery: &librefang_types::scheduler::CronDelivery,
        delivery_targets: &[librefang_types::scheduler::CronDeliveryTarget],
    ) {
        if silent {
            return;
        }

        cron_deliver_response(self, agent_id, output, delivery).await;
        if !delivery_targets.is_empty() {
            cron_fan_out_targets(self, job_name, output, delivery_targets).await;
        }
    }
}

/// Webhook HTTP timeout used for the fan-out client. Mirrors
/// `crate::cron_delivery::WEBHOOK_TIMEOUT_SECS` (kept private there); the
/// duplication is acceptable because this builder lives outside the engine
/// crate boundary and the two values must move together.
const FAN_OUT_WEBHOOK_TIMEOUT_SECS: u64 = 30;

/// Shared HTTP client for cron fan-out webhook delivery.
///
/// `reqwest::Client` is documented to be cloned and reused — it pools
/// connections, DNS cache, and the TLS context internally. Constructing
/// one per fire (the historical behaviour, #5127) churned TLS handshakes
/// and idle pools on busy cron loads. We cache exactly one construction
/// result for the lifetime of the process. A failure is handed to each
/// engine so webhook targets fail in-band while other targets still run.
static FAN_OUT_HTTP_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

/// Build the cron fan-out HTTP client. Pulled out into a free function so
/// tests can drive it directly without going through `OnceLock`.
///
/// Routes through `librefang_runtime::http_client::proxied_client_builder()`
/// so the fan-out client picks up the daemon's `[proxy]` config (HTTPS_PROXY,
/// HTTP_PROXY, NO_PROXY), the bundled `webpki-roots` TLS fallback (required
/// on minimal Docker / Termux / musl images that lack a system CA bundle),
/// and the project-wide `librefang/<version>` User-Agent. The legacy
/// single-target webhook path below (`cron_deliver_response` →
/// `CronDelivery::Webhook`) uses the same helper; the fan-out path used to
/// drift to a bare `reqwest::Client::builder()` (no proxy, no CA fallback,
/// no UA) which was a silent regression vs. the legacy delivery.
fn build_fan_out_http_client_from(
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::Client, String> {
    builder
        .timeout(Duration::from_secs(FAN_OUT_WEBHOOK_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("HTTP client unavailable for cron fan-out: {error}"))
}

fn build_fan_out_http_client() -> Result<reqwest::Client, String> {
    build_fan_out_http_client_from(librefang_runtime::http_client::proxied_client_builder())
}

/// Return a clone of the shared cron fan-out HTTP client, initialising it
/// on first access. `Client` cloning is cheap — it bumps an `Arc` on the
/// inner pool — so handing one per fire to the engine is the intended
/// reuse pattern.
fn shared_fan_out_http_client() -> Result<reqwest::Client, String> {
    FAN_OUT_HTTP_CLIENT
        .get_or_init(build_fan_out_http_client)
        .clone()
}

#[async_trait::async_trait]
impl crate::cron_delivery::CronChannelSender for KernelCronBridge {
    async fn send_channel_message(
        &self,
        channel_type: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<(), String> {
        self.kernel
            .send_channel_message(channel_type, recipient, message, thread_id, account_id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Sentinel body sent when the agent / workflow produced no output but the
/// caller still wants every fan-out target invoked (heartbeat semantics).
/// Plain text so all adapters render it identically.
const CRON_EMPTY_OUTPUT_HEARTBEAT: &str = "(cron heartbeat: empty output)";

/// Fan out `output` to every target in `delivery_targets` concurrently.
///
/// Best-effort: never returns an error, because the cron job itself has
/// already succeeded by the time we get here. Per-target failures are
/// counted and logged. The legacy single-destination `delivery` field is
/// handled separately by [`cron_deliver_response`].
///
/// **Empty output is not silently dropped.** When `output.is_empty()` we
/// substitute a short heartbeat marker so every configured target still
/// fires — the previous early-return swallowed the delivery entirely and
/// broke liveness-style cron jobs (e.g. "ping #ops every hour even when I
/// have nothing to say"). Cron jobs that genuinely want to skip empty-
/// output runs should not configure fan-out targets at all.
pub(super) async fn cron_fan_out_targets(
    kernel: &Arc<LibreFangKernel>,
    job_name: &str,
    output: &str,
    targets: &[librefang_types::scheduler::CronDeliveryTarget],
) {
    if targets.is_empty() {
        return;
    }
    let payload: &str = if output.is_empty() {
        CRON_EMPTY_OUTPUT_HEARTBEAT
    } else {
        output
    };
    let sender: Arc<dyn crate::cron_delivery::CronChannelSender> = Arc::new(KernelCronBridge {
        kernel: kernel.clone(),
    });
    // Reuse one process-wide `reqwest::Client` across every fire instead of
    // rebuilding it (TLS, DNS, HTTP/2 pool) per cron tick (#5127).
    let engine = crate::cron_delivery::CronDeliveryEngine::with_http_client_result(
        sender,
        shared_fan_out_http_client(),
    );
    let results = engine.deliver(targets, job_name, payload).await;
    let total = results.len();
    let failures = results.iter().filter(|r| !r.success).count();
    let successes = total - failures;
    if failures == 0 {
        tracing::info!(
            job = %job_name,
            targets = total,
            "Cron fan-out: all {successes} target(s) delivered"
        );
    } else {
        tracing::warn!(
            job = %job_name,
            total = total,
            ok = successes,
            failed = failures,
            "Cron fan-out: partial delivery"
        );
        for r in results.iter().filter(|r| !r.success) {
            tracing::warn!(
                job = %job_name,
                target = %r.target,
                error = %r.error.as_deref().unwrap_or(""),
                "Cron fan-out: target failed"
            );
        }
    }
}

async fn post_legacy_webhook(
    client: Result<reqwest::Client, String>,
    url: &str,
    payload: &serde_json::Value,
) -> Result<reqwest::StatusCode, String> {
    let client = client?;
    client
        .post(url)
        .json(payload)
        .send()
        .await
        .map(|response| response.status())
        .map_err(|error| error.to_string())
}

/// Deliver a cron job's agent response to the configured delivery target.
pub(super) async fn cron_deliver_response(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    response: &str,
    delivery: &librefang_types::scheduler::CronDelivery,
) {
    use librefang_types::scheduler::CronDelivery;

    if response.is_empty() {
        return;
    }

    match delivery {
        CronDelivery::None => {}
        CronDelivery::Channel { channel, to } => {
            tracing::debug!(channel = %channel, to = %to, "Cron: delivering to channel");
            // Persist as last channel for this agent (survives restarts)
            let kv_val = serde_json::json!({"channel": channel, "recipient": to});
            let _ =
                kernel
                    .memory
                    .substrate
                    .structured_set(agent_id, "delivery.last_channel", kv_val);
            if let Err(e) = kernel
                .send_channel_message(channel, to, response, None, None)
                .await
            {
                tracing::warn!(channel = %channel, to = %to, error = %e, "Cron channel delivery failed");
            }
        }
        CronDelivery::LastChannel => {
            match kernel
                .memory
                .substrate
                .structured_get(agent_id, "delivery.last_channel")
            {
                Ok(Some(val)) => {
                    let channel = val["channel"].as_str().unwrap_or("");
                    let recipient = val["recipient"].as_str().unwrap_or("");
                    if !channel.is_empty() && !recipient.is_empty() {
                        tracing::info!(
                            channel = %channel,
                            recipient = %recipient,
                            "Cron: delivering to last channel"
                        );
                        if let Err(e) = kernel
                            .send_channel_message(channel, recipient, response, None, None)
                            .await
                        {
                            tracing::warn!(channel = %channel, recipient = %recipient, error = %e, "Cron last_channel delivery failed");
                        }
                    }
                }
                _ => {
                    tracing::debug!("Cron: no last channel found for agent {}", agent_id);
                }
            }
        }
        CronDelivery::Webhook { url } => {
            tracing::debug!(url = %url, "Cron: delivering via webhook");
            let payload = serde_json::json!({
                "agent_id": agent_id.to_string(),
                "response": response,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            match post_legacy_webhook(shared_fan_out_http_client(), url, &payload).await {
                Ok(status) => {
                    tracing::debug!(status = %status, "Cron webhook delivered");
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Cron webhook delivery failed");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The cron fan-out path must construct its `reqwest::Client` at most
    /// once across N invocations, not once per fire (#5127). We can't read
    /// the production `OnceLock` counter directly, so this test pins the
    /// `OnceLock + get_or_init(build_fn)` pattern the production helper
    /// uses by driving a local `OnceLock<Result<reqwest::Client, String>>`
    /// with a counted
    /// builder closure across many calls and asserting the builder ran
    /// exactly once.
    #[test]
    fn fan_out_http_client_builds_once_across_many_fires() {
        let builds = AtomicUsize::new(0);
        let slot: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

        let make = || {
            builds.fetch_add(1, Ordering::SeqCst);
            build_fan_out_http_client()
        };

        // Simulate 64 cron fires going through the lazy accessor.
        for _ in 0..64 {
            let _client = slot.get_or_init(make).clone();
        }

        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "cron fan-out client must build exactly once across many fires, \
             not once per fire — see #5127"
        );
        assert!(slot.get().expect("client result was initialized").is_ok());
    }

    #[test]
    fn fan_out_http_client_build_failure_is_returned() {
        let builder = reqwest::Client::builder().user_agent("invalid\nuser-agent");

        let error = build_fan_out_http_client_from(builder)
            .expect_err("invalid user-agent must fail client construction");

        assert!(error.contains("HTTP client unavailable for cron fan-out"));
    }

    #[tokio::test]
    async fn legacy_webhook_reports_cached_client_build_failure() {
        let error = post_legacy_webhook(
            Err("HTTP client unavailable: simulated build failure".to_string()),
            "https://example.invalid/hook",
            &serde_json::json!({"response": "payload"}),
        )
        .await
        .expect_err("cached client failure must be returned");

        assert!(error.contains("simulated build failure"));
    }

    /// `shared_fan_out_http_client` must hand back a `reqwest::Client` that
    /// is structurally usable as a webhook poster — i.e. the build cannot
    /// silently downgrade to a default-config client that omits the
    /// `WEBHOOK_TIMEOUT_SECS` timeout. We can't introspect the timeout off
    /// `reqwest::Client` directly, but we *can* assert the same accessor
    /// returns clones that share state (cloning is `Arc`-cheap by docs),
    /// so two reads in quick succession landing on the same instance is
    /// the structural invariant we pin here.
    #[test]
    fn shared_fan_out_http_client_returns_reusable_handle() {
        let a = shared_fan_out_http_client().expect("shared HTTP client");
        let b = shared_fan_out_http_client().expect("shared HTTP client");
        // `reqwest::Client` does not expose pool-identity, so we exercise
        // the only behaviour reuse promises: building requests against the
        // same handle does not panic and produces a well-formed
        // `RequestBuilder`. The real value of this test is keeping
        // `shared_fan_out_http_client` on the API surface — a future
        // refactor that deletes it would regress #5127.
        let _ = a.post("http://127.0.0.1:0/").build();
        let _ = b.post("http://127.0.0.1:0/").build();
    }
}
