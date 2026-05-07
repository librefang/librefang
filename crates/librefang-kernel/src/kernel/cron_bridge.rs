//! Cron-side glue between [`LibreFangKernel::send_channel_message`] and the
//! multi-target fan-out engine in [`crate::cron_delivery`]. The
//! `KernelCronBridge` struct itself lives in `kernel::mod` (it holds an
//! `Arc<LibreFangKernel>`); only the trait impl and the two cron fan-out
//! helpers live here.

use std::sync::Arc;

use librefang_runtime::kernel_handle::ChannelSender;
use librefang_types::agent::AgentId;

use super::{KernelCronBridge, LibreFangKernel};

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
    let engine = crate::cron_delivery::CronDeliveryEngine::new(sender);
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
            let client = librefang_runtime::http_client::proxied_client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .build();
            if let Ok(client) = client {
                let payload = serde_json::json!({
                    "agent_id": agent_id.to_string(),
                    "response": response,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });
                match client.post(url).json(&payload).send().await {
                    Ok(resp) => {
                        tracing::debug!(status = %resp.status(), "Cron webhook delivered");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Cron webhook delivery failed");
                    }
                }
            }
        }
    }
}
