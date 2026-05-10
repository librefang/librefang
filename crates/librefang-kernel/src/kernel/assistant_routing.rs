//! Cluster pulled out of mod.rs in #4713 phase 3c.
//!
//! Hosts the assistant-routing helpers: intent classification, hand
//! activation, route caching, and the "send streaming with resolution"
//! path used by the streaming dispatch entry points in
//! [`super::messaging`]. Methods here decide whether an inbound message
//! should be answered by the requested agent, a specialist sibling, or
//! an active Hand instance.
//!
//! Sibling submodule of `kernel::mod`. Several methods are bumped to
//! `pub(crate)` because they are called from `super::messaging` (a
//! sibling that cannot see this module's private items) or from
//! `kernel::tests` — see #4713 phase 3c notes.

use librefang_channels::types::SenderContext;
use librefang_types::agent::AgentId;

use crate::error::KernelResult;

use super::*;

impl LibreFangKernel {
    pub(crate) fn notify_owner_bg(&self, message: String) {
        let weak = match self.self_handle.get() {
            Some(w) => w.clone(),
            None => return,
        };
        // Note: this is kernel-scoped (not agent-scoped) — sending owner
        // notifications via channel adapters touches `kernel.send_channel_message`
        // which has its own lifecycle. No per-agent tracking needed here.
        spawn_logged("owner_notify", async move {
            let kernel = match weak.upgrade() {
                Some(k) => k,
                None => return,
            };
            let cfg = kernel.config.load();
            let bindings = match cfg.users.iter().find(|u| u.role == "owner") {
                Some(u) => u.channel_bindings.clone(),
                None => return,
            };
            drop(cfg);
            for (channel, platform_id) in &bindings {
                if kernel.mesh.channel_adapters.contains_key(channel.as_str()) {
                    if let Err(e) = kernel
                        .send_channel_message(channel, platform_id, &message, None, None)
                        .await
                    {
                        warn!(channel = %channel, error = %e, "Failed to send owner notification");
                    }
                }
            }
        });
    }

    /// LLM-based intent classification for routing.
    ///
    /// Given a user message, uses a lightweight LLM call to determine which
    /// specialist agent should handle it. Returns the agent name (e.g. "coder",
    /// "researcher") or "assistant" for general queries.
    async fn llm_classify_intent(&self, message: &str) -> Option<String> {
        use librefang_runtime::llm_driver::CompletionRequest;
        use librefang_types::message::Message;

        // Skip classification for very short/simple messages — likely greetings
        if Self::should_skip_intent_classification(message) {
            return None;
        }

        let dynamic_choices = router::all_template_descriptions(
            &self.home_dir_boot.join("workspaces").join("agents"),
        );
        let routable_names: HashSet<String> = dynamic_choices
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        let route_choices = dynamic_choices
            .iter()
            .map(|(name, desc)| {
                let prefix = format!("{name}: ");
                let prompt_desc = desc.strip_prefix(&prefix).unwrap_or(desc);
                format!("- {name}: {prompt_desc}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let classify_prompt = format!(
            "You are an intent classifier. Given a user message, reply with ONLY the agent name that should handle it. Choose from:\n- assistant: greetings, simple questions, casual chat, general knowledge\n{}\n\nReply with ONLY the agent name, nothing else.",
            route_choices
        );

        let request = CompletionRequest {
            model: String::new(), // use driver default
            messages: std::sync::Arc::new(vec![Message::user(message.to_string())]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 20,
            temperature: 0.0,
            system: Some(classify_prompt),
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        };

        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.llm.default_driver.complete(request),
        )
        .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                debug!(error = %e, "LLM classify failed — falling back to assistant");
                return None;
            }
            Err(_) => {
                debug!("LLM classify timed out (5s) — falling back to assistant");
                return None;
            }
        };

        let agent_name = result.text().trim().to_lowercase();
        if agent_name != "assistant" && routable_names.contains(agent_name.as_str()) {
            info!(
                target_agent = %agent_name,
                "LLM intent classification: routing to specialist"
            );
            Some(agent_name)
        } else {
            None // assistant handles it
        }
    }

    /// Resolve a specialist agent by name — find existing or spawn from template.
    fn resolve_or_spawn_specialist(&self, name: &str) -> KernelResult<AgentId> {
        if let Some(entry) = self.agents.registry.find_by_name(name) {
            return Ok(entry.id);
        }
        let manifest = router::load_template_manifest(&self.home_dir_boot, name)
            .map_err(|e| KernelError::LibreFang(LibreFangError::Internal(e)))?;
        let id = self.spawn_agent(manifest)?;
        info!(agent = %name, id = %id, "Spawned specialist agent for LLM routing");
        Ok(id)
    }

    pub(crate) async fn send_message_streaming_resolved(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
        sender_context: Option<&SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let effective_id = self
            .resolve_assistant_target(agent_id, message, sender_context)
            .await?;
        self.send_message_streaming_with_sender_and_session(
            effective_id,
            message,
            kernel_handle,
            sender_context,
            thinking_override,
            session_id_override,
        )
    }

    pub(crate) async fn resolve_assistant_target(
        &self,
        agent_id: AgentId,
        message: &str,
        sender_context: Option<&SenderContext>,
    ) -> KernelResult<AgentId> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;
        if entry.name != "assistant" {
            return Ok(agent_id);
        }
        drop(entry);

        // Per-channel auto-routing strategy gate.
        //
        // When `auto_route` is `Off` (the default for all channels), channel messages
        // bypass classification entirely — preserving legacy behaviour.
        // Other strategies allow opt-in routing with different cache semantics.
        if let Some(ctx) = sender_context {
            let cache_key = format!(
                "{}:{}:{}:{}",
                agent_id,
                ctx.channel,
                ctx.account_id.as_deref().unwrap_or(""),
                ctx.user_id,
            );
            let ttl = std::time::Duration::from_secs(ctx.auto_route_ttl_minutes as u64 * 60);

            match ctx.auto_route {
                AutoRouteStrategy::Off => return Ok(agent_id),

                AutoRouteStrategy::ExplicitOnly => {
                    if let Some(entry) = self.events.assistant_routes.get(&cache_key) {
                        let target = entry.value().0.clone();
                        drop(entry);
                        match self.resolve_assistant_route_target(&target) {
                            Ok(routed_id) => return Ok(routed_id),
                            Err(_) => {
                                self.events.assistant_routes.remove(&cache_key);
                            }
                        }
                    }
                    // No cached entry — fall through to LLM classification once,
                    // then store the result.
                }

                AutoRouteStrategy::StickyTtl => {
                    if let Some(entry) = self.events.assistant_routes.get(&cache_key) {
                        if entry.value().1.elapsed() < ttl {
                            let target = entry.value().0.clone();
                            drop(entry);
                            match self.resolve_assistant_route_target(&target) {
                                Ok(routed_id) => return Ok(routed_id),
                                Err(_) => {
                                    self.events.assistant_routes.remove(&cache_key);
                                }
                            }
                        }
                    }
                    // Cache miss or TTL expired — fall through to re-classify.
                }

                AutoRouteStrategy::StickyHeuristic => {
                    let heuristic_target = self.route_assistant_by_metadata(message);
                    if let Some(h_target) = heuristic_target {
                        if let Some(entry) = self.events.assistant_routes.get(&cache_key) {
                            let cached = entry.value().0.clone();
                            drop(entry);

                            if h_target == cached {
                                // Heuristic agrees with cache — reset divergence counter.
                                self.events.route_divergence.remove(&cache_key);
                                match self.resolve_assistant_route_target(&cached) {
                                    Ok(routed_id) => return Ok(routed_id),
                                    Err(_) => {
                                        self.events.assistant_routes.remove(&cache_key);
                                    }
                                }
                            } else {
                                // Disagreement — increment divergence counter.
                                let count = {
                                    let mut div_entry = self
                                        .events
                                        .route_divergence
                                        .entry(cache_key.clone())
                                        .or_insert(0);
                                    *div_entry += 1;
                                    *div_entry
                                };
                                if count < ctx.auto_route_divergence_count {
                                    // Not enough divergence yet — stay on cached route.
                                    if let Some(entry) =
                                        self.events.assistant_routes.get(&cache_key)
                                    {
                                        let target = entry.value().0.clone();
                                        drop(entry);
                                        match self.resolve_assistant_route_target(&target) {
                                            Ok(routed_id) => return Ok(routed_id),
                                            Err(_) => {
                                                self.events.assistant_routes.remove(&cache_key);
                                            }
                                        }
                                    }
                                }
                                // Enough divergence — fall through to LLM re-classification.
                                self.events.route_divergence.remove(&cache_key);
                            }
                        }
                        // No cached entry — fall through to LLM classification.
                    } else {
                        // Heuristic returned nothing — reuse cache within TTL if available.
                        if let Some(entry) = self.events.assistant_routes.get(&cache_key) {
                            if entry.value().1.elapsed() < ttl {
                                let target = entry.value().0.clone();
                                drop(entry);
                                match self.resolve_assistant_route_target(&target) {
                                    Ok(routed_id) => return Ok(routed_id),
                                    Err(_) => {
                                        self.events.assistant_routes.remove(&cache_key);
                                    }
                                }
                            }
                        }
                        // Cache miss or expired — fall through to LLM classification.
                    }
                }
            }
        }

        let route_key = Self::assistant_route_key(agent_id, sender_context);

        if Self::should_reuse_cached_route(message) {
            if let Some(target) = self
                .events
                .assistant_routes
                .get(&route_key)
                .map(|entry| entry.value().0.clone())
            {
                match self.resolve_assistant_route_target(&target) {
                    Ok(routed_id) => {
                        // Update last-used timestamp for GC
                        self.events.assistant_routes.insert(
                            route_key.clone(),
                            (target.clone(), std::time::Instant::now()),
                        );
                        info!(
                            route_type = target.route_type(),
                            target = %target.name(),
                            "Assistant reusing cached route for follow-up"
                        );
                        return Ok(routed_id);
                    }
                    Err(e) => {
                        warn!(
                            route_type = target.route_type(),
                            target = %target.name(),
                            error = %e,
                            "Cached assistant route failed — clearing"
                        );
                        self.events.assistant_routes.remove(&route_key);
                    }
                }
            }
        }

        if let Some(specialist) = self.llm_classify_intent(message).await {
            let routed_id = self.resolve_or_spawn_specialist(&specialist)?;
            self.events.assistant_routes.insert(
                route_key,
                (
                    AssistantRouteTarget::Specialist(specialist.clone()),
                    std::time::Instant::now(),
                ),
            );
            return Ok(routed_id);
        }

        if let Some(target) = self.route_assistant_by_metadata(message) {
            let routed_id = self.resolve_assistant_route_target(&target)?;
            info!(
                route_type = target.route_type(),
                target = %target.name(),
                "Assistant routed via metadata fallback"
            );
            self.events
                .assistant_routes
                .insert(route_key, (target, std::time::Instant::now()));
            return Ok(routed_id);
        }

        self.events.assistant_routes.remove(&route_key);
        Ok(agent_id)
    }

    fn route_assistant_by_metadata(&self, message: &str) -> Option<AssistantRouteTarget> {
        let hand_selection = router::auto_select_hand(message, None);
        let template_selection = router::auto_select_template(
            message,
            &self.home_dir_boot.join("workspaces").join("agents"),
            None,
        );

        let hand_candidate = hand_selection
            .hand_id
            .filter(|hand_id| hand_selection.score > 0 && self.hand_requirements_met(hand_id));

        if let Some(hand_id) = hand_candidate {
            if hand_selection.score >= template_selection.score {
                return Some(AssistantRouteTarget::Hand(hand_id));
            }
        }

        if template_selection.score > 0 && template_selection.template != "assistant" {
            return Some(AssistantRouteTarget::Specialist(
                template_selection.template,
            ));
        }

        None
    }

    fn resolve_assistant_route_target(
        &self,
        target: &AssistantRouteTarget,
    ) -> KernelResult<AgentId> {
        match target {
            AssistantRouteTarget::Specialist(name) => self.resolve_or_spawn_specialist(name),
            AssistantRouteTarget::Hand(hand_id) => self.resolve_or_activate_hand(hand_id),
        }
    }

    fn resolve_or_activate_hand(&self, hand_id: &str) -> KernelResult<AgentId> {
        if let Some(agent_id) = self.active_hand_agent_id(hand_id) {
            return Ok(agent_id);
        }

        let instance = self.activate_hand(hand_id, std::collections::HashMap::new())?;
        instance.agent_id().ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Hand '{hand_id}' activated without an agent id"
            )))
        })
    }

    fn active_hand_agent_id(&self, hand_id: &str) -> Option<AgentId> {
        self.skills
            .hand_registry
            .list_instances()
            .into_iter()
            .find(|instance| {
                instance.hand_id == hand_id
                    && instance.status == librefang_hands::HandStatus::Active
            })
            .and_then(|instance| instance.agent_id())
    }

    fn hand_requirements_met(&self, hand_id: &str) -> bool {
        match self.skills.hand_registry.check_requirements(hand_id) {
            Ok(results) => {
                for (req, satisfied) in &results {
                    if !satisfied {
                        info!(
                            hand = %hand_id,
                            requirement = %req.label,
                            "Hand requirement not met, skipping assistant auto-route"
                        );
                        return false;
                    }
                }
                true
            }
            Err(_) => true,
        }
    }

    pub(crate) fn assistant_route_key(
        agent_id: AgentId,
        sender_context: Option<&SenderContext>,
    ) -> String {
        match sender_context {
            Some(sender) => format!(
                "{agent_id}:{}:{}:{}:{}",
                sender.channel,
                sender.account_id.as_deref().unwrap_or_default(),
                sender.user_id,
                sender.thread_id.as_deref().unwrap_or_default()
            ),
            None => agent_id.to_string(),
        }
    }

    pub(crate) fn should_skip_intent_classification(message: &str) -> bool {
        let trimmed = message.trim();
        trimmed.len() < 15 && !trimmed.contains("http")
    }
}
