//! Channel bridge wiring — connects the LibreFang kernel to channel adapters.
//!
//! Implements `ChannelBridgeHandle` on `LibreFangKernel` and provides the
//! `start_channel_bridge()` entry point called by the daemon.

use librefang_channels::bridge::{BridgeManager, ChannelBridgeHandle};
use librefang_channels::router::AgentRouter;
use librefang_channels::sidecar::SidecarAdapter;
use librefang_channels::types::{ChannelAdapter, SenderContext};

/// Check if text looks like a raw tool call leaked as content.
///
/// Some providers emit tool calls as plain text (recovered by
/// `agent_loop::recover_text_tool_calls`). These should not be
/// forwarded to the user through streaming channels.
fn looks_like_tool_call(text: &str) -> bool {
    let t = text.trim();
    // JSON-style tool calls (may appear at start of text)
    t.starts_with("[{")
        || t.starts_with("functions.")
        || t.starts_with("{\"type\":\"function\"")
        || (t.starts_with('[') && t.contains("'type': 'text'"))
        || contains_bare_json_tool_call(t)
        // Tag-based patterns — use contains() because tool call tags may
        // appear after natural language preamble
        || t.contains("<function=")
        || t.contains("<function>")
        || t.contains("<function ")
        || t.contains("<tool>")
        || t.contains("[TOOL_CALL]")
        || t.contains("<tool_call>")
        // Pattern 4: markdown code block containing a tool call
        || contains_markdown_tool_call(t)
        // Pattern 5: backtick-wrapped tool call
        || contains_backtick_tool_call(t)
}

fn contains_markdown_tool_call(text: &str) -> bool {
    let mut in_block = false;
    let mut block_content = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_block {
                if looks_like_named_json_tool_call(&block_content) {
                    return true;
                }
                block_content.clear();
                in_block = false;
            } else {
                in_block = true;
                block_content.clear();
            }
        } else if in_block {
            if !block_content.is_empty() {
                block_content.push('\n');
            }
            block_content.push_str(trimmed);
        }
    }

    false
}

fn contains_backtick_tool_call(text: &str) -> bool {
    text.split('`')
        .skip(1)
        .step_by(2)
        .any(looks_like_named_json_tool_call)
}

fn looks_like_named_json_tool_call(text: &str) -> bool {
    let trimmed = text.trim();
    let Some(brace_pos) = trimmed.find('{') else {
        return false;
    };

    let potential_tool = trimmed[..brace_pos].trim();
    if potential_tool.is_empty() || !looks_like_tool_name(potential_tool) {
        return false;
    }

    serde_json::from_str::<serde_json::Value>(trimmed[brace_pos..].trim()).is_ok()
}

fn looks_like_tool_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/'))
}

fn contains_bare_json_tool_call(text: &str) -> bool {
    let mut scan_from = 0;

    while let Some(brace_start) = text[scan_from..].find('{') {
        let abs_brace = scan_from + brace_start;
        if let Some(end) = find_json_object_end(&text[abs_brace..]) {
            if looks_like_tool_call_object(&text[abs_brace..abs_brace + end]) {
                return true;
            }
        }
        scan_from = abs_brace + 1;
    }

    false
}

fn find_json_object_end(text: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;

    for (i, c) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }

            match c {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn looks_like_tool_call_object(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };

    let Some(name) = obj
        .get("name")
        .or_else(|| obj.get("function"))
        .or_else(|| obj.get("tool"))
        .and_then(|value| value.as_str())
    else {
        return false;
    };

    if !looks_like_tool_name(name) {
        return false;
    }

    let args = obj
        .get("arguments")
        .or_else(|| obj.get("parameters"))
        .or_else(|| obj.get("args"))
        .or_else(|| obj.get("input"));

    match args {
        Some(serde_json::Value::String(s)) => serde_json::from_str::<serde_json::Value>(s).is_ok(),
        Some(_) => true,
        None => matches!(
            obj.get("type").and_then(|value| value.as_str()),
            Some("function")
        ),
    }
}

// Feature-gated adapter imports
#[cfg(feature = "channel-discord")]
use librefang_channels::discord::DiscordAdapter;
#[cfg(feature = "channel-email")]
use librefang_channels::email::EmailAdapter;
#[cfg(feature = "channel-google-chat")]
use librefang_channels::google_chat::GoogleChatAdapter;
#[cfg(feature = "channel-irc")]
use librefang_channels::irc::IrcAdapter;
#[cfg(feature = "channel-matrix")]
use librefang_channels::matrix::MatrixAdapter;
#[cfg(feature = "channel-mattermost")]
use librefang_channels::mattermost::MattermostAdapter;
#[cfg(feature = "channel-rocketchat")]
use librefang_channels::rocketchat::RocketChatAdapter;
#[cfg(feature = "channel-signal")]
use librefang_channels::signal::SignalAdapter;
#[cfg(feature = "channel-slack")]
use librefang_channels::slack::SlackAdapter;
#[cfg(feature = "channel-teams")]
use librefang_channels::teams::TeamsAdapter;
#[cfg(feature = "channel-telegram")]
use librefang_channels::telegram::TelegramAdapter;
#[cfg(feature = "channel-twitch")]
use librefang_channels::twitch::TwitchAdapter;
#[cfg(feature = "channel-webhook")]
use librefang_channels::webhook::WebhookAdapter;
#[cfg(feature = "channel-whatsapp")]
use librefang_channels::whatsapp::WhatsAppAdapter;
#[cfg(feature = "channel-xmpp")]
use librefang_channels::xmpp::XmppAdapter;
#[cfg(feature = "channel-zulip")]
use librefang_channels::zulip::ZulipAdapter;
// Wave 3
#[cfg(feature = "channel-bluesky")]
use librefang_channels::bluesky::BlueskyAdapter;
#[cfg(feature = "channel-feishu")]
use librefang_channels::feishu::{FeishuAdapter, FeishuReceiveMode, FeishuRegion};
#[cfg(feature = "channel-line")]
use librefang_channels::line::LineAdapter;
#[cfg(feature = "channel-mastodon")]
use librefang_channels::mastodon::MastodonAdapter;
#[cfg(feature = "channel-messenger")]
use librefang_channels::messenger::MessengerAdapter;
#[cfg(feature = "channel-reddit")]
use librefang_channels::reddit::RedditAdapter;
#[cfg(feature = "channel-revolt")]
use librefang_channels::revolt::RevoltAdapter;
#[cfg(feature = "channel-viber")]
use librefang_channels::viber::ViberAdapter;
// Wave 4
#[cfg(feature = "channel-flock")]
use librefang_channels::flock::FlockAdapter;
#[cfg(feature = "channel-guilded")]
use librefang_channels::guilded::GuildedAdapter;
#[cfg(feature = "channel-keybase")]
use librefang_channels::keybase::KeybaseAdapter;
#[cfg(feature = "channel-nextcloud")]
use librefang_channels::nextcloud::NextcloudAdapter;
#[cfg(feature = "channel-nostr")]
use librefang_channels::nostr::NostrAdapter;
#[cfg(feature = "channel-pumble")]
use librefang_channels::pumble::PumbleAdapter;
#[cfg(feature = "channel-threema")]
use librefang_channels::threema::ThreemaAdapter;
#[cfg(feature = "channel-twist")]
use librefang_channels::twist::TwistAdapter;
#[cfg(feature = "channel-webex")]
use librefang_channels::webex::WebexAdapter;
// Wave 5
#[cfg(feature = "channel-dingtalk")]
use librefang_channels::dingtalk::DingTalkAdapter;
#[cfg(feature = "channel-discourse")]
use librefang_channels::discourse::DiscourseAdapter;
#[cfg(feature = "channel-gitter")]
use librefang_channels::gitter::GitterAdapter;
#[cfg(feature = "channel-gotify")]
use librefang_channels::gotify::GotifyAdapter;
#[cfg(feature = "channel-linkedin")]
use librefang_channels::linkedin::LinkedInAdapter;
#[cfg(feature = "channel-mumble")]
use librefang_channels::mumble::MumbleAdapter;
#[cfg(feature = "channel-ntfy")]
use librefang_channels::ntfy::NtfyAdapter;
#[cfg(feature = "channel-qq")]
use librefang_channels::qq::QqAdapter;
#[cfg(feature = "channel-wecom")]
use librefang_channels::wecom::WeComAdapter;

use async_trait::async_trait;
use librefang_kernel::error::KernelResult;
use librefang_kernel::LibreFangKernel;
use librefang_runtime::llm_driver::StreamEvent;
use librefang_types::agent::AgentId;
use std::sync::Arc;
#[cfg(feature = "channel-telegram")]
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use librefang_runtime::str_utils::safe_truncate_str;

fn start_stream_text_bridge(
    mut event_rx: mpsc::Receiver<StreamEvent>,
    kernel_handle: tokio::task::JoinHandle<
        KernelResult<librefang_runtime::agent_loop::AgentLoopResult>,
    >,
) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>(64);

    let bridge_handle = tokio::spawn(async move {
        // Buffer text per iteration. Some providers emit tool call syntax
        // as plain text (recovered by agent_loop later). We hold text until
        // ContentComplete, then flush only if it doesn't look like a raw
        // tool call or content-block array.
        let mut iter_buf = String::new();
        let mut saw_tool_use = false;

        while let Some(event) = event_rx.recv().await {
            match event {
                StreamEvent::TextDelta { text } => {
                    iter_buf.push_str(&text);
                }
                StreamEvent::ContentComplete { .. } => {
                    // Flush buffered text. Only suppress when ToolUseStart
                    // was seen in this iteration (the text is the tool call
                    // echoed as content). Do NOT apply heuristic filtering
                    // here — normal replies that demonstrate tool syntax
                    // (e.g. "use `web_search {…}`") must not be discarded.
                    if !iter_buf.is_empty() {
                        if saw_tool_use {
                            debug!("Streaming bridge: filtered tool-use-adjacent text");
                        } else if tx.send(std::mem::take(&mut iter_buf)).await.is_err() {
                            break;
                        }
                    }
                    iter_buf.clear();
                    saw_tool_use = false;
                }
                StreamEvent::ToolUseStart { .. } => {
                    saw_tool_use = true;
                }
                _ => {}
            }
        }

        if !iter_buf.is_empty() && !saw_tool_use {
            if !looks_like_tool_call(&iter_buf) {
                let _ = tx.send(iter_buf).await;
            } else {
                debug!("Streaming bridge: filtered leaked tool call text in final flush");
            }
        }
    });

    tokio::spawn(async move {
        match kernel_handle.await {
            Err(e) => error!("Streaming kernel task panicked: {e}"),
            Ok(Err(e)) => error!("Streaming kernel task returned error: {e}"),
            Ok(Ok(result)) => {
                debug!(
                    input_tokens = result.total_usage.input_tokens,
                    output_tokens = result.total_usage.output_tokens,
                    iterations = result.iterations,
                    "Streaming kernel task completed"
                );
            }
        }
        if let Err(e) = bridge_handle.await {
            error!("Streaming bridge task panicked: {e}");
        }
    });

    rx
}

/// Wraps `LibreFangKernel` to implement `ChannelBridgeHandle`.
pub struct KernelBridgeAdapter {
    kernel: Arc<LibreFangKernel>,
    started_at: Instant,
}

#[async_trait]
impl ChannelBridgeHandle for KernelBridgeAdapter {
    async fn send_message(&self, agent_id: AgentId, message: &str) -> Result<String, String> {
        let result = self
            .kernel
            .send_message(agent_id, message)
            .await
            .map_err(|e| format!("{e}"))?;
        // When the agent intentionally chose not to reply (NO_REPLY / [[silent]]),
        // return an empty string so the bridge skips sending a response to the channel.
        if result.silent {
            Ok(String::new())
        } else {
            Ok(result.response)
        }
    }

    async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        blocks: Vec<librefang_types::message::ContentBlock>,
    ) -> Result<String, String> {
        // Extract text for the message parameter (used for memory recall / logging)
        let text: String = blocks
            .iter()
            .filter_map(|b| match b {
                librefang_types::message::ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = if text.is_empty() {
            "[Image]".to_string()
        } else {
            text
        };
        let result = self
            .kernel
            .send_message_with_blocks(agent_id, &text, blocks)
            .await
            .map_err(|e| format!("{e}"))?;
        if result.silent {
            Ok(String::new())
        } else {
            Ok(result.response)
        }
    }

    async fn send_message_streaming(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> Result<mpsc::Receiver<String>, String> {
        let (event_rx, kernel_handle) = self
            .kernel
            .send_message_streaming_with_routing(agent_id, message, None)
            .await
            .map_err(|e| format!("{e}"))?;
        Ok(start_stream_text_bridge(event_rx, kernel_handle))
    }

    async fn send_message_streaming_with_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
    ) -> Result<mpsc::Receiver<String>, String> {
        let (event_rx, kernel_handle) = self
            .kernel
            .send_message_streaming_with_sender_context_and_routing(agent_id, message, None, sender)
            .await
            .map_err(|e| format!("{e}"))?;
        Ok(start_stream_text_bridge(event_rx, kernel_handle))
    }

    async fn send_message_with_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
    ) -> Result<String, String> {
        let result = self
            .kernel
            .send_message_with_sender_context(agent_id, message, sender)
            .await
            .map_err(|e| format!("{e}"))?;
        Ok(result.response)
    }

    async fn send_message_with_blocks_and_sender(
        &self,
        agent_id: AgentId,
        blocks: Vec<librefang_types::message::ContentBlock>,
        sender: &SenderContext,
    ) -> Result<String, String> {
        let text: String = blocks
            .iter()
            .filter_map(|b| match b {
                librefang_types::message::ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = if text.is_empty() {
            "[Image]".to_string()
        } else {
            text
        };
        let result = self
            .kernel
            .send_message_with_blocks_and_sender(agent_id, &text, blocks, sender)
            .await
            .map_err(|e| format!("{e}"))?;
        Ok(result.response)
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        Ok(self.kernel.registry.find_by_name(name).map(|e| e.id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self
            .kernel
            .registry
            .list()
            .iter()
            .map(|e| (e.id, e.name.clone()))
            .collect())
    }

    async fn spawn_agent_by_name(&self, manifest_name: &str) -> Result<AgentId, String> {
        // Look for manifest at ~/.librefang/agents/{name}/agent.toml
        let manifest_path = self
            .kernel
            .config
            .home_dir
            .join("agents")
            .join(manifest_name)
            .join("agent.toml");

        if !manifest_path.exists() {
            return Err(format!("Manifest not found: {}", manifest_path.display()));
        }

        let contents = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read manifest: {e}"))?;

        let manifest: librefang_types::agent::AgentManifest =
            toml::from_str(&contents).map_err(|e| format!("Invalid manifest TOML: {e}"))?;

        let agent_id = self
            .kernel
            .spawn_agent(manifest)
            .map_err(|e| format!("Failed to spawn agent: {e}"))?;

        Ok(agent_id)
    }

    async fn uptime_info(&self) -> String {
        let uptime = self.started_at.elapsed();
        let agents = self.list_agents().await.unwrap_or_default();
        let secs = uptime.as_secs();
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if hours > 0 {
            format!(
                "LibreFang status: {}h {}m uptime, {} agent(s)",
                hours,
                mins,
                agents.len()
            )
        } else {
            format!(
                "LibreFang status: {}m uptime, {} agent(s)",
                mins,
                agents.len()
            )
        }
    }

    async fn list_models_text(&self) -> String {
        let catalog = self
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let available = catalog.available_models();
        if available.is_empty() {
            return "No models available. Configure API keys to enable providers.".to_string();
        }
        let mut msg = format!("Available models ({}):\n", available.len());
        // Group by provider
        let mut by_provider: std::collections::HashMap<
            &str,
            Vec<&librefang_types::model_catalog::ModelCatalogEntry>,
        > = std::collections::HashMap::new();
        for m in &available {
            by_provider.entry(m.provider.as_str()).or_default().push(m);
        }
        let mut providers: Vec<&&str> = by_provider.keys().collect();
        providers.sort();
        for provider in providers {
            let provider_name = catalog
                .get_provider(provider)
                .map(|p| p.display_name.as_str())
                .unwrap_or(provider);
            msg.push_str(&format!("\n{}:\n", provider_name));
            for m in &by_provider[provider] {
                let cost = if m.input_cost_per_m > 0.0 {
                    format!(
                        " (${:.2}/${:.2} per M)",
                        m.input_cost_per_m, m.output_cost_per_m
                    )
                } else {
                    " (free/local)".to_string()
                };
                msg.push_str(&format!("  {} — {}{}\n", m.id, m.display_name, cost));
            }
        }
        msg
    }

    async fn list_providers_text(&self) -> String {
        let catalog = self
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let mut msg = "Providers:\n".to_string();
        for p in catalog.list_providers() {
            let status = match p.auth_status {
                librefang_types::model_catalog::AuthStatus::Configured => "configured",
                librefang_types::model_catalog::AuthStatus::Missing => "not configured",
                librefang_types::model_catalog::AuthStatus::NotRequired => "local (no key needed)",
            };
            msg.push_str(&format!(
                "  {} — {} [{}, {} model(s)]\n",
                p.id, p.display_name, status, p.model_count
            ));
        }
        msg
    }

    async fn list_skills_text(&self) -> String {
        let skills = self
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let skills = skills.list();
        if skills.is_empty() {
            return "No skills installed. Place skills in ~/.librefang/skills/ or install from the marketplace.".to_string();
        }
        let mut msg = format!("Installed skills ({}):\n", skills.len());
        for skill in &skills {
            let runtime = format!("{:?}", skill.manifest.runtime.runtime_type);
            let tools_count = skill.manifest.tools.provided.len();
            let enabled = if skill.enabled { "" } else { " [disabled]" };
            msg.push_str(&format!(
                "  {} — {} ({}, {} tool(s)){}\n",
                skill.manifest.skill.name,
                skill.manifest.skill.description,
                runtime,
                tools_count,
                enabled,
            ));
        }
        msg
    }

    async fn list_hands_text(&self) -> String {
        let defs = self.kernel.hand_registry.list_definitions();
        if defs.is_empty() {
            return "No hands available.".to_string();
        }
        let instances = self.kernel.hand_registry.list_instances();
        let mut msg = format!("Available hands ({}):\n", defs.len());
        for d in &defs {
            let reqs_met = self
                .kernel
                .hand_registry
                .check_requirements(&d.id)
                .map(|r| r.iter().all(|(_, ok)| *ok))
                .unwrap_or(false);
            let badge = if reqs_met { "Ready" } else { "Setup needed" };
            msg.push_str(&format!(
                "  {} {} — {} [{}]\n",
                d.icon, d.name, d.description, badge
            ));
        }
        if !instances.is_empty() {
            msg.push_str(&format!("\nActive ({}):\n", instances.len()));
            for i in &instances {
                msg.push_str(&format!(
                    "  {} — {} ({})\n",
                    i.agent_name, i.hand_id, i.status
                ));
            }
        }
        msg
    }

    // ── Automation: workflows, triggers, schedules, approvals ──

    async fn list_workflows_text(&self) -> String {
        let workflows = self.kernel.workflows.list_workflows().await;
        if workflows.is_empty() {
            return "No workflows defined.".to_string();
        }
        let mut msg = format!("Workflows ({}):\n", workflows.len());
        for wf in &workflows {
            let steps = wf.steps.len();
            let desc = if wf.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", wf.description)
            };
            msg.push_str(&format!("  {} ({} step(s)){}\n", wf.name, steps, desc));
        }
        msg
    }

    async fn run_workflow_text(&self, name: &str, input: &str) -> String {
        let workflows = self.kernel.workflows.list_workflows().await;
        let wf = match workflows.iter().find(|w| w.name.eq_ignore_ascii_case(name)) {
            Some(w) => w.clone(),
            None => return format!("Workflow '{name}' not found. Use /workflows to list."),
        };

        let run_id = match self
            .kernel
            .workflows
            .create_run(wf.id, input.to_string())
            .await
        {
            Some(id) => id,
            None => return "Failed to create workflow run.".to_string(),
        };

        let kernel = self.kernel.clone();
        let registry_ref = &self.kernel.registry;
        let result = self
            .kernel
            .workflows
            .execute_run(
                run_id,
                |step_agent| match step_agent {
                    librefang_kernel::workflow::StepAgent::ById { id } => {
                        let aid: AgentId = id.parse().ok()?;
                        let entry = registry_ref.get(aid)?;
                        Some((aid, entry.name.clone()))
                    }
                    librefang_kernel::workflow::StepAgent::ByName { name } => {
                        let entry = registry_ref.find_by_name(name)?;
                        Some((entry.id, entry.name.clone()))
                    }
                },
                |agent_id, message| {
                    let k = kernel.clone();
                    async move {
                        let result = k
                            .send_message(agent_id, &message)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok((
                            result.response,
                            result.total_usage.input_tokens,
                            result.total_usage.output_tokens,
                        ))
                    }
                },
            )
            .await;

        match result {
            Ok(output) => format!("Workflow '{}' completed:\n{}", wf.name, output),
            Err(e) => format!("Workflow '{}' failed: {}", wf.name, e),
        }
    }

    async fn list_triggers_text(&self) -> String {
        let triggers = self.kernel.triggers.list_all();
        if triggers.is_empty() {
            return "No triggers configured.".to_string();
        }
        let mut msg = format!("Triggers ({}):\n", triggers.len());
        for t in &triggers {
            let agent_name = self
                .kernel
                .registry
                .get(t.agent_id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| t.agent_id.to_string());
            let status = if t.enabled { "on" } else { "off" };
            let id_str = t.id.0.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            msg.push_str(&format!(
                "  [{}] {} -> {} ({:?}) fires:{} [{}]\n",
                id_short,
                agent_name,
                t.prompt_template.chars().take(40).collect::<String>(),
                t.pattern,
                t.fire_count,
                status,
            ));
        }
        msg
    }

    async fn create_trigger_text(
        &self,
        agent_name: &str,
        pattern_str: &str,
        prompt: &str,
    ) -> String {
        let agent = match self.kernel.registry.find_by_name(agent_name) {
            Some(e) => e,
            None => return format!("Agent '{agent_name}' not found."),
        };

        let pattern = match parse_trigger_pattern(pattern_str) {
            Some(p) => p,
            None => {
                return format!(
                "Unknown pattern '{pattern_str}'. Valid: lifecycle, spawned:<name>, terminated, \
                 system, system:<keyword>, memory, memory:<key>, match:<text>, all"
            )
            }
        };

        let trigger_id = self
            .kernel
            .triggers
            .register(agent.id, pattern, prompt.to_string(), 0);
        let id_str = trigger_id.0.to_string();
        let id_short = safe_truncate_str(&id_str, 8);
        format!("Trigger created [{id_short}] for agent '{agent_name}'.")
    }

    async fn delete_trigger_text(&self, id_prefix: &str) -> String {
        let triggers = self.kernel.triggers.list_all();
        let matched: Vec<_> = triggers
            .iter()
            .filter(|t| t.id.0.to_string().starts_with(id_prefix))
            .collect();
        match matched.len() {
            0 => format!("No trigger found matching '{id_prefix}'."),
            1 => {
                let t = matched[0];
                if self.kernel.triggers.remove(t.id) {
                    let id_str = t.id.0.to_string();
                    format!("Trigger [{}] removed.", safe_truncate_str(&id_str, 8))
                } else {
                    "Failed to remove trigger.".to_string()
                }
            }
            n => format!("{n} triggers match '{id_prefix}'. Be more specific."),
        }
    }

    async fn list_schedules_text(&self) -> String {
        let jobs = self.kernel.cron_scheduler.list_all_jobs();
        if jobs.is_empty() {
            return "No scheduled jobs.".to_string();
        }
        let mut msg = format!("Cron jobs ({}):\n", jobs.len());
        for job in &jobs {
            let agent_name = self
                .kernel
                .registry
                .get(job.agent_id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| job.agent_id.to_string());
            let status = if job.enabled { "on" } else { "off" };
            let id_str = job.id.0.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            let sched = match &job.schedule {
                librefang_types::scheduler::CronSchedule::Cron { expr, .. } => expr.clone(),
                librefang_types::scheduler::CronSchedule::Every { every_secs } => {
                    format!("every {every_secs}s")
                }
                librefang_types::scheduler::CronSchedule::At { at } => {
                    format!("at {}", at.format("%Y-%m-%d %H:%M"))
                }
            };
            let last = job
                .last_run
                .map(|t| t.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".to_string());
            msg.push_str(&format!(
                "  [{}] {} — {} ({}) last:{} [{}]\n",
                id_short, job.name, sched, agent_name, last, status,
            ));
        }
        msg
    }

    async fn manage_schedule_text(&self, action: &str, args: &[String]) -> String {
        match action {
            "add" => {
                // Expected: <agent> <f1> <f2> <f3> <f4> <f5> <message...>
                // 5 cron fields: min hour dom month dow
                if args.len() < 7 {
                    return "Usage: /schedule add <agent> <min> <hour> <dom> <month> <dow> <message>".to_string();
                }
                let agent_name = &args[0];
                let agent = match self.kernel.registry.find_by_name(agent_name) {
                    Some(e) => e,
                    None => return format!("Agent '{agent_name}' not found."),
                };
                let cron_expr = args[1..6].join(" ");
                let message = args[6..].join(" ");

                let job = librefang_types::scheduler::CronJob {
                    id: librefang_types::scheduler::CronJobId::new(),
                    agent_id: agent.id,
                    name: format!("chat-{}", &agent.name),
                    enabled: true,
                    schedule: librefang_types::scheduler::CronSchedule::Cron {
                        expr: cron_expr.clone(),
                        tz: None,
                    },
                    action: librefang_types::scheduler::CronAction::AgentTurn {
                        message: message.clone(),
                        model_override: None,
                        timeout_secs: None,
                    },
                    delivery: librefang_types::scheduler::CronDelivery::None,
                    created_at: chrono::Utc::now(),
                    last_run: None,
                    next_run: None,
                };

                match self.kernel.cron_scheduler.add_job(job, false) {
                    Ok(id) => {
                        let id_str = id.0.to_string();
                        let id_short = safe_truncate_str(&id_str, 8);
                        format!("Job [{id_short}] created: '{cron_expr}' -> {agent_name}: \"{message}\"")
                    }
                    Err(e) => format!("Failed to create job: {e}"),
                }
            }
            "del" => {
                if args.is_empty() {
                    return "Usage: /schedule del <id-prefix>".to_string();
                }
                let prefix = &args[0];
                let jobs = self.kernel.cron_scheduler.list_all_jobs();
                let matched: Vec<_> = jobs
                    .iter()
                    .filter(|j| j.id.0.to_string().starts_with(prefix.as_str()))
                    .collect();
                match matched.len() {
                    0 => format!("No job found matching '{prefix}'."),
                    1 => {
                        let j = matched[0];
                        match self.kernel.cron_scheduler.remove_job(j.id) {
                            Ok(_) => {
                                let id_str = j.id.0.to_string();
                                format!(
                                    "Job [{}] '{}' removed.",
                                    safe_truncate_str(&id_str, 8),
                                    j.name
                                )
                            }
                            Err(e) => format!("Failed to remove job: {e}"),
                        }
                    }
                    n => format!("{n} jobs match '{prefix}'. Be more specific."),
                }
            }
            "run" => {
                if args.is_empty() {
                    return "Usage: /schedule run <id-prefix>".to_string();
                }
                let prefix = &args[0];
                let jobs = self.kernel.cron_scheduler.list_all_jobs();
                let matched: Vec<_> = jobs
                    .iter()
                    .filter(|j| j.id.0.to_string().starts_with(prefix.as_str()))
                    .collect();
                match matched.len() {
                    0 => format!("No job found matching '{prefix}'."),
                    1 => {
                        let j = matched[0];
                        let id_str = j.id.0.to_string();
                        let id_short = safe_truncate_str(&id_str, 8);
                        match &j.action {
                            librefang_types::scheduler::CronAction::AgentTurn {
                                message, ..
                            } => match self.kernel.send_message(j.agent_id, message).await {
                                Ok(result) => {
                                    format!("Job [{id_short}] ran:\n{}", result.response)
                                }
                                Err(e) => format!("Failed to run job: {e}"),
                            },
                            librefang_types::scheduler::CronAction::SystemEvent { text } => {
                                match self.kernel.send_message(j.agent_id, text).await {
                                    Ok(result) => {
                                        format!("Job [{id_short}] ran:\n{}", result.response)
                                    }
                                    Err(e) => format!("Failed to run job: {e}"),
                                }
                            }
                            librefang_types::scheduler::CronAction::Workflow {
                                workflow_id,
                                input,
                                ..
                            } => {
                                // Resolve workflow by UUID or name
                                let resolved = if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id)
                                {
                                    Some(librefang_kernel::workflow::WorkflowId(uuid))
                                } else {
                                    let workflows = self.kernel.workflows.list_workflows().await;
                                    workflows
                                        .iter()
                                        .find(|w| w.name == *workflow_id)
                                        .map(|w| w.id)
                                };
                                match resolved {
                                    Some(wf_id) => {
                                        let input_text = input.clone().unwrap_or_default();
                                        match self.kernel.run_workflow(wf_id, input_text).await {
                                            Ok((_run_id, output)) => {
                                                format!(
                                                    "Job [{id_short}] workflow ran:\n{}",
                                                    output
                                                )
                                            }
                                            Err(e) => format!("Failed to run workflow: {e}"),
                                        }
                                    }
                                    None => format!("Workflow not found: {workflow_id}"),
                                }
                            }
                        }
                    }
                    n => format!("{n} jobs match '{prefix}'. Be more specific."),
                }
            }
            _ => "Unknown schedule action. Use: add, del, run".to_string(),
        }
    }

    async fn list_approvals_text(&self) -> String {
        let pending = self.kernel.approval_manager.list_pending();
        if pending.is_empty() {
            return "No pending approvals.".to_string();
        }
        let mut msg = format!("Pending approvals ({}):\n", pending.len());
        for req in &pending {
            let id_str = req.id.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            let age_secs = (chrono::Utc::now() - req.requested_at).num_seconds();
            let age = if age_secs >= 60 {
                format!("{}m", age_secs / 60)
            } else {
                format!("{age_secs}s")
            };
            msg.push_str(&format!(
                "  [{}] {} — {} ({:?}) age:{}\n",
                id_short, req.agent_id, req.tool_name, req.risk_level, age,
            ));
            if !req.action_summary.is_empty() {
                msg.push_str(&format!("    {}\n", req.action_summary));
            }
        }
        msg.push_str("\nUse /approve <id> or /reject <id>");
        msg
    }

    async fn resolve_approval_text(&self, id_prefix: &str, approve: bool) -> String {
        let pending = self.kernel.approval_manager.list_pending();
        let matched: Vec<_> = pending
            .iter()
            .filter(|r| r.id.to_string().starts_with(id_prefix))
            .collect();
        match matched.len() {
            0 => format!("No pending approval matching '{id_prefix}'."),
            1 => {
                let req = matched[0];
                let decision = if approve {
                    librefang_types::approval::ApprovalDecision::Approved
                } else {
                    librefang_types::approval::ApprovalDecision::Denied
                };
                match self.kernel.approval_manager.resolve(
                    req.id,
                    decision,
                    Some("channel".to_string()),
                ) {
                    Ok(_) => {
                        let verb = if approve { "Approved" } else { "Rejected" };
                        let id_str = req.id.to_string();
                        format!(
                            "{} [{}] {} — {}",
                            verb,
                            safe_truncate_str(&id_str, 8),
                            req.tool_name,
                            req.agent_id
                        )
                    }
                    Err(e) => format!("Failed to resolve approval: {e}"),
                }
            }
            n => format!("{n} approvals match '{id_prefix}'. Be more specific."),
        }
    }

    async fn reset_session(&self, agent_id: AgentId) -> Result<String, String> {
        self.kernel
            .reset_session(agent_id)
            .map_err(|e| format!("{e}"))?;
        Ok("Session reset. Chat history cleared.".to_string())
    }

    async fn reboot_session(&self, agent_id: AgentId) -> Result<String, String> {
        self.kernel
            .reboot_session(agent_id)
            .map_err(|e| format!("{e}"))?;
        Ok("Session rebooted. Context cleared.".to_string())
    }

    async fn compact_session(&self, agent_id: AgentId) -> Result<String, String> {
        self.kernel
            .compact_agent_session(agent_id)
            .await
            .map_err(|e| format!("{e}"))
    }

    async fn set_model(&self, agent_id: AgentId, model: &str) -> Result<String, String> {
        if model.is_empty() {
            // Show current model
            let entry = self
                .kernel
                .registry
                .get(agent_id)
                .ok_or_else(|| "Agent not found".to_string())?;
            return Ok(format!(
                "Current model: {} (provider: {})",
                entry.manifest.model.model, entry.manifest.model.provider
            ));
        }
        self.kernel
            .set_agent_model(agent_id, model, None)
            .map_err(|e| format!("{e}"))?;
        // Read back resolved model+provider from registry
        let entry = self
            .kernel
            .registry
            .get(agent_id)
            .ok_or_else(|| "Agent not found after model switch".to_string())?;
        Ok(format!(
            "Model switched to: {} (provider: {})",
            entry.manifest.model.model, entry.manifest.model.provider
        ))
    }

    async fn stop_run(&self, agent_id: AgentId) -> Result<String, String> {
        let cancelled = self
            .kernel
            .stop_agent_run(agent_id)
            .map_err(|e| format!("{e}"))?;
        if cancelled {
            Ok("Run cancelled.".to_string())
        } else {
            Ok("No active run to cancel.".to_string())
        }
    }

    async fn session_usage(&self, agent_id: AgentId) -> Result<String, String> {
        let (input, output, cost) = self
            .kernel
            .session_usage_cost(agent_id)
            .map_err(|e| format!("{e}"))?;
        let total = input + output;
        let mut msg = format!("Session usage:\n  Input: ~{input} tokens\n  Output: ~{output} tokens\n  Total: ~{total} tokens");
        if cost > 0.0 {
            msg.push_str(&format!("\n  Estimated cost: ${cost:.4}"));
        }
        Ok(msg)
    }

    async fn set_thinking(&self, _agent_id: AgentId, on: bool) -> Result<String, String> {
        // Future-ready: stores preference but doesn't affect model behavior yet
        let state = if on { "enabled" } else { "disabled" };
        Ok(format!(
            "Extended thinking {state}. (This will take effect when supported by the model.)"
        ))
    }

    async fn channel_overrides(
        &self,
        channel_type: &str,
        account_id: Option<&str>,
    ) -> Option<librefang_types::config::ChannelOverrides> {
        let channels = &self.kernel.config.channels;

        /// Look up channel overrides, preferring the entry whose `account_id`
        /// matches the message's account_id. Falls back to the first entry
        /// when no account_id is provided.
        macro_rules! find_overrides {
            ($field:ident) => {
                if let Some(aid) = account_id {
                    channels
                        .$field
                        .iter()
                        .find(|c| c.account_id.as_deref() == Some(aid))
                        .map(|c| c.overrides.clone())
                } else {
                    channels.$field.first().map(|c| c.overrides.clone())
                }
            };
        }

        match channel_type {
            "telegram" => find_overrides!(telegram),
            "discord" => find_overrides!(discord),
            "slack" => find_overrides!(slack),
            "whatsapp" => find_overrides!(whatsapp),
            "signal" => find_overrides!(signal),
            "matrix" => find_overrides!(matrix),
            "email" => find_overrides!(email),
            "teams" => find_overrides!(teams),
            "mattermost" => find_overrides!(mattermost),
            "irc" => find_overrides!(irc),
            "google_chat" => find_overrides!(google_chat),
            "twitch" => find_overrides!(twitch),
            "rocketchat" => find_overrides!(rocketchat),
            "zulip" => find_overrides!(zulip),
            "xmpp" => find_overrides!(xmpp),
            // Wave 3
            "line" => find_overrides!(line),
            "viber" => find_overrides!(viber),
            "messenger" => find_overrides!(messenger),
            "reddit" => find_overrides!(reddit),
            "mastodon" => find_overrides!(mastodon),
            "bluesky" => find_overrides!(bluesky),
            "feishu" => find_overrides!(feishu),
            "revolt" => find_overrides!(revolt),
            // Wave 4
            "nextcloud" => find_overrides!(nextcloud),
            "guilded" => find_overrides!(guilded),
            "keybase" => find_overrides!(keybase),
            "threema" => find_overrides!(threema),
            "nostr" => find_overrides!(nostr),
            "webex" => find_overrides!(webex),
            "pumble" => find_overrides!(pumble),
            "flock" => find_overrides!(flock),
            "twist" => find_overrides!(twist),
            // Wave 5
            "mumble" => find_overrides!(mumble),
            "dingtalk" => find_overrides!(dingtalk),
            "discourse" => find_overrides!(discourse),
            "gitter" => find_overrides!(gitter),
            "ntfy" => find_overrides!(ntfy),
            "gotify" => find_overrides!(gotify),
            "webhook" => find_overrides!(webhook),
            "linkedin" => find_overrides!(linkedin),
            "wecom" => find_overrides!(wecom),
            _ => None,
        }
    }

    async fn authorize_channel_user(
        &self,
        channel_type: &str,
        platform_id: &str,
        action: &str,
    ) -> Result<(), String> {
        if !self.kernel.auth.is_enabled() {
            return Ok(()); // RBAC not configured — allow all
        }

        let user_id = self
            .kernel
            .auth
            .identify(channel_type, platform_id)
            .ok_or_else(|| "Unrecognized user. Contact an admin to get access.".to_string())?;

        let auth_action = match action {
            "chat" => librefang_kernel::auth::Action::ChatWithAgent,
            "spawn" => librefang_kernel::auth::Action::SpawnAgent,
            "kill" => librefang_kernel::auth::Action::KillAgent,
            "install_skill" => librefang_kernel::auth::Action::InstallSkill,
            _ => librefang_kernel::auth::Action::ChatWithAgent,
        };

        self.kernel
            .auth
            .authorize(user_id, &auth_action)
            .map_err(|e| e.to_string())
    }

    async fn record_delivery(
        &self,
        agent_id: AgentId,
        channel: &str,
        recipient: &str,
        success: bool,
        error: Option<&str>,
        thread_id: Option<&str>,
    ) {
        let receipt = if success {
            librefang_kernel::DeliveryTracker::sent_receipt(channel, recipient)
        } else {
            librefang_kernel::DeliveryTracker::failed_receipt(
                channel,
                recipient,
                error.unwrap_or("Unknown error"),
            )
        };
        self.kernel.delivery_tracker.record(agent_id, receipt);

        // Persist last channel for cron CronDelivery::LastChannel.
        // Include thread_id when present so forum-topic context survives restarts.
        if success {
            let mut kv_val = serde_json::json!({"channel": channel, "recipient": recipient});
            if let Some(tid) = thread_id {
                kv_val["thread_id"] = serde_json::json!(tid);
            }
            let _ = self
                .kernel
                .memory
                .structured_set(agent_id, "delivery.last_channel", kv_val);
        }
    }

    async fn check_auto_reply(&self, agent_id: AgentId, message: &str) -> Option<String> {
        // Check if auto-reply should fire for this message
        let channel_type = "bridge"; // Generic; the bridge layer handles specifics
        self.kernel
            .auto_reply_engine
            .should_reply(message, channel_type, agent_id)?;
        // Fire auto-reply synchronously (bridge already runs in background task)
        match self.kernel.send_message(agent_id, message).await {
            Ok(result) => Some(result.response),
            Err(e) => {
                tracing::warn!(error = %e, "Auto-reply failed");
                None
            }
        }
    }

    // ── Budget, Network, A2A ──

    async fn budget_text(&self) -> String {
        let budget = &self.kernel.config.budget;
        let status = self.kernel.metering.budget_status(budget);

        let fmt_limit = |v: f64| -> String {
            if v > 0.0 {
                format!("${v:.2}")
            } else {
                "unlimited".to_string()
            }
        };
        let fmt_pct = |pct: f64, limit: f64| -> String {
            if limit > 0.0 {
                format!(" ({:.1}%)", pct * 100.0)
            } else {
                String::new()
            }
        };

        format!(
            "Budget Status:\n\
             \n\
             Hourly:  ${:.4} / {}{}\n\
             Daily:   ${:.4} / {}{}\n\
             Monthly: ${:.4} / {}{}\n\
             \n\
             Alert threshold: {}%",
            status.hourly_spend,
            fmt_limit(status.hourly_limit),
            fmt_pct(status.hourly_pct, status.hourly_limit),
            status.daily_spend,
            fmt_limit(status.daily_limit),
            fmt_pct(status.daily_pct, status.daily_limit),
            status.monthly_spend,
            fmt_limit(status.monthly_limit),
            fmt_pct(status.monthly_pct, status.monthly_limit),
            (status.alert_threshold * 100.0) as u32,
        )
    }

    async fn peers_text(&self) -> String {
        if !self.kernel.config.network_enabled {
            return "OFP peer network is disabled. Set network_enabled = true in config.toml."
                .to_string();
        }
        match self.kernel.peer_registry.get() {
            Some(registry) => {
                let peers = registry.all_peers();
                if peers.is_empty() {
                    "OFP network enabled but no peers connected.".to_string()
                } else {
                    let mut msg = format!("OFP Peers ({} connected):\n", peers.len());
                    for p in &peers {
                        msg.push_str(&format!(
                            "  {} — {} ({:?})\n",
                            p.node_id, p.address, p.state
                        ));
                    }
                    msg
                }
            }
            None => "OFP peer node not started.".to_string(),
        }
    }

    async fn a2a_agents_text(&self) -> String {
        let agents = self
            .kernel
            .a2a_external_agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if agents.is_empty() {
            return "No external A2A agents discovered.\nUse the dashboard or API to discover agents.".to_string();
        }
        let mut msg = format!("External A2A Agents ({}):\n", agents.len());
        for (url, card) in agents.iter() {
            msg.push_str(&format!("  {} — {}\n", card.name, url));
            let desc = &card.description;
            if !desc.is_empty() {
                let short = librefang_types::truncate_str(desc, 60);
                msg.push_str(&format!("    {short}\n"));
            }
        }
        msg
    }
}

/// Parse a trigger pattern string from chat into a `TriggerPattern`.
fn parse_trigger_pattern(s: &str) -> Option<librefang_kernel::triggers::TriggerPattern> {
    use librefang_kernel::triggers::TriggerPattern;
    if let Some(rest) = s.strip_prefix("spawned:") {
        return Some(TriggerPattern::AgentSpawned {
            name_pattern: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("system:") {
        return Some(TriggerPattern::SystemKeyword {
            keyword: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("memory:") {
        return Some(TriggerPattern::MemoryKeyPattern {
            key_pattern: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("match:") {
        return Some(TriggerPattern::ContentMatch {
            substring: rest.to_string(),
        });
    }
    match s {
        "lifecycle" => Some(TriggerPattern::Lifecycle),
        "terminated" => Some(TriggerPattern::AgentTerminated),
        "system" => Some(TriggerPattern::System),
        "memory" => Some(TriggerPattern::MemoryUpdate),
        "all" => Some(TriggerPattern::All),
        _ => None,
    }
}

/// Read a token from an env var, returning None with a warning if missing/empty.
#[allow(dead_code)]
fn read_token(env_var: &str, adapter_name: &str) -> Option<String> {
    match std::env::var(env_var) {
        Ok(t) if !t.is_empty() => Some(t),
        Ok(_) => {
            warn!("{adapter_name} bot token env var '{env_var}' is empty, skipping");
            None
        }
        Err(_) => {
            warn!("{adapter_name} bot token env var '{env_var}' not set, skipping");
            None
        }
    }
}

/// Start the channel bridge for all configured channels based on kernel config.
///
/// Returns `Some(BridgeManager)` if any channels were configured and started,
/// or `None` if no channels are configured.
pub async fn start_channel_bridge(kernel: Arc<LibreFangKernel>) -> Option<BridgeManager> {
    let channels = kernel.config.channels.clone();
    let (bridge, _names) = start_channel_bridge_with_config(kernel, &channels).await;
    bridge
}

/// Start channels from an explicit `ChannelsConfig` (used by hot-reload).
///
/// Returns `(Option<BridgeManager>, Vec<started_channel_names>)`.
pub async fn start_channel_bridge_with_config(
    kernel: Arc<LibreFangKernel>,
    config: &librefang_types::config::ChannelsConfig,
) -> (Option<BridgeManager>, Vec<String>) {
    // Check which channels have config — only consider enabled features
    #[allow(unused_mut)]
    let mut has_any = false;

    // Emit warnings for configured-but-disabled channels, track enabled ones
    macro_rules! check_channel {
        ($field:ident, $feature:literal, $name:expr) => {
            #[cfg(feature = $feature)]
            if config.$field.is_some() {
                has_any = true;
            }
            #[cfg(not(feature = $feature))]
            if config.$field.is_some() {
                warn!(
                    "{} channel configured but '{}' feature is not enabled — skipping",
                    $name, $feature
                );
            }
        };
    }

    check_channel!(telegram, "channel-telegram", "Telegram");
    check_channel!(discord, "channel-discord", "Discord");
    check_channel!(slack, "channel-slack", "Slack");
    check_channel!(whatsapp, "channel-whatsapp", "WhatsApp");
    check_channel!(signal, "channel-signal", "Signal");
    check_channel!(matrix, "channel-matrix", "Matrix");
    check_channel!(email, "channel-email", "Email");
    check_channel!(teams, "channel-teams", "Teams");
    check_channel!(mattermost, "channel-mattermost", "Mattermost");
    check_channel!(irc, "channel-irc", "IRC");
    check_channel!(google_chat, "channel-google-chat", "Google Chat");
    check_channel!(twitch, "channel-twitch", "Twitch");
    check_channel!(rocketchat, "channel-rocketchat", "Rocket.Chat");
    check_channel!(zulip, "channel-zulip", "Zulip");
    check_channel!(xmpp, "channel-xmpp", "XMPP");
    check_channel!(line, "channel-line", "LINE");
    check_channel!(viber, "channel-viber", "Viber");
    check_channel!(messenger, "channel-messenger", "Messenger");
    check_channel!(reddit, "channel-reddit", "Reddit");
    check_channel!(mastodon, "channel-mastodon", "Mastodon");
    check_channel!(bluesky, "channel-bluesky", "Bluesky");
    check_channel!(feishu, "channel-feishu", "Feishu");
    check_channel!(revolt, "channel-revolt", "Revolt");
    check_channel!(wecom, "channel-wecom", "WeCom");
    check_channel!(nextcloud, "channel-nextcloud", "Nextcloud");
    check_channel!(guilded, "channel-guilded", "Guilded");
    check_channel!(keybase, "channel-keybase", "Keybase");
    check_channel!(threema, "channel-threema", "Threema");
    check_channel!(nostr, "channel-nostr", "Nostr");
    check_channel!(webex, "channel-webex", "Webex");
    check_channel!(pumble, "channel-pumble", "Pumble");
    check_channel!(flock, "channel-flock", "Flock");
    check_channel!(twist, "channel-twist", "Twist");
    check_channel!(mumble, "channel-mumble", "Mumble");
    check_channel!(dingtalk, "channel-dingtalk", "DingTalk");
    check_channel!(qq, "channel-qq", "QQ");
    check_channel!(discourse, "channel-discourse", "Discourse");
    check_channel!(gitter, "channel-gitter", "Gitter");
    check_channel!(ntfy, "channel-ntfy", "ntfy");
    check_channel!(gotify, "channel-gotify", "Gotify");
    check_channel!(webhook, "channel-webhook", "Webhook");
    check_channel!(linkedin, "channel-linkedin", "LinkedIn");

    // Sidecar channels (always available, not feature-gated)
    if !kernel.config.sidecar_channels.is_empty() {
        has_any = true;
    }

    if !has_any {
        return (None, Vec::new());
    }

    let handle = KernelBridgeAdapter {
        kernel: kernel.clone(),
        started_at: Instant::now(),
    };

    // Collect all adapters to start: (adapter, default_agent_name, account_id)
    #[allow(unused_mut, clippy::type_complexity)]
    let mut adapters: Vec<(Arc<dyn ChannelAdapter>, Option<String>, Option<String>)> = Vec::new();

    // Telegram
    #[cfg(feature = "channel-telegram")]
    for tg_config in config.telegram.iter() {
        if let Some(token) = read_token(&tg_config.bot_token_env, "Telegram") {
            let poll_interval = Duration::from_secs(tg_config.poll_interval_secs);
            let adapter = Arc::new(
                TelegramAdapter::new(
                    token,
                    tg_config.allowed_users.clone(),
                    poll_interval,
                    tg_config.api_url.clone(),
                )
                .with_account_id(tg_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                tg_config.default_agent.clone(),
                tg_config.account_id.clone(),
            ));
        }
    }

    // Discord
    #[cfg(feature = "channel-discord")]
    for dc_config in config.discord.iter() {
        if let Some(token) = read_token(&dc_config.bot_token_env, "Discord") {
            let adapter = Arc::new(
                DiscordAdapter::new(
                    token,
                    dc_config.allowed_guilds.clone(),
                    dc_config.allowed_users.clone(),
                    dc_config.ignore_bots,
                    dc_config.intents,
                )
                .with_account_id(dc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                dc_config.default_agent.clone(),
                dc_config.account_id.clone(),
            ));
        }
    }

    // Slack
    #[cfg(feature = "channel-slack")]
    for sl_config in config.slack.iter() {
        if let Some(app_token) = read_token(&sl_config.app_token_env, "Slack (app)") {
            if let Some(bot_token) = read_token(&sl_config.bot_token_env, "Slack (bot)") {
                let adapter = Arc::new(
                    SlackAdapter::new(app_token, bot_token, sl_config.allowed_channels.clone())
                        .with_account_id(sl_config.account_id.clone()),
                );
                adapters.push((
                    adapter,
                    sl_config.default_agent.clone(),
                    sl_config.account_id.clone(),
                ));
            }
        }
    }

    // WhatsApp — supports Cloud API mode (access token) or Web/QR mode (gateway URL)
    #[cfg(feature = "channel-whatsapp")]
    for wa_config in config.whatsapp.iter() {
        let cloud_token = read_token(&wa_config.access_token_env, "WhatsApp");
        let gateway_url = std::env::var(&wa_config.gateway_url_env)
            .ok()
            .filter(|u| !u.is_empty());

        if cloud_token.is_some() || gateway_url.is_some() {
            let token = cloud_token.unwrap_or_default();
            let verify_token =
                read_token(&wa_config.verify_token_env, "WhatsApp (verify)").unwrap_or_default();
            let adapter = Arc::new(
                WhatsAppAdapter::new(
                    wa_config.phone_number_id.clone(),
                    token,
                    verify_token,
                    wa_config.webhook_port,
                    wa_config.allowed_users.clone(),
                )
                .with_gateway(gateway_url)
                .with_account_id(wa_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                wa_config.default_agent.clone(),
                wa_config.account_id.clone(),
            ));
        }
    }

    // Signal
    #[cfg(feature = "channel-signal")]
    for sig_config in config.signal.iter() {
        if !sig_config.phone_number.is_empty() {
            let adapter = Arc::new(
                SignalAdapter::new(
                    sig_config.api_url.clone(),
                    sig_config.phone_number.clone(),
                    sig_config.allowed_users.clone(),
                )
                .with_account_id(sig_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                sig_config.default_agent.clone(),
                sig_config.account_id.clone(),
            ));
        } else {
            warn!("Signal configured but phone_number is empty, skipping");
        }
    }

    // Matrix
    #[cfg(feature = "channel-matrix")]
    for mx_config in config.matrix.iter() {
        if let Some(token) = read_token(&mx_config.access_token_env, "Matrix") {
            let adapter = Arc::new(
                MatrixAdapter::new(
                    mx_config.homeserver_url.clone(),
                    mx_config.user_id.clone(),
                    token,
                    mx_config.allowed_rooms.clone(),
                    mx_config.auto_accept_invites,
                )
                .with_account_id(mx_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                mx_config.default_agent.clone(),
                mx_config.account_id.clone(),
            ));
        }
    }

    // Email
    #[cfg(feature = "channel-email")]
    for em_config in config.email.iter() {
        if let Some(password) = read_token(&em_config.password_env, "Email") {
            let adapter = Arc::new(
                EmailAdapter::new(
                    em_config.imap_host.clone(),
                    em_config.imap_port,
                    em_config.smtp_host.clone(),
                    em_config.smtp_port,
                    em_config.username.clone(),
                    password,
                    em_config.poll_interval_secs,
                    em_config.folders.clone(),
                    em_config.allowed_senders.clone(),
                )
                .with_account_id(em_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                em_config.default_agent.clone(),
                em_config.account_id.clone(),
            ));
        }
    }

    // Teams
    #[cfg(feature = "channel-teams")]
    for tm_config in config.teams.iter() {
        if let Some(password) = read_token(&tm_config.app_password_env, "Teams") {
            let adapter = Arc::new(
                TeamsAdapter::new(
                    tm_config.app_id.clone(),
                    password,
                    tm_config.webhook_port,
                    tm_config.allowed_tenants.clone(),
                )
                .with_account_id(tm_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                tm_config.default_agent.clone(),
                tm_config.account_id.clone(),
            ));
        }
    }

    // Mattermost
    #[cfg(feature = "channel-mattermost")]
    for mm_config in config.mattermost.iter() {
        if let Some(token) = read_token(&mm_config.token_env, "Mattermost") {
            let adapter = Arc::new(
                MattermostAdapter::new(
                    mm_config.server_url.clone(),
                    token,
                    mm_config.allowed_channels.clone(),
                )
                .with_account_id(mm_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                mm_config.default_agent.clone(),
                mm_config.account_id.clone(),
            ));
        }
    }

    // IRC
    #[cfg(feature = "channel-irc")]
    for irc_config in config.irc.iter() {
        if !irc_config.server.is_empty() {
            let password = irc_config
                .password_env
                .as_ref()
                .and_then(|env| read_token(env, "IRC"));
            let adapter = Arc::new(
                IrcAdapter::new(
                    irc_config.server.clone(),
                    irc_config.port,
                    irc_config.nick.clone(),
                    password,
                    irc_config.channels.clone(),
                    irc_config.use_tls,
                )
                .with_account_id(irc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                irc_config.default_agent.clone(),
                irc_config.account_id.clone(),
            ));
        } else {
            warn!("IRC configured but server is empty, skipping");
        }
    }

    // Google Chat
    #[cfg(feature = "channel-google-chat")]
    for gc_config in config.google_chat.iter() {
        // Try service_account_key_path first, then fall back to env var
        let key = gc_config
            .service_account_key_path
            .as_ref()
            .filter(|p| !p.is_empty())
            .and_then(|path| match std::fs::read_to_string(path) {
                Ok(contents) => Some(contents),
                Err(e) => {
                    warn!("Google Chat: failed to read service account key from {path}: {e}");
                    None
                }
            })
            .or_else(|| read_token(&gc_config.service_account_env, "Google Chat"));
        if let Some(key) = key {
            let adapter = Arc::new(
                GoogleChatAdapter::new(key, gc_config.space_ids.clone(), gc_config.webhook_port)
                    .with_account_id(gc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                gc_config.default_agent.clone(),
                gc_config.account_id.clone(),
            ));
        } else {
            warn!("Google Chat configured but no credentials found (neither service_account_key_path nor {} env var), skipping", gc_config.service_account_env);
        }
    }

    // Twitch
    #[cfg(feature = "channel-twitch")]
    for tw_config in config.twitch.iter() {
        if let Some(token) = read_token(&tw_config.oauth_token_env, "Twitch") {
            let adapter = Arc::new(
                TwitchAdapter::new(token, tw_config.channels.clone(), tw_config.nick.clone())
                    .with_account_id(tw_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                tw_config.default_agent.clone(),
                tw_config.account_id.clone(),
            ));
        }
    }

    // Rocket.Chat
    #[cfg(feature = "channel-rocketchat")]
    for rc_config in config.rocketchat.iter() {
        if let Some(token) = read_token(&rc_config.token_env, "Rocket.Chat") {
            let adapter = Arc::new(
                RocketChatAdapter::new(
                    rc_config.server_url.clone(),
                    token,
                    rc_config.user_id.clone(),
                    rc_config.allowed_channels.clone(),
                )
                .with_account_id(rc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                rc_config.default_agent.clone(),
                rc_config.account_id.clone(),
            ));
        }
    }

    // Zulip
    #[cfg(feature = "channel-zulip")]
    for z_config in config.zulip.iter() {
        if let Some(api_key) = read_token(&z_config.api_key_env, "Zulip") {
            let adapter = Arc::new(
                ZulipAdapter::new(
                    z_config.server_url.clone(),
                    z_config.bot_email.clone(),
                    api_key,
                    z_config.streams.clone(),
                )
                .with_account_id(z_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                z_config.default_agent.clone(),
                z_config.account_id.clone(),
            ));
        }
    }

    // XMPP
    #[cfg(feature = "channel-xmpp")]
    for x_config in config.xmpp.iter() {
        if let Some(password) = read_token(&x_config.password_env, "XMPP") {
            let adapter = Arc::new(
                XmppAdapter::new(
                    x_config.jid.clone(),
                    password,
                    x_config.server.clone(),
                    x_config.port,
                    x_config.rooms.clone(),
                )
                .with_account_id(x_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                x_config.default_agent.clone(),
                x_config.account_id.clone(),
            ));
        }
    }

    // ── Wave 3 ──────────────────────────────────────────────────

    // LINE
    #[cfg(feature = "channel-line")]
    for ln_config in config.line.iter() {
        if let Some(secret) = read_token(&ln_config.channel_secret_env, "LINE (secret)") {
            if let Some(token) = read_token(&ln_config.access_token_env, "LINE (token)") {
                let adapter = Arc::new(
                    LineAdapter::new(secret, token, ln_config.webhook_port)
                        .with_account_id(ln_config.account_id.clone()),
                );
                adapters.push((
                    adapter,
                    ln_config.default_agent.clone(),
                    ln_config.account_id.clone(),
                ));
            }
        }
    }

    // Viber
    #[cfg(feature = "channel-viber")]
    for vb_config in config.viber.iter() {
        if let Some(token) = read_token(&vb_config.auth_token_env, "Viber") {
            let adapter = Arc::new(
                ViberAdapter::new(token, vb_config.webhook_url.clone(), vb_config.webhook_port)
                    .with_account_id(vb_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                vb_config.default_agent.clone(),
                vb_config.account_id.clone(),
            ));
        }
    }

    // Facebook Messenger
    #[cfg(feature = "channel-messenger")]
    for ms_config in config.messenger.iter() {
        if let Some(page_token) = read_token(&ms_config.page_token_env, "Messenger (page)") {
            let verify_token =
                read_token(&ms_config.verify_token_env, "Messenger (verify)").unwrap_or_default();
            let adapter = Arc::new(
                MessengerAdapter::new(page_token, verify_token, ms_config.webhook_port)
                    .with_account_id(ms_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                ms_config.default_agent.clone(),
                ms_config.account_id.clone(),
            ));
        }
    }

    // Reddit
    #[cfg(feature = "channel-reddit")]
    for rd_config in config.reddit.iter() {
        if let Some(secret) = read_token(&rd_config.client_secret_env, "Reddit (secret)") {
            if let Some(password) = read_token(&rd_config.password_env, "Reddit (password)") {
                let adapter = Arc::new(
                    RedditAdapter::new(
                        rd_config.client_id.clone(),
                        secret,
                        rd_config.username.clone(),
                        password,
                        rd_config.subreddits.clone(),
                    )
                    .with_account_id(rd_config.account_id.clone()),
                );
                adapters.push((
                    adapter,
                    rd_config.default_agent.clone(),
                    rd_config.account_id.clone(),
                ));
            }
        }
    }

    // Mastodon
    #[cfg(feature = "channel-mastodon")]
    for md_config in config.mastodon.iter() {
        if let Some(token) = read_token(&md_config.access_token_env, "Mastodon") {
            let adapter = Arc::new(
                MastodonAdapter::new(md_config.instance_url.clone(), token)
                    .with_account_id(md_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                md_config.default_agent.clone(),
                md_config.account_id.clone(),
            ));
        }
    }

    // Bluesky
    #[cfg(feature = "channel-bluesky")]
    for bs_config in config.bluesky.iter() {
        if let Some(password) = read_token(&bs_config.app_password_env, "Bluesky") {
            let adapter = Arc::new(
                BlueskyAdapter::new(bs_config.identifier.clone(), password)
                    .with_account_id(bs_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                bs_config.default_agent.clone(),
                bs_config.account_id.clone(),
            ));
        }
    }

    // Feishu/Lark (unified adapter)
    #[cfg(feature = "channel-feishu")]
    for fs_config in config.feishu.iter() {
        let region = match fs_config.region.as_str() {
            "intl" | "lark" => FeishuRegion::Intl,
            _ => FeishuRegion::Cn,
        };
        let receive_mode = match fs_config.receive_mode.as_str() {
            "webhook" => FeishuReceiveMode::Webhook,
            _ => FeishuReceiveMode::Websocket,
        };
        let label = region.label();
        if let Some(secret) = read_token(&fs_config.app_secret_env, label) {
            let adapter = Arc::new(
                FeishuAdapter::new(
                    fs_config.app_id.clone(),
                    secret,
                    fs_config.webhook_port,
                    region,
                    receive_mode,
                )
                .with_account_id(fs_config.account_id.clone())
                .with_verification(
                    fs_config.verification_token.clone(),
                    fs_config.encrypt_key.clone(),
                ),
            );
            adapters.push((
                adapter,
                fs_config.default_agent.clone(),
                fs_config.account_id.clone(),
            ));
        }
    }

    // Revolt
    #[cfg(feature = "channel-revolt")]
    for rv_config in config.revolt.iter() {
        if let Some(token) = read_token(&rv_config.bot_token_env, "Revolt") {
            let adapter =
                Arc::new(RevoltAdapter::new(token).with_account_id(rv_config.account_id.clone()));
            adapters.push((
                adapter,
                rv_config.default_agent.clone(),
                rv_config.account_id.clone(),
            ));
        }
    }

    // WeCom/WeChat Work
    #[cfg(feature = "channel-wecom")]
    for wc_config in config.wecom.iter() {
        if let Some(secret) = read_token(&wc_config.secret_env, "WeCom") {
            let adapter = Arc::new(
                WeComAdapter::with_verification(
                    wc_config.corp_id.clone(),
                    wc_config.agent_id.clone(),
                    secret,
                    wc_config.webhook_port,
                    wc_config
                        .encoding_aes_key_env
                        .as_ref()
                        .and_then(|e| std::env::var(e).ok()),
                    wc_config
                        .token_env
                        .as_ref()
                        .and_then(|e| std::env::var(e).ok()),
                )
                .with_account_id(wc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                wc_config.default_agent.clone(),
                wc_config.account_id.clone(),
            ));
        }
    }

    // ── Wave 4 ──────────────────────────────────────────────────

    // Nextcloud Talk
    #[cfg(feature = "channel-nextcloud")]
    for nc_config in config.nextcloud.iter() {
        if let Some(token) = read_token(&nc_config.token_env, "Nextcloud") {
            let adapter = Arc::new(
                NextcloudAdapter::new(
                    nc_config.server_url.clone(),
                    token,
                    nc_config.allowed_rooms.clone(),
                )
                .with_account_id(nc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                nc_config.default_agent.clone(),
                nc_config.account_id.clone(),
            ));
        }
    }

    // Guilded
    #[cfg(feature = "channel-guilded")]
    for gd_config in config.guilded.iter() {
        if let Some(token) = read_token(&gd_config.bot_token_env, "Guilded") {
            let adapter = Arc::new(
                GuildedAdapter::new(token, gd_config.server_ids.clone())
                    .with_account_id(gd_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                gd_config.default_agent.clone(),
                gd_config.account_id.clone(),
            ));
        }
    }

    // Keybase
    #[cfg(feature = "channel-keybase")]
    for kb_config in config.keybase.iter() {
        if let Some(paperkey) = read_token(&kb_config.paperkey_env, "Keybase") {
            let adapter = Arc::new(
                KeybaseAdapter::new(
                    kb_config.username.clone(),
                    paperkey,
                    kb_config.allowed_teams.clone(),
                )
                .with_account_id(kb_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                kb_config.default_agent.clone(),
                kb_config.account_id.clone(),
            ));
        }
    }

    // Threema
    #[cfg(feature = "channel-threema")]
    for tm_config in config.threema.iter() {
        if let Some(secret) = read_token(&tm_config.secret_env, "Threema") {
            let adapter = Arc::new(
                ThreemaAdapter::new(tm_config.threema_id.clone(), secret, tm_config.webhook_port)
                    .with_account_id(tm_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                tm_config.default_agent.clone(),
                tm_config.account_id.clone(),
            ));
        }
    }

    // Nostr
    #[cfg(feature = "channel-nostr")]
    for ns_config in config.nostr.iter() {
        if let Some(key) = read_token(&ns_config.private_key_env, "Nostr") {
            let adapter = Arc::new(
                NostrAdapter::new(key, ns_config.relays.clone())
                    .with_account_id(ns_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                ns_config.default_agent.clone(),
                ns_config.account_id.clone(),
            ));
        }
    }

    // Webex
    #[cfg(feature = "channel-webex")]
    for wx_config in config.webex.iter() {
        if let Some(token) = read_token(&wx_config.bot_token_env, "Webex") {
            let adapter = Arc::new(
                WebexAdapter::new(token, wx_config.allowed_rooms.clone())
                    .with_account_id(wx_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                wx_config.default_agent.clone(),
                wx_config.account_id.clone(),
            ));
        }
    }

    // Pumble
    #[cfg(feature = "channel-pumble")]
    for pb_config in config.pumble.iter() {
        if let Some(token) = read_token(&pb_config.bot_token_env, "Pumble") {
            let adapter = Arc::new(
                PumbleAdapter::new(token, pb_config.webhook_port)
                    .with_account_id(pb_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                pb_config.default_agent.clone(),
                pb_config.account_id.clone(),
            ));
        }
    }

    // Flock
    #[cfg(feature = "channel-flock")]
    for fl_config in config.flock.iter() {
        if let Some(token) = read_token(&fl_config.bot_token_env, "Flock") {
            let adapter = Arc::new(
                FlockAdapter::new(token, fl_config.webhook_port)
                    .with_account_id(fl_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                fl_config.default_agent.clone(),
                fl_config.account_id.clone(),
            ));
        }
    }

    // Twist
    #[cfg(feature = "channel-twist")]
    for tw_config in config.twist.iter() {
        if let Some(token) = read_token(&tw_config.token_env, "Twist") {
            let adapter = Arc::new(
                TwistAdapter::new(
                    token,
                    tw_config.workspace_id.clone(),
                    tw_config.allowed_channels.clone(),
                )
                .with_account_id(tw_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                tw_config.default_agent.clone(),
                tw_config.account_id.clone(),
            ));
        }
    }

    // ── Wave 5 ──────────────────────────────────────────────────

    // Mumble
    #[cfg(feature = "channel-mumble")]
    for mb_config in config.mumble.iter() {
        if let Some(password) = read_token(&mb_config.password_env, "Mumble") {
            let adapter = Arc::new(
                MumbleAdapter::new(
                    mb_config.host.clone(),
                    mb_config.port,
                    password,
                    mb_config.username.clone(),
                    mb_config.channel.clone(),
                )
                .with_account_id(mb_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                mb_config.default_agent.clone(),
                mb_config.account_id.clone(),
            ));
        }
    }

    // DingTalk
    #[cfg(feature = "channel-dingtalk")]
    for dt_config in config.dingtalk.iter() {
        if let Some(token) = read_token(&dt_config.access_token_env, "DingTalk") {
            let secret = read_token(&dt_config.secret_env, "DingTalk (secret)").unwrap_or_default();
            let adapter = Arc::new(
                DingTalkAdapter::new(token, secret, dt_config.webhook_port)
                    .with_account_id(dt_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                dt_config.default_agent.clone(),
                dt_config.account_id.clone(),
            ));
        }
    }

    // QQ
    #[cfg(feature = "channel-qq")]
    for qq_config in config.qq.iter() {
        if let Some(secret) = read_token(&qq_config.app_secret_env, "QQ") {
            let adapter = Arc::new(
                QqAdapter::new(
                    qq_config.app_id.clone(),
                    secret,
                    qq_config.allowed_users.clone(),
                )
                .with_account_id(qq_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                qq_config.default_agent.clone(),
                qq_config.account_id.clone(),
            ));
        }
    }

    // Discourse
    #[cfg(feature = "channel-discourse")]
    for dc_config in config.discourse.iter() {
        if let Some(api_key) = read_token(&dc_config.api_key_env, "Discourse") {
            let adapter = Arc::new(
                DiscourseAdapter::new(
                    dc_config.base_url.clone(),
                    api_key,
                    dc_config.api_username.clone(),
                    dc_config.categories.clone(),
                )
                .with_account_id(dc_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                dc_config.default_agent.clone(),
                dc_config.account_id.clone(),
            ));
        }
    }

    // Gitter
    #[cfg(feature = "channel-gitter")]
    for gt_config in config.gitter.iter() {
        if let Some(token) = read_token(&gt_config.token_env, "Gitter") {
            let adapter = Arc::new(
                GitterAdapter::new(token, gt_config.room_id.clone())
                    .with_account_id(gt_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                gt_config.default_agent.clone(),
                gt_config.account_id.clone(),
            ));
        }
    }

    // ntfy
    #[cfg(feature = "channel-ntfy")]
    for nf_config in config.ntfy.iter() {
        let token = if nf_config.token_env.is_empty() {
            String::new()
        } else {
            read_token(&nf_config.token_env, "ntfy").unwrap_or_default()
        };
        let adapter = Arc::new(
            NtfyAdapter::new(nf_config.server_url.clone(), nf_config.topic.clone(), token)
                .with_account_id(nf_config.account_id.clone()),
        );
        adapters.push((
            adapter,
            nf_config.default_agent.clone(),
            nf_config.account_id.clone(),
        ));
    }

    // Gotify
    #[cfg(feature = "channel-gotify")]
    for gf_config in config.gotify.iter() {
        if let Some(app_token) = read_token(&gf_config.app_token_env, "Gotify (app)") {
            let client_token =
                read_token(&gf_config.client_token_env, "Gotify (client)").unwrap_or_default();
            let adapter = Arc::new(
                GotifyAdapter::new(gf_config.server_url.clone(), app_token, client_token)
                    .with_account_id(gf_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                gf_config.default_agent.clone(),
                gf_config.account_id.clone(),
            ));
        }
    }

    // Webhook
    #[cfg(feature = "channel-webhook")]
    for wh_config in config.webhook.iter() {
        if let Some(secret) = read_token(&wh_config.secret_env, "Webhook") {
            let adapter = Arc::new(
                WebhookAdapter::new(
                    secret,
                    wh_config.listen_port,
                    wh_config.callback_url.clone(),
                )
                .with_account_id(wh_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                wh_config.default_agent.clone(),
                wh_config.account_id.clone(),
            ));
        }
    }

    // LinkedIn
    #[cfg(feature = "channel-linkedin")]
    for li_config in config.linkedin.iter() {
        if let Some(token) = read_token(&li_config.access_token_env, "LinkedIn") {
            let adapter = Arc::new(
                LinkedInAdapter::new(token, li_config.organization_id.clone())
                    .with_account_id(li_config.account_id.clone()),
            );
            adapters.push((
                adapter,
                li_config.default_agent.clone(),
                li_config.account_id.clone(),
            ));
        }
    }

    // ── Sidecar channel adapters ───────────────────────────────
    for sidecar_config in &kernel.config.sidecar_channels {
        info!(
            name = %sidecar_config.name,
            command = %sidecar_config.command,
            "Registering sidecar channel adapter"
        );
        let adapter = Arc::new(SidecarAdapter::new(sidecar_config));
        adapters.push((adapter, None, None));
    }

    if adapters.is_empty() {
        return (None, Vec::new());
    }

    // Resolve per-channel default agents AND set the first one as system-wide fallback
    let mut router = AgentRouter::new();
    let mut system_default_set = false;
    for (adapter, default_agent, account_id) in &adapters {
        if let Some(ref name) = default_agent {
            // Resolve agent name to ID
            let agent_id = match handle.find_agent_by_name(name).await {
                Ok(Some(id)) => Some(id),
                _ => match handle.spawn_agent_by_name(name).await {
                    Ok(id) => Some(id),
                    Err(e) => {
                        warn!(
                            "{}: could not find or spawn default agent '{}': {e}",
                            adapter.name(),
                            name
                        );
                        None
                    }
                },
            };
            if let Some(agent_id) = agent_id {
                // Use account_id-qualified channel key for multi-bot routing
                let channel_key = match account_id {
                    Some(aid) => format!("{:?}:{}", adapter.channel_type(), aid),
                    None => format!("{:?}", adapter.channel_type()),
                };
                info!(
                    "{} default agent: {name} ({agent_id}) [channel: {channel_key}]",
                    adapter.name()
                );
                router.set_channel_default_with_name(channel_key, agent_id, name.clone());
                // First configured default also becomes system-wide fallback
                if !system_default_set {
                    router.set_default(agent_id);
                    system_default_set = true;
                }
            }
        }
    }

    // Load bindings and broadcast config from kernel
    let bindings = kernel.list_bindings();
    if !bindings.is_empty() {
        // Register all known agents in the router's name cache for binding resolution
        for entry in kernel.registry.list() {
            router.register_agent(entry.name.clone(), entry.id);
        }
        router.load_bindings(&bindings);
        info!(count = bindings.len(), "Loaded agent bindings into router");
    }
    router.load_broadcast(kernel.broadcast.clone());

    let bridge_handle: Arc<dyn ChannelBridgeHandle> = Arc::new(KernelBridgeAdapter {
        kernel: kernel.clone(),
        started_at: Instant::now(),
    });
    let router = Arc::new(router);
    let mut manager = BridgeManager::new(bridge_handle, router);

    let mut started_names = Vec::new();
    for (adapter, _, _account_id) in adapters {
        let name = adapter.name().to_string();
        // Register adapter in kernel so agents can use `channel_send` tool
        kernel
            .channel_adapters
            .insert(name.clone(), adapter.clone());
        match manager.start_adapter(adapter).await {
            Ok(()) => {
                info!("{name} channel bridge started");
                started_names.push(name);
            }
            Err(e) => {
                // Remove from kernel map if start failed
                kernel.channel_adapters.remove(&name);
                error!("Failed to start {name} bridge: {e}");
            }
        }
    }

    if started_names.is_empty() {
        (None, Vec::new())
    } else {
        (Some(manager), started_names)
    }
}

/// Reload channels from disk config — stops old bridge, starts new one.
///
/// Reads `config.toml` fresh, rebuilds the channel bridge, and stores it
/// in `AppState.bridge_manager`. Returns the list of started channel names.
pub async fn reload_channels_from_disk(
    state: &crate::routes::AppState,
) -> Result<Vec<String>, String> {
    // Stop existing bridge
    {
        let mut guard = state.bridge_manager.lock().await;
        if let Some(ref mut bridge) = *guard {
            bridge.stop().await;
        }
        *guard = None;
    }

    // Re-read secrets.env so new API tokens are available in std::env
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if secrets_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&secrets_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some(eq_pos) = trimmed.find('=') {
                    let key = trimmed[..eq_pos].trim();
                    let mut value = trimmed[eq_pos + 1..].trim().to_string();
                    if !key.is_empty() {
                        // Strip matching quotes
                        if ((value.starts_with('"') && value.ends_with('"'))
                            || (value.starts_with('\'') && value.ends_with('\'')))
                            && value.len() >= 2
                        {
                            value = value[1..value.len() - 1].to_string();
                        }
                        // Always overwrite — the file is the source of truth after dashboard edits
                        std::env::set_var(key, &value);
                    }
                }
            }
            info!("Reloaded secrets.env for channel hot-reload");
        }
    }

    // Re-read config from disk
    let config_path = state.kernel.config.home_dir.join("config.toml");
    let fresh_config = librefang_kernel::config::load_config(Some(&config_path));

    // Update the live channels config so list_channels() reflects reality
    *state.channels_config.write().await = fresh_config.channels.clone();

    // Start new bridge with fresh channel config
    let (new_bridge, started) =
        start_channel_bridge_with_config(state.kernel.clone(), &fresh_config.channels).await;

    // Store the new bridge
    *state.bridge_manager.lock().await = new_bridge;

    info!(
        started = started.len(),
        channels = ?started,
        "Channel hot-reload complete"
    );

    Ok(started)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_tool_call_detects_markdown_tool_call_with_preamble() {
        let text = "Here is the tool call:\n```json\nweb_search {\"query\":\"rust\"}\n```";
        assert!(looks_like_tool_call(text));
    }

    #[test]
    fn test_looks_like_tool_call_detects_backtick_tool_call_with_preamble() {
        let text = "I'll use `web_search {\"query\":\"rust\"}` for that.";
        assert!(looks_like_tool_call(text));
    }

    #[test]
    fn test_looks_like_tool_call_detects_bare_json_tool_call_with_preamble() {
        let text =
            "I'll run that: {\"name\":\"shell_exec\",\"arguments\":{\"command\":\"ls -la\"}}";
        assert!(looks_like_tool_call(text));
    }

    #[test]
    fn test_looks_like_tool_call_allows_normal_code_block() {
        let text = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        assert!(!looks_like_tool_call(text));
    }

    #[test]
    fn test_looks_like_tool_call_allows_inline_json_example() {
        let text = "Use `{\"foo\":\"bar\"}` in your config.";
        assert!(!looks_like_tool_call(text));
    }

    #[test]
    fn test_looks_like_tool_call_allows_non_tool_json_object() {
        let text = "Profile payload: {\"name\":\"Alice\",\"role\":\"admin\"}";
        assert!(!looks_like_tool_call(text));
    }

    #[tokio::test]
    async fn test_bridge_skips_when_no_config() {
        let config = librefang_types::config::KernelConfig::default();
        assert!(config.channels.telegram.is_none());
        assert!(config.channels.discord.is_none());
        assert!(config.channels.slack.is_none());
        assert!(config.channels.whatsapp.is_none());
        assert!(config.channels.signal.is_none());
        assert!(config.channels.matrix.is_none());
        assert!(config.channels.email.is_none());
        assert!(config.channels.teams.is_none());
        assert!(config.channels.mattermost.is_none());
        assert!(config.channels.irc.is_none());
        assert!(config.channels.google_chat.is_none());
        assert!(config.channels.twitch.is_none());
        assert!(config.channels.rocketchat.is_none());
        assert!(config.channels.zulip.is_none());
        assert!(config.channels.xmpp.is_none());
        // Wave 3
        assert!(config.channels.line.is_none());
        assert!(config.channels.viber.is_none());
        assert!(config.channels.messenger.is_none());
        assert!(config.channels.reddit.is_none());
        assert!(config.channels.mastodon.is_none());
        assert!(config.channels.bluesky.is_none());
        assert!(config.channels.feishu.is_none());
        assert!(config.channels.revolt.is_none());
        // Wave 4
        assert!(config.channels.nextcloud.is_none());
        assert!(config.channels.guilded.is_none());
        assert!(config.channels.keybase.is_none());
        assert!(config.channels.threema.is_none());
        assert!(config.channels.nostr.is_none());
        assert!(config.channels.webex.is_none());
        assert!(config.channels.pumble.is_none());
        assert!(config.channels.flock.is_none());
        assert!(config.channels.twist.is_none());
        // Wave 5
        assert!(config.channels.mumble.is_none());
        assert!(config.channels.dingtalk.is_none());
        assert!(config.channels.discourse.is_none());
        assert!(config.channels.gitter.is_none());
        assert!(config.channels.ntfy.is_none());
        assert!(config.channels.gotify.is_none());
        assert!(config.channels.webhook.is_none());
        assert!(config.channels.linkedin.is_none());
    }
}
