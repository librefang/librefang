//! `impl ContextEngine for ScriptableContextEngine` — the eight trait
//! methods that fan out into per-hook plugin script invocations.

use super::*;

/// Host-side execution of a `compact` hook's `request_llm_summary` directive
/// (#6264). When a `compact` script returns
/// `{ "request_llm_summary": { prompt?, summarize: [..], keep: [..], max_tokens? } }`
/// instead of final `messages`, the host runs the LLM summary itself: the
/// script chooses WHAT to fold and HOW (prompt, budget), but the driver,
/// credentials, and routing stay host-side. Returns `None` — so the caller
/// falls through to the `messages` path / `inner.compact` fallback, never a
/// panic — when there is no such directive, nothing to summarize, or the LLM
/// call fails / returns empty.
async fn try_host_summary(
    output: &serde_json::Value,
    driver: &std::sync::Arc<dyn LlmDriver>,
    model: &str,
) -> Option<CompactionResult> {
    use librefang_types::message::{ContentBlock, MessageContent, Role};

    let req = output.get("request_llm_summary")?.as_object()?;

    let parse_msgs = |key: &str| -> Vec<Message> {
        req.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    let summarize = parse_msgs("summarize");
    if summarize.is_empty() {
        return None; // nothing to fold — let the caller fall back
    }
    let keep = parse_msgs("keep");
    let max_tokens = req
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(2048) as u32;
    let instruction = req
        .get("prompt")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(
            "Summarize the following conversation preserving key facts, decisions, \
             user preferences, and important context. Be concise but thorough. \
             Output only the summary, no preamble.",
        );

    // Render the messages to plain role-labelled text so the summarization
    // request never replays raw tool_use/tool_result blocks (which would risk
    // provider role-ordering errors) — same approach as the built-in
    // compactor's `summarize_messages`.
    let convo = summarize
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            format!("{role}: {}", m.content.text_content())
        })
        .collect::<Vec<_>>()
        .join("\n");

    let request = crate::llm_driver::CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: format!("{instruction}\n\n---\n{convo}\n---"),
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }]),
        max_tokens,
        temperature: 0.3,
        system: Some(
            "You are a conversation summarizer. Produce a concise summary that captures \
             all key facts, decisions, and context from the conversation."
                .to_string(),
        ),
        ..Default::default()
    };

    match driver.complete(request).await {
        Ok(resp) => {
            let text = resp.text();
            if text.is_empty() {
                warn!("request_llm_summary: host LLM returned an empty summary; falling back");
                None
            } else {
                let compacted_count = summarize.len();
                Some(CompactionResult {
                    summary: text,
                    kept_messages: keep,
                    compacted_count,
                    chunks_used: 1,
                    used_fallback: false,
                })
            }
        }
        Err(e) => {
            warn!(error = %e, "request_llm_summary: host LLM call failed; falling back");
            None
        }
    }
}

#[async_trait]
impl ContextEngine for ScriptableContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        // Validate all declared hook scripts at startup: existence + executable bit.
        for (name, opt_path) in [
            ("ingest", &self.ingest_script),
            ("after_turn", &self.after_turn_script),
            ("bootstrap", &self.bootstrap_script),
            ("assemble", &self.assemble_script),
            ("compact", &self.compact_script),
            ("transform_tool_result", &self.transform_tool_result_script),
            ("prepare_subagent", &self.prepare_subagent_script),
            ("merge_subagent", &self.merge_subagent_script),
            ("on_event", &self.on_event_script),
        ] {
            if let Some(ref path) = opt_path {
                let resolved = Self::resolve_script_path(path);
                let p = std::path::Path::new(&resolved);
                if !p.exists() {
                    warn!("{name} hook script not found: {resolved}");
                } else {
                    // On Unix, check executable bit so we surface "chmod +x" issues early
                    // rather than getting a cryptic "permission denied" at runtime.
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = std::fs::metadata(p) {
                            let mode = meta.permissions().mode();
                            if mode & 0o111 == 0 {
                                warn!(
                                    "{name} hook script is not executable (run `chmod +x {resolved}`)"
                                );
                            }
                        }
                    }
                    debug!("{name} hook configured: {resolved}");
                }
            }
        }

        self.inner.bootstrap(config).await?;

        // Run bootstrap script if configured.
        // Bootstrap runs once and may need extra time for external connections,
        // so it gets double the configured hook timeout.
        if let Some(ref script) = self.bootstrap_script {
            let bootstrap_timeout = self.hook_timeout_secs.saturating_mul(2);
            let input = serde_json::json!({
                "type": "bootstrap",
                "context_window_tokens": config.context_window_tokens,
                "stable_prefix_mode": config.stable_prefix_mode,
                "max_recall_results": config.max_recall_results,
            });
            match self
                .call_hook_dispatch("bootstrap", script, input, bootstrap_timeout, None)
                .await
            {
                Ok((ref output, ms)) => {
                    Self::record_hook(&self.metrics, "bootstrap", ms, true);
                    debug!("Bootstrap hook completed (timeout={bootstrap_timeout}s, {ms}ms)");
                    self.apply_bootstrap_overrides(output);
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "bootstrap", 0, false);
                    let _ = self.apply_failure_policy("bootstrap", &e);
                }
            }
        }

        Ok(())
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        // In stable_prefix_mode, skip all recall (including hooks) to keep prompt stable
        if self.inner.config.stable_prefix_mode {
            return Ok(IngestResult {
                recalled_memories: Vec::new(),
            });
        }

        // If no ingest script, delegate entirely to default engine
        let Some(ref script) = self.ingest_script else {
            return self.inner.ingest(agent_id, user_message, peer_id).await;
        };

        // Apply ingest_filter — skip hook when message doesn't match.
        // Bootstrap overrides take precedence over the statically configured filter.
        let effective_ingest_filter: Option<String> = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard
                .ingest_filter
                .clone()
                .or_else(|| self.ingest_filter.clone())
        };
        if let Some(ref filter) = effective_ingest_filter {
            if !user_message.contains(filter.as_str()) {
                debug!(
                    filter = filter.as_str(),
                    "Ingest hook skipped (filter mismatch)"
                );
                return self.inner.ingest(agent_id, user_message, peer_id).await;
            }
        }

        // Apply ingest_regex filter.
        if let Some(ref re) = self.ingest_regex {
            if !re.is_match(user_message) {
                debug!("Ingest hook skipped (ingest_regex mismatch)");
                return self.inner.ingest(agent_id, user_message, peer_id).await;
            }
        }

        // Apply agent_id_filter — skip hook for agents not in the allowlist.
        if !self.agent_passes_filter(&agent_id) {
            debug!("Ingest hook skipped (agent_id not in only_for_agent_ids filter)");
            return self.inner.ingest(agent_id, user_message, peer_id).await;
        }

        // Run default recall first (for embedding-based memories)
        let default_result = self.inner.ingest(agent_id, user_message, peer_id).await?;

        // Run the hook for additional/custom recall
        let input = serde_json::json!({
            "type": "ingest",
            "agent_id": agent_id.0.to_string(),
            "message": user_message,
            "peer_id": peer_id,
        });

        // TTL-based cache: skip subprocess if we have a fresh cached result.
        if let Some(ttl_secs) = self.ingest_cache_ttl_secs {
            let cache_key = {
                let raw = serde_json::to_string(&input).unwrap_or_default();
                crate::plugin_manager::sha256_hex(raw.as_bytes())
            };
            let cached = {
                let guard = self.ingest_cache.lock().unwrap();
                guard.get(&cache_key).and_then(|(val, inserted_at)| {
                    if inserted_at.elapsed().as_secs() < ttl_secs {
                        Some(val.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(cached_output) = cached {
                tracing::info!(hook = "ingest", agent_id = %agent_id, ttl_secs, "Ingest hook succeeded (cache hit)");
                debug!("Ingest hook cache hit (ttl={}s)", ttl_secs);
                let mut memories = default_result.recalled_memories;
                if let Some(hook_memories) =
                    cached_output.get("memories").and_then(|m| m.as_array())
                {
                    for mem in hook_memories {
                        if let Some(content) = mem.get("content").and_then(|c| c.as_str()) {
                            memories.push(MemoryFragment {
                                id: librefang_types::memory::MemoryId::new(),
                                agent_id,
                                content: content.to_string(),
                                embedding: None,
                                metadata: std::collections::HashMap::new(),
                                source: librefang_types::memory::MemorySource::System,
                                confidence: 1.0,
                                created_at: chrono::Utc::now(),
                                accessed_at: chrono::Utc::now(),
                                access_count: 0,
                                scope: "hook_cached".to_string(),
                                image_url: None,
                                image_embedding: None,
                                modality: Default::default(),
                            });
                        }
                    }
                }
                return Ok(IngestResult {
                    recalled_memories: memories,
                });
            }
            // Cache miss — run hook and store result below
            let cache_key_owned = cache_key;
            let cache_arc = self.ingest_cache.clone();
            match self
                .call_hook_dispatch(
                    "ingest",
                    script,
                    input.clone(),
                    self.hook_timeout_secs,
                    Some(&agent_id),
                )
                .await
            {
                Ok((output, ms)) => {
                    Self::record_hook(&self.metrics, "ingest", ms, true);
                    tracing::info!(hook = "ingest", agent_id = %agent_id, elapsed_ms = ms, "Ingest hook succeeded (cache miss)");
                    // Store in cache
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key_owned, (output.clone(), std::time::Instant::now()));
                        // Evict expired entries when cache grows large
                        if guard.len() > 512 {
                            guard.retain(|_, (_, inserted_at)| {
                                inserted_at.elapsed().as_secs() < ttl_secs
                            });
                        }
                    }
                    let mut memories = default_result.recalled_memories;
                    if let Some(hook_memories) = output.get("memories").and_then(|m| m.as_array()) {
                        for mem in hook_memories {
                            if let Some(content) = mem.get("content").and_then(|c| c.as_str()) {
                                memories.push(MemoryFragment {
                                    id: librefang_types::memory::MemoryId::new(),
                                    agent_id,
                                    content: content.to_string(),
                                    embedding: None,
                                    metadata: std::collections::HashMap::new(),
                                    source: librefang_types::memory::MemorySource::System,
                                    confidence: 1.0,
                                    created_at: chrono::Utc::now(),
                                    accessed_at: chrono::Utc::now(),
                                    access_count: 0,
                                    scope: "hook".to_string(),
                                    image_url: None,
                                    image_embedding: None,
                                    modality: Default::default(),
                                });
                            }
                        }
                    }
                    return Ok(IngestResult {
                        recalled_memories: memories,
                    });
                }
                Err(err) => {
                    Self::record_hook(&self.metrics, "ingest", 0, false);
                    self.apply_failure_policy("ingest", &err)?;
                    return Ok(default_result); // reached only for Warn/Skip policy
                }
            }
        }

        match self
            .call_hook_dispatch(
                "ingest",
                script,
                input,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                Self::record_hook(&self.metrics, "ingest", ms, true);
                self.record_per_agent(&agent_id, ms, true);
                tracing::info!(hook = "ingest", agent_id = %agent_id, elapsed_ms = ms, "Ingest hook succeeded (no cache)");
                // Merge hook memories with default memories
                let mut memories = default_result.recalled_memories;
                if let Some(hook_memories) = output.get("memories").and_then(|m| m.as_array()) {
                    for mem in hook_memories {
                        if let Some(content) = mem.get("content").and_then(|c| c.as_str()) {
                            memories.push(MemoryFragment {
                                id: librefang_types::memory::MemoryId::new(),
                                agent_id,
                                content: content.to_string(),
                                embedding: None,
                                metadata: std::collections::HashMap::new(),
                                source: librefang_types::memory::MemorySource::System,
                                confidence: 1.0,
                                created_at: chrono::Utc::now(),
                                accessed_at: chrono::Utc::now(),
                                access_count: 0,
                                scope: "hook".to_string(),
                                image_url: None,
                                image_embedding: None,
                                modality: Default::default(),
                            });
                        }
                    }
                }
                Ok(IngestResult {
                    recalled_memories: memories,
                })
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "ingest", 0, false);
                self.record_per_agent(&agent_id, 0, false);
                self.apply_failure_policy("ingest", &e)?;
                Ok(default_result)
            }
        }
    }

    async fn assemble(
        &self,
        agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        let Some(ref script) = self.assemble_script else {
            return self
                .inner
                .assemble(
                    agent_id,
                    messages,
                    system_prompt,
                    tools,
                    context_window_tokens,
                )
                .await;
        };

        // Serialize full message structure — tool_use/tool_result blocks preserved
        let msg_values: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        let input = serde_json::json!({
            "type": "assemble",
            "agent_id": agent_id.0.to_string(),
            "system_prompt": system_prompt,
            "messages": msg_values,
            "context_window_tokens": context_window_tokens,
        });

        // Apply agent_id_filter for assemble hook.
        if !self.agent_passes_filter(&agent_id) {
            return self
                .inner
                .assemble(
                    agent_id,
                    messages,
                    system_prompt,
                    tools,
                    context_window_tokens,
                )
                .await;
        }

        // TTL-based cache for assemble hook.
        if let Some(ttl_secs) = self.assemble_cache_ttl_secs {
            let cache_key = crate::plugin_manager::sha256_hex(
                serde_json::to_string(&input).unwrap_or_default().as_bytes(),
            );
            let cached = {
                let guard = self.assemble_cache.lock().unwrap();
                guard.get(&cache_key).and_then(|(val, inserted_at)| {
                    if inserted_at.elapsed().as_secs() < ttl_secs {
                        Some(val.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(cached_output) = cached {
                debug!("Assemble hook cache hit (ttl={}s)", ttl_secs);
                if let Some(new_msgs) = cached_output.get("messages").and_then(|v| v.as_array()) {
                    let assembled: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    if !assembled.is_empty() {
                        *messages = assembled;
                        return Ok(AssembleResult {
                            recovery: crate::context_overflow::RecoveryStage::None,
                        });
                    }
                }
                // Cached result had no messages; fall through to default
                return self
                    .inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await;
            }
            // Cache miss — run hook and store result.
            let cache_arc = self.assemble_cache.clone();
            let result = self
                .call_hook_dispatch(
                    "assemble",
                    script,
                    input,
                    self.hook_timeout_secs,
                    Some(&agent_id),
                )
                .await;
            match result {
                Ok((output, ms)) => {
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key, (output.clone(), std::time::Instant::now()));
                        if guard.len() > 256 {
                            guard.retain(|_, (_, inserted_at)| {
                                inserted_at.elapsed().as_secs() < ttl_secs
                            });
                        }
                    }
                    if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                        let assembled: Vec<Message> = new_msgs
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .collect();
                        if !assembled.is_empty() {
                            Self::record_hook(&self.metrics, "assemble", ms, true);
                            *messages = assembled;
                            return Ok(AssembleResult {
                                recovery: crate::context_overflow::RecoveryStage::None,
                            });
                        }
                    }
                    Self::record_hook(&self.metrics, "assemble", ms, false);
                    return self
                        .inner
                        .assemble(
                            agent_id,
                            messages,
                            system_prompt,
                            tools,
                            context_window_tokens,
                        )
                        .await;
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "assemble", 0, false);
                    self.apply_failure_policy("assemble", &e)?;
                    return self
                        .inner
                        .assemble(
                            agent_id,
                            messages,
                            system_prompt,
                            tools,
                            context_window_tokens,
                        )
                        .await;
                }
            }
        }

        match self
            .call_hook_dispatch(
                "assemble",
                script,
                input,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                    let assembled: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();

                    if !assembled.is_empty() {
                        Self::record_hook(&self.metrics, "assemble", ms, true);
                        *messages = assembled;
                        return Ok(AssembleResult {
                            recovery: crate::context_overflow::RecoveryStage::None,
                        });
                    }
                    warn!("Assemble hook returned empty messages, falling back to default");
                } else {
                    warn!("Assemble hook returned no 'messages' field, falling back to default");
                }
                Self::record_hook(&self.metrics, "assemble", ms, false);
                self.inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "assemble", 0, false);
                self.apply_failure_policy("assemble", &e)?;
                self.inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await
            }
        }
    }

    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
        model: &str,
        context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult> {
        let Some(ref script) = self.compact_script else {
            return self
                .inner
                .compact(agent_id, messages, driver, model, context_window_tokens)
                .await;
        };

        // Serialize full message structure — tool_use/tool_result blocks preserved
        let msg_values: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        // Build token pressure metadata for the compact hook.
        let used_tokens = crate::compactor::estimate_token_count(messages, None, None);
        let max_ctx = if context_window_tokens > 0 {
            context_window_tokens
        } else {
            100_000
        };
        let pressure = (used_tokens as f64 / max_ctx as f64).min(1.0);
        let recommendation = match pressure {
            p if p >= 0.9 => "critical",
            p if p >= 0.8 => "aggressive",
            p if p >= 0.6 => "moderate",
            _ => "light",
        };
        let token_pressure = serde_json::json!({
            "used_tokens": used_tokens,
            "max_tokens": max_ctx,
            "pressure": pressure,
            "recommendation": recommendation,
        });

        let mut input = serde_json::json!({
            "type": "compact",
            "agent_id": agent_id.0.to_string(),
            "messages": msg_values,
            "model": model,
            "context_window_tokens": context_window_tokens,
        });
        if let Some(obj) = input.as_object_mut() {
            obj.insert("token_pressure".to_string(), token_pressure);
        }

        // TTL-based cache for compact hook.
        if let Some(ttl_secs) = self.compact_cache_ttl_secs {
            let cache_key = crate::plugin_manager::sha256_hex(
                serde_json::to_string(&input).unwrap_or_default().as_bytes(),
            );
            let cached = {
                let guard = self.compact_cache.lock().unwrap();
                guard.get(&cache_key).and_then(|(val, inserted_at)| {
                    if inserted_at.elapsed().as_secs() < ttl_secs {
                        Some(val.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(cached_output) = cached {
                debug!("Compact hook cache hit (ttl={}s)", ttl_secs);
                if let Some(result) = try_host_summary(&cached_output, &driver, model).await {
                    return Ok(result);
                }
                if let Some(new_msgs) = cached_output.get("messages").and_then(|v| v.as_array()) {
                    let compacted: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    if !compacted.is_empty() {
                        let summary = cached_output
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("plugin compaction (cached)")
                            .to_string();
                        let removed = messages.len().saturating_sub(compacted.len());
                        return Ok(CompactionResult {
                            summary,
                            kept_messages: compacted,
                            compacted_count: removed,
                            chunks_used: 1,
                            used_fallback: false,
                        });
                    }
                }
                return self
                    .inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await;
            }
            // Cache miss — run hook and store result.
            let cache_arc = self.compact_cache.clone();
            let result = self
                .call_hook_dispatch(
                    "compact",
                    script,
                    input,
                    self.hook_timeout_secs,
                    Some(&agent_id),
                )
                .await;
            match result {
                Ok((output, ms)) => {
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key, (output.clone(), std::time::Instant::now()));
                        if guard.len() > 256 {
                            guard.retain(|_, (_, inserted_at)| {
                                inserted_at.elapsed().as_secs() < ttl_secs
                            });
                        }
                    }
                    if let Some(result) = try_host_summary(&output, &driver, model).await {
                        Self::record_hook(&self.metrics, "compact", ms, true);
                        return Ok(result);
                    }
                    if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                        let compacted: Vec<Message> = new_msgs
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .collect();
                        if !compacted.is_empty() {
                            Self::record_hook(&self.metrics, "compact", ms, true);
                            let summary = output
                                .get("summary")
                                .and_then(|v| v.as_str())
                                .unwrap_or("plugin compaction")
                                .to_string();
                            let removed = messages.len().saturating_sub(compacted.len());
                            return Ok(CompactionResult {
                                summary,
                                kept_messages: compacted,
                                compacted_count: removed,
                                chunks_used: 1,
                                used_fallback: false,
                            });
                        }
                    }
                    Self::record_hook(&self.metrics, "compact", ms, false);
                    return self
                        .inner
                        .compact(agent_id, messages, driver, model, context_window_tokens)
                        .await;
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "compact", 0, false);
                    self.apply_failure_policy("compact", &e)?;
                    return self
                        .inner
                        .compact(agent_id, messages, driver, model, context_window_tokens)
                        .await;
                }
            }
        }

        match self
            .call_hook_dispatch(
                "compact",
                script,
                input,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                if let Some(result) = try_host_summary(&output, &driver, model).await {
                    Self::record_hook(&self.metrics, "compact", ms, true);
                    return Ok(result);
                }
                if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                    let compacted: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();

                    if !compacted.is_empty() {
                        Self::record_hook(&self.metrics, "compact", ms, true);
                        let summary = output
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("plugin compaction")
                            .to_string();
                        let removed = messages.len().saturating_sub(compacted.len());
                        return Ok(CompactionResult {
                            summary,
                            kept_messages: compacted,
                            compacted_count: removed,
                            chunks_used: 1,
                            used_fallback: false,
                        });
                    }
                    warn!("Compact hook returned empty messages, falling back to default");
                } else {
                    warn!("Compact hook returned no 'messages' field, falling back to default");
                }
                Self::record_hook(&self.metrics, "compact", ms, false);
                self.inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "compact", 0, false);
                self.apply_failure_policy("compact", &e)?;
                self.inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await
            }
        }
    }

    async fn transform_tool_result(
        &self,
        agent_id: AgentId,
        tool_name: &str,
        tool_use_id: &str,
        input: &serde_json::Value,
        content: &str,
        is_error: bool,
        status: librefang_types::tool::ToolExecutionStatus,
    ) -> LibreFangResult<Option<String>> {
        let Some(ref script) = self.transform_tool_result_script else {
            return Ok(None);
        };

        if !self.agent_passes_filter(&agent_id) {
            return Ok(None);
        }

        let input_json = serde_json::json!({
            "type": "transform_tool_result",
            "agent_id": agent_id.0.to_string(),
            "tool_name": tool_name,
            "tool_use_id": tool_use_id,
            "input": input,
            "content": content,
            "is_error": is_error,
            "status": status,
        });

        match self
            .call_hook_dispatch(
                "transform_tool_result",
                script,
                input_json,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                Self::record_hook(&self.metrics, "transform_tool_result", ms, true);
                self.record_per_agent(&agent_id, ms, true);
                Ok(output
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned))
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "transform_tool_result", 0, false);
                self.record_per_agent(&agent_id, 0, false);
                self.apply_failure_policy("transform_tool_result", &e)?;
                Ok(None)
            }
        }
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        // Run default after_turn first
        self.inner.after_turn(agent_id, messages).await?;

        // If no after_turn script, we're done
        let Some(ref script) = self.after_turn_script else {
            return Ok(());
        };

        // Send full message structure so scripts can index tool_use/tool_result/image blocks.
        let msg_values: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        let input = serde_json::json!({
            "type": "after_turn",
            "agent_id": agent_id.0.to_string(),
            "messages": msg_values,
        });

        // Spawn as fire-and-forget — after_turn is best-effort, don't block the agent.
        // Log if the task panics so failures aren't silently swallowed.

        // Circuit-breaker check: skip spawning if the circuit is already open.
        if self.circuit_is_open("after_turn", Some(&agent_id)) {
            debug!("after_turn hook skipped — circuit breaker is open");
            return Ok(());
        }

        // Apply agent_id_filter for after_turn hook.
        if !self.agent_passes_filter(&agent_id) {
            return Ok(());
        }

        let script = script.clone();
        let runtime = self.runtime.clone();
        let timeout_secs = self.hook_timeout_secs;
        // Merge bootstrap env overrides into the env passed to the background task.
        let plugin_env = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut env = self.plugin_env.clone();
            for (k, v) in &guard.env_overrides {
                if !env.iter().any(|(ek, _)| ek == k) {
                    env.push((k.clone(), v.clone()));
                }
            }
            env
        };
        let metrics = std::sync::Arc::clone(&self.metrics);
        let max_retries = self.max_retries;
        let retry_delay_ms = self.retry_delay_ms;
        let max_memory_mb = self.max_memory_mb;
        let allow_network = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard.allow_network.unwrap_or(self.allow_network)
        };
        let traces = std::sync::Arc::clone(&self.traces);
        let hook_schemas = self.hook_schemas.clone();
        let persistent_subprocess = self.persistent_subprocess;
        let process_pool = std::sync::Arc::clone(&self.process_pool);
        let sem = std::sync::Arc::clone(&self.after_turn_sem);
        let trace_store = self.trace_store.clone();
        let plugin_name = self.plugin_name.clone();
        let agent_id_str = agent_id.0.to_string();
        // Compute agent-scoped state path for this after_turn call.
        let shared_state_path = self
            .shared_state_path
            .as_deref()
            .map(|p| agent_scoped_state_path(p, Some(agent_id_str.as_str())));
        let memory_substrate = std::sync::Arc::clone(&self.memory_substrate);
        let output_schema_strict = self.inner.config.output_schema_strict;
        let after_turn_correlation_id = generate_trace_id();
        let event_bus_arc = self.event_bus.clone();
        // Clone circuit-breaker state for updating from the background task.
        let cb_breakers = std::sync::Arc::clone(&self.circuit_breakers);
        let cb_cfg = self.circuit_breaker_cfg.clone();
        let cb_trace_store = self.trace_store.clone();
        {
            let mut tasks = self.after_turn_tasks.lock().await;
            // Reap already-completed tasks to prevent unbounded growth.
            while tasks.try_join_next().is_some() {}

            let correlation_id_at = after_turn_correlation_id.clone();
            tasks.spawn(async move {
                // Bounded concurrency: acquire a semaphore permit before running the hook.
                // `.ok()` is intentional: if the semaphore is closed (daemon shutting down),
                // `acquire()` returns `Err(AcquireError)`. Ignoring it with `.ok()` lets the
                // task complete its current hook call cleanly instead of panicking.  The permit
                // is held for the lifetime of this spawned task via the `_permit` binding.
                let _permit = sem.acquire().await.ok();
                let result = if persistent_subprocess {
                    let config = crate::plugin_runtime::HookConfig {
                        timeout_secs,
                        plugin_env: plugin_env.clone(),
                        max_memory_mb,
                        allow_network,
                        state_file: shared_state_path.clone(),
                        ..Default::default()
                    };
                    let trace_id = generate_trace_id();
                    let input_preview = if input.to_string().len() > 2048 {
                        serde_json::json!({"_truncated": true, "type": input.get("type")})
                    } else {
                        input.clone()
                    };
                    let started_at = chrono::Utc::now().to_rfc3339();
                    let t = std::time::Instant::now();
                    let call_result = process_pool.call(&script, runtime, &input, &config).await;
                    let elapsed_ms = t.elapsed().as_millis() as u64;
                    match call_result {
                        Ok(output) => {
                            Self::push_trace(
                                &traces,
                                HookTrace {
                                    trace_id: trace_id.clone(),
                                    correlation_id: correlation_id_at.clone(),
                                    hook: "after_turn".to_string(),
                                    started_at,
                                    elapsed_ms,
                                    success: true,
                                    error: None,
                                    input_preview,
                                    output_preview: Some(output.clone()),
                                    annotations: output.get("annotations").cloned(),
                                },
                                trace_store.as_ref(),
                                &plugin_name,
                            )
                            .await;
                            Ok((output, elapsed_ms))
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            Self::push_trace(
                                &traces,
                                HookTrace {
                                    trace_id: trace_id.clone(),
                                    correlation_id: correlation_id_at.clone(),
                                    hook: "after_turn".to_string(),
                                    started_at,
                                    elapsed_ms,
                                    success: false,
                                    error: Some(err_msg.clone()),
                                    input_preview,
                                    output_preview: None,
                                    annotations: None,
                                },
                                trace_store.as_ref(),
                                &plugin_name,
                            )
                            .await;
                            Err(err_msg)
                        }
                    }
                } else {
                    Self::run_hook(
                        "after_turn",
                        &script,
                        runtime,
                        input,
                        timeout_secs,
                        &plugin_env,
                        max_retries,
                        retry_delay_ms,
                        max_memory_mb,
                        allow_network,
                        &traces,
                        &hook_schemas,
                        shared_state_path.as_deref(),
                        trace_store.as_ref(),
                        &plugin_name,
                        &correlation_id_at,
                        output_schema_strict,
                    )
                    .await
                };
                let success = result.is_ok();
                match result {
                    Ok((output, ms)) => {
                        Self::record_hook(&metrics, "after_turn", ms, true);
                        debug!("After-turn hook completed ({ms}ms)");
                        // Inspect hook output for memories, logs, and annotations.
                        Self::process_after_turn_output(
                            &output,
                            &agent_id_str,
                            Some(&memory_substrate),
                            &plugin_name,
                            event_bus_arc.as_ref(),
                        );
                    }
                    Err(e) => {
                        Self::record_hook(&metrics, "after_turn", 0, false);
                        warn!("After-turn hook failed: {e}");
                    }
                }
                // Update circuit breaker from the background task so that repeated
                // after_turn failures can trip the circuit and stop future spawns.
                if let Some(ref cfg) = cb_cfg {
                    let key = format!("{}:after_turn", agent_id_str);
                    let (failures, opened_at_rfc3339, just_reset) = {
                        let mut guard = cb_breakers.lock().unwrap();
                        let state = guard
                            .entry(key.clone())
                            .or_insert_with(CircuitBreakerState::new);
                        if success {
                            state.record_success();
                            (0u32, None::<String>, true)
                        } else {
                            state.record_failure(cfg.max_failures);
                            if state.consecutive_failures == cfg.max_failures {
                                warn!(
                                    hook = "after_turn",
                                    cooldown_secs = cfg.reset_secs,
                                    "Hook circuit breaker opened"
                                );
                            }
                            let opened_str = state.opened_at.map(|instant| {
                                let elapsed = instant.elapsed();
                                (chrono::Utc::now()
                                    - chrono::Duration::from_std(elapsed).unwrap_or_default())
                                .to_rfc3339()
                            });
                            (state.consecutive_failures, opened_str, false)
                        }
                    };
                    if let Some(ref store) = cb_trace_store {
                        if just_reset {
                            let _ = store.delete_circuit_state(&key);
                        } else {
                            let _ = store.save_circuit_state(
                                &key,
                                failures,
                                opened_at_rfc3339.as_deref(),
                            );
                        }
                    }
                }
            });
        }

        Ok(())
    }

    async fn prepare_subagent_context(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
    ) -> LibreFangResult<()> {
        self.inner
            .prepare_subagent_context(parent_id, child_id)
            .await?;

        if let Some(ref script) = self.prepare_subagent_script {
            let input = serde_json::json!({
                "type": "prepare_subagent",
                "parent_id": parent_id.0.to_string(),
                "child_id": child_id.0.to_string(),
            });
            match self
                .call_hook_dispatch(
                    "prepare_subagent",
                    script,
                    input,
                    self.hook_timeout_secs,
                    None,
                )
                .await
            {
                Ok((_, ms)) => {
                    Self::record_hook(&self.metrics, "prepare_subagent", ms, true);
                    debug!("Prepare-subagent hook completed ({ms}ms)");
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "prepare_subagent", 0, false);
                    self.apply_failure_policy("prepare_subagent", &e)?;
                }
            }
        }

        Ok(())
    }

    async fn merge_subagent_context(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
    ) -> LibreFangResult<()> {
        self.inner
            .merge_subagent_context(parent_id, child_id)
            .await?;

        if let Some(ref script) = self.merge_subagent_script {
            let input = serde_json::json!({
                "type": "merge_subagent",
                "parent_id": parent_id.0.to_string(),
                "child_id": child_id.0.to_string(),
            });
            match self
                .call_hook_dispatch(
                    "merge_subagent",
                    script,
                    input,
                    self.hook_timeout_secs,
                    None,
                )
                .await
            {
                Ok((_, ms)) => {
                    Self::record_hook(&self.metrics, "merge_subagent", ms, true);
                    debug!("Merge-subagent hook completed ({ms}ms)");
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "merge_subagent", 0, false);
                    self.apply_failure_policy("merge_subagent", &e)?;
                }
            }
        }

        Ok(())
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        self.inner
            .truncate_tool_result(content, context_window_tokens)
    }

    fn hook_metrics(&self) -> Option<HookMetrics> {
        Some(self.metrics())
    }

    fn hook_traces(&self) -> Vec<HookTrace> {
        self.traces_snapshot()
    }

    fn per_agent_metrics(&self) -> std::collections::HashMap<String, HookStats> {
        self.per_agent_metrics_snapshot()
    }
}

#[cfg(test)]
mod request_llm_summary_tests {
    use super::*;
    use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError};
    use async_trait::async_trait;
    use librefang_memory::MemorySubstrate;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
    use librefang_types::tool::ToolExecutionStatus;
    use std::sync::Arc;

    fn make_transform_script(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("rewrite.sh");
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        (tmp, script)
    }

    fn make_transform_engine(
        hooks: librefang_types::config::ContextEngineHooks,
    ) -> ScriptableContextEngine {
        let inner = DefaultContextEngine::new(
            ContextEngineConfig::default(),
            Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap()),
            None,
        );
        ScriptableContextEngine::new(inner, &hooks)
    }

    /// Stub driver that echoes a fixed summary, so the test asserts the host
    /// actually ran the LLM for a `request_llm_summary` directive.
    struct StubDriver {
        text: String,
    }

    #[async_trait]
    impl LlmDriver for StubDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.text.clone(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        }
    }

    fn driver(text: &str) -> std::sync::Arc<dyn LlmDriver> {
        std::sync::Arc::new(StubDriver {
            text: text.to_string(),
        })
    }

    #[tokio::test]
    async fn runs_host_llm_and_keeps_the_kept_messages() {
        let output = serde_json::json!({
            "request_llm_summary": {
                "prompt": "Summarize tersely.",
                "summarize": [Message::assistant("old turn to fold away")],
                "keep": [Message::user("recent message to keep verbatim")],
                "max_tokens": 256
            }
        });
        let result = try_host_summary(&output, &driver("HOST SUMMARY"), "test-model")
            .await
            .expect("request_llm_summary should produce a CompactionResult");
        assert_eq!(result.summary, "HOST SUMMARY");
        assert_eq!(result.kept_messages.len(), 1);
        assert_eq!(
            result.kept_messages[0].content.text_content(),
            "recent message to keep verbatim"
        );
        assert_eq!(result.compacted_count, 1);
        assert!(!result.used_fallback);
    }

    #[tokio::test]
    async fn no_directive_returns_none() {
        // Output with the legacy `messages` shape carries no directive.
        let output = serde_json::json!({ "messages": [] });
        assert!(try_host_summary(&output, &driver("x"), "m").await.is_none());
    }

    #[tokio::test]
    async fn empty_summarize_falls_back() {
        let output = serde_json::json!({
            "request_llm_summary": { "summarize": [], "keep": [Message::user("k")] }
        });
        assert!(try_host_summary(&output, &driver("x"), "m").await.is_none());
    }

    #[tokio::test]
    async fn transform_tool_result_script_rewrites_content() {
        let (_tmp, script) = make_transform_script(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"content\":\"cleaned cargo output\"}'\n",
        );
        let hooks = librefang_types::config::ContextEngineHooks {
            transform_tool_result: Some(script.to_string_lossy().to_string()),
            runtime: Some("native".to_string()),
            ..Default::default()
        };
        let engine = make_transform_engine(hooks);

        let rewritten = engine
            .transform_tool_result(
                AgentId::from_name("rust-agent"),
                "shell",
                "toolu_1",
                &serde_json::json!({"command": "cargo check"}),
                "Compiling noisy crate\nerror[E0308]: mismatched types",
                true,
                ToolExecutionStatus::Error,
            )
            .await
            .unwrap();

        assert_eq!(rewritten.as_deref(), Some("cleaned cargo output"));
        assert_eq!(engine.metrics().transform_tool_result.calls, 1);
        assert_eq!(engine.metrics().transform_tool_result.successes, 1);
    }

    #[tokio::test]
    async fn transform_tool_result_missing_content_is_noop() {
        let (_tmp, script) =
            make_transform_script("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}'\n");
        let hooks = librefang_types::config::ContextEngineHooks {
            transform_tool_result: Some(script.to_string_lossy().to_string()),
            runtime: Some("native".to_string()),
            ..Default::default()
        };
        let engine = make_transform_engine(hooks);

        let rewritten = engine
            .transform_tool_result(
                AgentId::from_name("rust-agent"),
                "shell",
                "toolu_1",
                &serde_json::json!({"command": "cargo check"}),
                "raw cargo output",
                false,
                ToolExecutionStatus::Completed,
            )
            .await
            .unwrap();

        assert_eq!(rewritten, None);
        assert_eq!(engine.metrics().transform_tool_result.calls, 1);
        assert_eq!(engine.metrics().transform_tool_result.successes, 1);
    }

    #[tokio::test]
    async fn transform_tool_result_script_failure_is_noop_with_warn_policy() {
        let (_tmp, script) = make_transform_script("#!/bin/sh\nexit 42\n");
        let hooks = librefang_types::config::ContextEngineHooks {
            transform_tool_result: Some(script.to_string_lossy().to_string()),
            runtime: Some("native".to_string()),
            ..Default::default()
        };
        let engine = make_transform_engine(hooks);

        let rewritten = engine
            .transform_tool_result(
                AgentId::from_name("rust-agent"),
                "shell",
                "toolu_1",
                &serde_json::json!({"command": "cargo check"}),
                "raw cargo output",
                true,
                ToolExecutionStatus::Error,
            )
            .await
            .unwrap();

        assert_eq!(rewritten, None);
        assert_eq!(engine.metrics().transform_tool_result.calls, 1);
        assert_eq!(engine.metrics().transform_tool_result.failures, 1);
    }

    #[tokio::test]
    async fn transform_tool_result_agent_filter_skips_hook() {
        let (_tmp, script) = make_transform_script(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"content\":\"wrong\"}'\n",
        );
        let hooks = librefang_types::config::ContextEngineHooks {
            transform_tool_result: Some(script.to_string_lossy().to_string()),
            runtime: Some("native".to_string()),
            only_for_agent_ids: vec!["android-agent".to_string()],
            ..Default::default()
        };
        let engine = make_transform_engine(hooks);

        let rewritten = engine
            .transform_tool_result(
                AgentId::from_name("rust-agent"),
                "shell",
                "toolu_1",
                &serde_json::json!({"command": "cargo check"}),
                "raw cargo output",
                false,
                ToolExecutionStatus::Completed,
            )
            .await
            .unwrap();

        assert_eq!(rewritten, None);
        assert_eq!(engine.metrics().transform_tool_result.calls, 0);
    }
}
