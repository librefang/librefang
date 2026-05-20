//! Pre-LLM prompt setup: PII-filtered user-message push, A/B experiment
//! selection, memory recall, system-prompt build, and message-list prep
//! through the session repair / trim pipeline.

use super::*;

pub(super) fn push_filtered_user_message(
    session: &mut Session,
    user_message: &str,
    user_content_blocks: Option<Vec<ContentBlock>>,
    pii_filter: &crate::pii_filter::PiiFilter,
    privacy_config: &librefang_types::config::PrivacyConfig,
    sender_prefix: Option<&str>,
) {
    let prefix = sender_prefix.unwrap_or("");
    if let Some(blocks) = user_content_blocks {
        let mut filtered_blocks: Vec<ContentBlock> =
            if privacy_config.mode != librefang_types::config::PrivacyMode::Off {
                blocks
                    .into_iter()
                    .map(|block| match block {
                        ContentBlock::Text {
                            text,
                            provider_metadata,
                        } => ContentBlock::Text {
                            text: pii_filter.filter_message(&text, &privacy_config.mode),
                            provider_metadata,
                        },
                        other => other,
                    })
                    .collect()
            } else {
                blocks
            };
        // Prepend the sanitized sender prefix to the first Text block (if any) so
        // the LLM sees "[Alice]: hello" but PII filter only ran over the raw text.
        if !prefix.is_empty() {
            if let Some(first_text) = filtered_blocks.iter_mut().find_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text),
                _ => None,
            }) {
                *first_text = format!("{prefix}{first_text}");
            } else {
                // No text block at all (e.g. image-only message) — insert a text block carrying the prefix.
                filtered_blocks.insert(
                    0,
                    ContentBlock::Text {
                        text: prefix.trim_end().to_string(),
                        provider_metadata: None,
                    },
                );
            }
        }
        session.push_message(Message::user_with_blocks(filtered_blocks));
    } else {
        let filtered_message = pii_filter.filter_message(user_message, &privacy_config.mode);
        let final_message = if prefix.is_empty() {
            filtered_message
        } else {
            format!("{prefix}{filtered_message}")
        };
        session.push_message(Message::user(&final_message));
    }
}

pub(super) async fn remember_interaction_best_effort(
    memory: &MemorySubstrate,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    agent_id: librefang_types::agent::AgentId,
    interaction_text: &str,
    streaming: bool,
    peer_id: Option<&str>,
) {
    if let Some(emb) = embedding_driver {
        match emb.embed_one(interaction_text).await {
            Ok(vec) => {
                if let Err(e) = memory
                    .remember_with_embedding_async(
                        agent_id,
                        interaction_text,
                        MemorySource::Conversation,
                        "episodic",
                        HashMap::new(),
                        Some(&vec),
                        peer_id,
                    )
                    .await
                {
                    warn!(
                        error = %e,
                        remember_context = if streaming { "streaming" } else { "non_streaming" },
                        "Failed to persist episodic memory with embedding"
                    );
                }
            }
            Err(e) => {
                warn!(
                    error = %e,
                    remember_context = if streaming { "streaming" } else { "non_streaming" },
                    "Embedding for remember failed; falling back to plain memory"
                );
                if let Err(e2) = memory
                    .remember(
                        agent_id,
                        interaction_text,
                        MemorySource::Conversation,
                        "episodic",
                        HashMap::new(),
                        peer_id,
                    )
                    .await
                {
                    warn!(
                        error = %e2,
                        remember_context = if streaming { "streaming" } else { "non_streaming" },
                        "Failed to persist episodic memory after embedding fallback"
                    );
                }
            }
        }
    } else if let Err(e) = memory
        .remember(
            agent_id,
            interaction_text,
            MemorySource::Conversation,
            "episodic",
            HashMap::new(),
            peer_id,
        )
        .await
    {
        warn!(
            error = %e,
            remember_context = if streaming { "streaming" } else { "non_streaming" },
            "Failed to persist episodic memory"
        );
    }
}

/// Convert a proactive `MemoryItem` into the `MemoryFragment` format used by the agent loop.
fn proactive_item_to_fragment(
    item: librefang_types::memory::MemoryItem,
    agent_id: librefang_types::agent::AgentId,
) -> MemoryFragment {
    let memory_id = MemoryId(uuid::Uuid::parse_str(&item.id).unwrap_or_else(|err| {
        let fallback = uuid::Uuid::new_v4();
        warn!(
            invalid_memory_id = %item.id,
            fallback_id = %fallback,
            error = %err,
            "Invalid proactive memory id; using generated UUID"
        );
        fallback
    }));

    MemoryFragment {
        id: memory_id,
        agent_id,
        content: item.content,
        embedding: None,
        metadata: item.metadata,
        source: librefang_types::memory::MemorySource::Conversation,
        confidence: 1.0,
        created_at: item.created_at,
        accessed_at: chrono::Utc::now(),
        access_count: 0,
        scope: item.level.scope_str().to_string(),
        image_url: None,
        image_embedding: None,
        modality: Default::default(),
    }
}

pub(super) struct PromptExperimentSelection {
    pub(super) experiment_context: Option<ExperimentContext>,
    pub(super) running_experiment: Option<librefang_types::agent::PromptExperiment>,
}

pub(super) struct RecallSetup {
    pub(super) memories: Vec<MemoryFragment>,
    pub(super) memories_used: Vec<String>,
}

pub(super) struct RecallSetupContext<'a> {
    pub(super) session: &'a Session,
    pub(super) user_message: &'a str,
    pub(super) memory: &'a MemorySubstrate,
    pub(super) embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    pub(super) proactive_memory: Option<&'a Arc<librefang_memory::ProactiveMemoryStore>>,
    pub(super) context_engine: Option<&'a dyn ContextEngine>,
    pub(super) sender_user_id: Option<&'a str>,
    /// Bare channel type (`"telegram"`, `"slack"`, `"whatsapp"`, …) used
    /// for ACL resolution (`KernelHandle::memory_acl_for_sender`) and
    /// kernel-internal sentinel matching (`cron`, `autonomous`, `webui`).
    /// MUST stay bare — `memory_acl_for_sender` looks the channel up in
    /// `format!("{ch}:{sid}")` form, and a chat-suffixed channel would
    /// miss the ACL index.
    pub(super) sender_channel: Option<&'a str>,
    /// Chat-qualified scope (`"telegram:<chatId>"`, `"slack:<channelId>"`,
    /// `"whatsapp:<jid>"`, …) used for the #5227 cross-chat memory-bleed
    /// filter. Produced by `compose_sender_scope(channel, chat_id)` at
    /// the kernel inject site so it matches the formula
    /// `SessionId::for_sender_scope` uses. `None` for non-channel callers
    /// (dashboard, direct API, CLI) — the filter then degrades to a
    /// no-op, preserving legacy recall behaviour.
    pub(super) sender_chat_scope: Option<&'a str>,
    /// Optional kernel handle used to resolve the per-user memory ACL
    /// (RBAC M3, #3054). When `None` the auto-retrieve path runs without
    /// a guard — preserving pre-M3 single-user behaviour.
    pub(super) kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub(super) stable_prefix_mode: bool,
    pub(super) streaming: bool,
    pub(super) opts: &'a LoopOptions,
}

pub(super) struct PromptSetup {
    pub(super) system_prompt: String,
    pub(super) memory_context_msg: Option<String>,
}

pub(super) struct PromptSetupContext<'a> {
    pub(super) manifest: &'a AgentManifest,
    pub(super) session: &'a Session,
    pub(super) kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub(super) experiment_context: Option<&'a ExperimentContext>,
    pub(super) running_experiment: Option<&'a librefang_types::agent::PromptExperiment>,
    pub(super) memories: &'a [MemoryFragment],
    pub(super) stable_prefix_mode: bool,
    pub(super) streaming: bool,
}

pub(super) struct PreparedMessages {
    pub(super) messages: Vec<Message>,
    pub(super) new_messages_start: usize,
    pub(super) repair_stats: crate::session_repair::RepairStats,
}

pub(super) fn reply_directives_from_parsed(
    parsed_directives: crate::reply_directives::DirectiveSet,
) -> librefang_types::message::ReplyDirectives {
    librefang_types::message::ReplyDirectives {
        reply_to: parsed_directives.reply_to,
        current_thread: parsed_directives.current_thread,
        silent: parsed_directives.silent,
    }
}

pub(super) fn select_running_experiment(
    manifest: &AgentManifest,
    session: &Session,
    kernel: Option<&Arc<dyn KernelHandle>>,
    streaming: bool,
) -> PromptExperimentSelection {
    let mut experiment_context: Option<ExperimentContext> = None;
    let mut running_experiment: Option<librefang_types::agent::PromptExperiment> = None;
    if let Some(kernel) = kernel {
        let agent_id = session.agent_id.to_string();
        match kernel.get_running_experiment(&agent_id) {
            Ok(Some(exp)) => {
                running_experiment = Some(exp.clone());
                if !exp.variants.is_empty() {
                    let hash_val = (session.id.0.as_u128() % 100) as u8;
                    let mut cumulative = 0u8;
                    let mut variant_index = 0;
                    for (i, &weight) in exp.traffic_split.iter().enumerate() {
                        cumulative = cumulative.saturating_add(weight);
                        if hash_val < cumulative {
                            variant_index = i;
                            break;
                        }
                    }
                    variant_index = variant_index.min(exp.variants.len() - 1);
                    let variant = &exp.variants[variant_index];
                    info!(
                        agent = %manifest.name,
                        experiment = %exp.name,
                        variant = %variant.name,
                        index = variant_index,
                        "A/B experiment active - using variant{}",
                        if streaming { " (streaming)" } else { "" }
                    );
                    experiment_context = Some(ExperimentContext::new(
                        exp.id,
                        variant.id,
                        variant.name.clone(),
                    ));
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, "get_running_experiment failed");
            }
        }
    }

    PromptExperimentSelection {
        experiment_context,
        running_experiment,
    }
}

pub(super) async fn setup_recalled_memories(ctx: RecallSetupContext<'_>) -> RecallSetup {
    // #5227: when an active chat scope is supplied, ask the substrate for
    // a wider candidate window so the per-scope post-filter below has
    // enough headroom to keep `MEMORY_RECALL_LIMIT` legitimate results.
    // Without the inflation a substrate query that returns ~5 memories
    // all stamped for the *other* chat would leave zero results after
    // filtering. Matches the inflation factor used by `auto_retrieve`.
    const MEMORY_RECALL_LIMIT: usize = 5;
    // Use the chat-qualified scope (`"telegram:<chatId>"`) for the
    // #5227 filter, not the bare `sender_channel` (`"telegram"`). On
    // Telegram / Slack / Discord native bridges the latter is identical
    // across DM and group of the same peer, which would make the filter
    // a no-op (#5227 follow-up). The kernel inject sites stamp both
    // keys — see `messaging.rs::send_message_full_inner` and
    // `agent_execution.rs::execute_llm_agent`.
    let chat_scope_active = ctx
        .sender_chat_scope
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    let recall_fetch_limit = if chat_scope_active {
        (MEMORY_RECALL_LIMIT * 4).max(50)
    } else {
        MEMORY_RECALL_LIMIT
    };
    let mut memories = if let Some(engine) = ctx.context_engine {
        // The context engine's `ingest` uses its own (typically small,
        // default 5) recall budget and is unaware of `chat_scope`. When
        // a chat scope is active, its top-N can be dominated by
        // OTHER-chat memories that the post-filter at the end of this
        // function will drop, leaving zero results for the active chat
        // even though same-scope rows existed just below the engine's
        // cut-off (P2, #5227 second-pass review).
        //
        // Mitigation: after the engine call, run a supplemental
        // substrate recall with the widened `recall_fetch_limit` and
        // merge the new rows (by id) into the engine's result. The
        // post-filter then runs over the union, so same-scope rows that
        // the engine missed get a chance to land in the prompt. Engines
        // that already scope-filter internally will return the same
        // top-N as the supplemental fetch and the merge becomes a no-op.
        let mut engine_mem = recall_or_default(
            engine
                .ingest(ctx.session.agent_id, ctx.user_message, ctx.sender_user_id)
                .await
                .map(|r| r.recalled_memories),
            if ctx.streaming {
                "Context engine ingest failed (streaming); continuing without recalled memories"
            } else {
                "Context engine ingest failed; continuing without recalled memories"
            },
        );
        if chat_scope_active && !ctx.stable_prefix_mode {
            let extra = if let Some(emb) = ctx.embedding_driver {
                match emb.embed_one(ctx.user_message).await {
                    Ok(qv) => recall_or_default(
                        ctx.memory
                            .recall_with_embedding_async(
                                ctx.user_message,
                                recall_fetch_limit,
                                Some(MemoryFilter {
                                    agent_id: Some(ctx.session.agent_id),
                                    peer_id: ctx.sender_user_id.map(str::to_owned),
                                    ..Default::default()
                                }),
                                Some(&qv),
                            )
                            .await,
                        "Supplemental vector recall failed alongside context engine; \
                         continuing with engine-only results",
                    ),
                    Err(_) => recall_or_default(
                        ctx.memory
                            .recall(
                                ctx.user_message,
                                recall_fetch_limit,
                                Some(MemoryFilter {
                                    agent_id: Some(ctx.session.agent_id),
                                    peer_id: ctx.sender_user_id.map(str::to_owned),
                                    ..Default::default()
                                }),
                            )
                            .await,
                        "Supplemental text recall failed alongside context engine; \
                         continuing with engine-only results",
                    ),
                }
            } else {
                recall_or_default(
                    ctx.memory
                        .recall(
                            ctx.user_message,
                            recall_fetch_limit,
                            Some(MemoryFilter {
                                agent_id: Some(ctx.session.agent_id),
                                peer_id: ctx.sender_user_id.map(str::to_owned),
                                ..Default::default()
                            }),
                        )
                        .await,
                    "Supplemental text recall failed alongside context engine; \
                     continuing with engine-only results",
                )
            };
            // Merge by stable id — keep engine ordering first (it has
            // local re-ranking signals we should preserve), then append
            // substrate rows not already present.
            let seen: std::collections::HashSet<_> = engine_mem.iter().map(|f| f.id.0).collect();
            for frag in extra {
                if !seen.contains(&frag.id.0) {
                    engine_mem.push(frag);
                }
            }
        }
        engine_mem
    } else if ctx.stable_prefix_mode {
        Vec::new()
    } else if let Some(emb) = ctx.embedding_driver {
        match emb.embed_one(ctx.user_message).await {
            Ok(query_vec) => {
                if ctx.streaming {
                    debug!("Using vector recall (streaming, dims={})", query_vec.len());
                } else {
                    debug!("Using vector recall (dims={})", query_vec.len());
                }
                recall_or_default(
                    ctx.memory
                        .recall_with_embedding_async(
                            ctx.user_message,
                            recall_fetch_limit,
                            Some(MemoryFilter {
                                agent_id: Some(ctx.session.agent_id),
                                peer_id: ctx.sender_user_id.map(str::to_owned),
                                ..Default::default()
                            }),
                            Some(&query_vec),
                        )
                        .await,
                    if ctx.streaming {
                        "Vector memory recall failed (streaming); continuing without recalled memories"
                    } else {
                        "Vector memory recall failed; continuing without recalled memories"
                    },
                )
            }
            Err(e) => {
                if ctx.streaming {
                    warn!("Embedding recall failed (streaming), falling back to text search: {e}");
                } else {
                    warn!("Embedding recall failed, falling back to text search: {e}");
                }
                recall_or_default(
                    ctx.memory
                        .recall(
                            ctx.user_message,
                            recall_fetch_limit,
                            Some(MemoryFilter {
                                agent_id: Some(ctx.session.agent_id),
                                peer_id: ctx.sender_user_id.map(str::to_owned),
                                ..Default::default()
                            }),
                        )
                        .await,
                    if ctx.streaming {
                        "Text memory recall failed after embedding fallback (streaming); continuing without recalled memories"
                    } else {
                        "Text memory recall failed after embedding fallback; continuing without recalled memories"
                    },
                )
            }
        }
    } else {
        recall_or_default(
            ctx.memory
                .recall(
                    ctx.user_message,
                    recall_fetch_limit,
                    Some(MemoryFilter {
                        agent_id: Some(ctx.session.agent_id),
                        peer_id: ctx.sender_user_id.map(str::to_owned),
                        ..Default::default()
                    }),
                )
                .await,
            if ctx.streaming {
                "Text memory recall failed (streaming); continuing without recalled memories"
            } else {
                "Text memory recall failed; continuing without recalled memories"
            },
        )
    };

    // #5227: drop fragments whose stored `chat_scope` belongs to a
    // different chat (same agent + same peer, different conversation).
    // `MemoryLevel::User` and untagged legacy rows pass through. The
    // context-engine `ingest` path also funnels here so its results get
    // filtered too — engines that perform their own scope filtering can
    // pass `sender_chat_scope` upstream and this becomes a no-op for them.
    if chat_scope_active {
        let want = ctx.sender_chat_scope.unwrap();
        memories.retain(|frag| {
            librefang_types::memory::memory_scope_allows_recall(&frag.scope, &frag.metadata, want)
        });
    }
    // Truncate AFTER the scope filter, not before. The fetch widened
    // to `recall_fetch_limit = max(MEMORY_RECALL_LIMIT*4, 50)` above
    // specifically so the filter has something to throw away — capping
    // here is what restores the prompt's expected `MEMORY_RECALL_LIMIT`
    // window. When `chat_scope_active == false` the fetch was already
    // capped at `MEMORY_RECALL_LIMIT`, making this `truncate` a no-op.
    memories.truncate(MEMORY_RECALL_LIMIT);

    // Fork turns skip auto_retrieve: (a) it would add memory fragments
    // to the prompt that the parent turn didn't have, breaking byte-
    // alignment with the cached prefix and missing the Anthropic cache
    // entirely; (b) the fork is by definition a short derivative task
    // (dream / memory extraction) whose context should be exactly the
    // parent's, not a fresh retrieval.
    if !ctx.stable_prefix_mode && !ctx.opts.is_fork {
        if let Some(pm_store_arc) = ctx.proactive_memory {
            let user_id = ctx.session.agent_id.0.to_string();
            // RBAC M3 (#3054): build a memory namespace guard from the
            // attributed end user (resolved by the kernel via channel
            // bindings). When the guard denies "proactive" reads we skip
            // the retrieval rather than letting the fragments leak into
            // the LLM prompt. PII redaction is applied to the returned
            // items as well.
            let guard = ctx.kernel.and_then(|kh| {
                kh.memory_acl_for_sender(ctx.sender_user_id, ctx.sender_channel)
                    .map(librefang_memory::namespace_acl::MemoryNamespaceGuard::new)
            });
            let auto_retrieve_result = match guard.as_ref() {
                Some(g) => match g.check_read("proactive") {
                    librefang_memory::namespace_acl::NamespaceGate::Allow => {
                        let mut items = pm_store_arc
                            .auto_retrieve(
                                &user_id,
                                ctx.user_message,
                                ctx.sender_user_id,
                                ctx.sender_chat_scope,
                            )
                            .await;
                        if let Ok(ref mut its) = items {
                            g.redact_all(its);
                        }
                        items
                    }
                    librefang_memory::namespace_acl::NamespaceGate::Deny(reason) => {
                        debug!("Skipping proactive memory auto_retrieve: {reason}",);
                        Ok(Vec::new())
                    }
                },
                None => {
                    pm_store_arc
                        .auto_retrieve(
                            &user_id,
                            ctx.user_message,
                            ctx.sender_user_id,
                            ctx.sender_chat_scope,
                        )
                        .await
                }
            };
            match auto_retrieve_result {
                Ok(pm_memories) if !pm_memories.is_empty() => {
                    if ctx.streaming {
                        debug!(
                            "Proactive memory (streaming) retrieved {} items",
                            pm_memories.len()
                        );
                    } else {
                        debug!("Proactive memory retrieved {} items", pm_memories.len());
                    }
                    let pm_fragments: Vec<_> = pm_memories
                        .into_iter()
                        .map(|item| proactive_item_to_fragment(item, ctx.session.agent_id))
                        .filter(|frag| !memories.iter().any(|m| m.content == frag.content))
                        .collect();
                    memories.extend(pm_fragments);
                }
                Ok(_) => {
                    if ctx.streaming {
                        debug!("No proactive memories retrieved (streaming)");
                    } else {
                        debug!("No proactive memories retrieved");
                    }
                }
                Err(e) => {
                    if ctx.streaming {
                        warn!("Proactive memory auto_retrieve failed (streaming): {e}");
                    } else {
                        warn!("Proactive memory auto_retrieve failed: {e}");
                    }
                }
            }
        }
    }

    let memories_used = memories.iter().map(|m| m.content.clone()).collect();
    RecallSetup {
        memories,
        memories_used,
    }
}

pub(super) fn build_prompt_setup(ctx: PromptSetupContext<'_>) -> PromptSetup {
    let mut system_prompt = ctx.manifest.model.system_prompt.clone();

    if let Some(kernel) = ctx.kernel {
        if let Err(e) = kernel.auto_track_prompt_version(ctx.session.agent_id, &system_prompt) {
            warn!(error = %e, "auto_track_prompt_version failed");
        }
    }

    if let Some(experiment_context) = ctx.experiment_context {
        if let Some(exp) = ctx.running_experiment {
            if let Some(kernel) = ctx.kernel {
                if let Some(variant) = exp
                    .variants
                    .iter()
                    .find(|v| v.id == experiment_context.variant_id)
                {
                    if let Ok(Some(prompt_version)) =
                        kernel.get_prompt_version(&variant.prompt_version_id.to_string())
                    {
                        debug!(
                            agent = %ctx.manifest.name,
                            experiment = %exp.name,
                            variant = %variant.name,
                            version = prompt_version.version,
                            "Using experiment variant prompt version{}",
                            if ctx.streaming { " (streaming)" } else { "" }
                        );
                        system_prompt = prompt_version.system_prompt.clone();
                    }
                }
            }
        }
    }

    let memory_context_msg = if !ctx.memories.is_empty() {
        let mem_pairs: Vec<(String, String)> = ctx
            .memories
            .iter()
            .map(|m| (String::new(), m.content.clone()))
            .collect();
        if ctx.stable_prefix_mode {
            let personal_ctx =
                crate::prompt_builder::format_memory_items_as_personal_context(&mem_pairs);
            Some(personal_ctx)
        } else {
            let section = crate::prompt_builder::build_memory_section(&mem_pairs);
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&section);
            None
        }
    } else {
        None
    };

    // Instruct the model to match the user's language for both thinking and
    // response. Applied unconditionally so it covers models that generate
    // reasoning traces without an explicit thinking config (e.g. Gemma4,
    // Qwen3 via Ollama). Models that cannot follow this instruction are
    // unaffected.
    system_prompt.push_str(
        "\n\nIMPORTANT: Always use the same language as the user's message for both your thinking process and your response.",
    );

    PromptSetup {
        system_prompt,
        memory_context_msg,
    }
}

pub(super) fn prepare_llm_messages(
    manifest: &AgentManifest,
    session: &mut Session,
    user_message: &str,
    memory_context_msg: Option<String>,
    max_history: usize,
) -> PreparedMessages {
    let has_system_messages = session.messages.iter().any(|m| m.role == Role::System);
    let llm_messages: Vec<Message> = if has_system_messages {
        session
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .cloned()
            .collect()
    } else {
        session.messages.clone()
    };

    debug!(
        agent = %manifest.name,
        session_id = %session.id,
        msg_count = llm_messages.len(),
        last_two_roles = ?llm_messages.iter().rev().take(2).map(|m| m.role).collect::<Vec<_>>(),
        "Pre-repair message snapshot (prepare_llm_messages)"
    );

    let (mut messages, repair_stats) = if session.last_repaired_generation
        == Some(session.messages_generation)
    {
        (llm_messages, crate::session_repair::RepairStats::default())
    } else {
        let (msgs, stats) = crate::session_repair::validate_and_repair_with_stats(&llm_messages);
        session.last_repaired_generation = Some(session.messages_generation);
        (msgs, stats)
    };

    if let Some(cc_msg) = manifest
        .metadata
        .get("canonical_context_msg")
        .and_then(|v| v.as_str())
    {
        if !cc_msg.is_empty() {
            messages.insert(0, Message::user(cc_msg));
        }
    }

    if let Some(mem_msg) = memory_context_msg {
        messages.insert(
            0,
            Message::user(format!(
                "[System context — what you know about this person]\n{mem_msg}"
            )),
        );
    }

    let (_working_trimmed, session_trimmed) = safe_trim_messages(
        &mut messages,
        &mut session.messages,
        &manifest.name,
        user_message,
        max_history,
    );
    let new_messages_start = session.messages.len().saturating_sub(1);
    let _working_stripped = strip_prior_image_data(&mut messages);
    let session_stripped = strip_prior_image_data(&mut session.messages);
    if session_trimmed || session_stripped {
        session.mark_messages_mutated();
    }

    PreparedMessages {
        messages,
        new_messages_start,
        repair_stats,
    }
}

/// Emit a single structured log line summarizing any repairs that session
/// repair applied to the outgoing message history. Silent when the history
/// was already well-formed (stats equal to default).
pub(super) fn log_repair_stats(
    manifest: &AgentManifest,
    session: &Session,
    stats: &crate::session_repair::RepairStats,
) {
    if stats == &crate::session_repair::RepairStats::default() {
        return;
    }
    info!(
        agent = %manifest.name,
        session_id = %session.id,
        orphaned = stats.orphaned_results_removed,
        empty = stats.empty_messages_removed,
        merged = stats.messages_merged,
        reordered = stats.results_reordered,
        synthetic = stats.synthetic_results_inserted,
        duplicates = stats.duplicates_removed,
        rescued = stats.misplaced_results_rescued,
        positional_synthetic = stats.positional_synthetic_inserted,
        "Session repair applied fixes before LLM call"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_engine::{ContextEngineConfig, DefaultContextEngine};
    use librefang_memory::MemorySubstrate;
    use librefang_types::agent::{AgentId, SessionId};
    use librefang_types::memory::{MemoryFilter, MemorySource, CHAT_SCOPE_METADATA_KEY};

    fn empty_session(agent_id: AgentId) -> Session {
        Session {
            id: SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
            model_override: None,
            messages_generation: 0,
            last_repaired_generation: None,
            peer_id: None,
        }
    }

    /// #5227 P2 (second-pass review) — when a `ContextEngine` is wired
    /// in, `engine.ingest` uses its OWN small recall budget (default 5)
    /// and is unaware of `chat_scope`. If the substrate has many memories
    /// for the same `(agent, peer)` pair spread across multiple chats,
    /// the engine can return five OTHER-chat rows that get filtered out
    /// of the prompt by the cross-scope filter, leaving zero
    /// same-chat results — even though same-chat rows existed just below
    /// the engine's cut-off.
    ///
    /// The fix is a supplemental substrate recall with the widened
    /// `recall_fetch_limit` after `engine.ingest`, merged by id, so the
    /// post-filter sees the union and same-scope rows have a fair shot
    /// at landing in the prompt.
    ///
    /// Repro: populate `(peer, group_scope)` with 5 distinct memories
    /// and `(peer, dm_scope)` with 3 distinct memories, all matching the
    /// recall query. The engine alone returns 5 group-scope rows. The
    /// post-filter against `dm_scope` previously dropped all 5, leaving
    /// the prompt empty. Post-fix the supplemental fetch (limit
    /// 5 × 4 = 20, floor 50) pulls in the dm rows too and the recall
    /// surfaces all 3.
    #[tokio::test]
    async fn engine_recall_widens_fetch_to_avoid_chat_scope_starvation_5227() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let agent_id = AgentId::new();
        let dm_scope = "telegram:dm-2227";
        let group_scope = "telegram:group--999";

        // Seed via the substrate's public `remember_with_embedding` (no
        // peer scoping — the recall context below also passes
        // `sender_user_id: None`, so the substrate's `peer_id` filter is
        // a no-op and every row participates). The chat-scope filter
        // at the end of `setup_recalled_memories` is what we're
        // exercising here, not peer isolation.
        let write_scoped = |content: &str, scope: &str| {
            let mut meta = std::collections::HashMap::new();
            meta.insert(
                CHAT_SCOPE_METADATA_KEY.to_string(),
                serde_json::Value::String(scope.to_string()),
            );
            substrate
                .remember_with_embedding(
                    agent_id,
                    content,
                    MemorySource::Conversation,
                    librefang_types::memory::MemoryLevel::Session.scope_str(),
                    meta,
                    None,
                    None,
                )
                .unwrap();
        };

        // 5 group-scope rows (will dominate any small-limit recall).
        for i in 0..5 {
            write_scoped(&format!("project Atlas group note {i}"), group_scope);
        }
        // 3 dm-scope rows (the ones we MUST surface in a DM recall).
        for i in 0..3 {
            write_scoped(&format!("project Atlas dm reminder {i}"), dm_scope);
        }

        // Engine with the production default `max_recall_results = 5`.
        let engine_cfg = ContextEngineConfig {
            max_recall_results: 5,
            ..Default::default()
        };
        let engine = DefaultContextEngine::new(engine_cfg, Arc::clone(&substrate), None);

        let session = empty_session(agent_id);
        let opts = LoopOptions::default();
        let setup = setup_recalled_memories(RecallSetupContext {
            session: &session,
            user_message: "project Atlas",
            memory: substrate.as_ref(),
            embedding_driver: None,
            proactive_memory: None,
            context_engine: Some(&engine),
            sender_user_id: None,
            sender_channel: Some("telegram"),
            sender_chat_scope: Some(dm_scope),
            kernel: None,
            stable_prefix_mode: false,
            streaming: false,
            opts: &opts,
        })
        .await;

        // All 3 dm-scope memories must surface. Pre-fix: the engine's
        // top-5 returned only group rows, the filter dropped all of them,
        // and `setup.memories` was empty even though `dm_scope` rows
        // existed in the substrate.
        let dm_hits: Vec<_> = setup
            .memories
            .iter()
            .filter(|f| f.content.contains("dm reminder"))
            .collect();
        assert_eq!(
            dm_hits.len(),
            3,
            "engine-path recall must surface all 3 dm-scope memories \
             after the supplemental fetch fills in candidates the engine \
             missed; got {} dm hits, total memories = {:?}",
            dm_hits.len(),
            setup
                .memories
                .iter()
                .map(|f| &f.content)
                .collect::<Vec<_>>()
        );

        // And no group-scope row may leak into the DM prompt — the
        // post-filter is still doing its job.
        for f in &setup.memories {
            assert!(
                !f.content.contains("group note"),
                "regression: group-scope memory leaked into dm recall via \
                 engine path: {:?}",
                f.content
            );
        }
    }

    /// #5474: `remember_interaction_best_effort` must propagate `peer_id` so
    /// that stored episodic memories carry the sender's user identity and are
    /// reachable by per-user recall (which filters on `(agent_id, peer_id)`).
    #[tokio::test]
    async fn remember_interaction_best_effort_persists_peer_id() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let agent_id = AgentId::new();

        // Write with a known peer_id, no embedding driver (hits the
        // plain-memory fallback path).
        remember_interaction_best_effort(
            substrate.as_ref(),
            None, // no embedding driver
            agent_id,
            "[Past exchange]\nThem: hello\nYou: hi",
            false, // non-streaming
            Some("user-42"),
        )
        .await;

        // Recall with matching peer_id should find the row.
        let results = substrate
            .recall(
                "hello",
                10,
                Some(MemoryFilter {
                    agent_id: Some(agent_id),
                    peer_id: Some("user-42".into()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "peer-scoped recall must find the row");
        assert!(results[0].content.contains("[Past exchange]"));

        // Recall with a different peer_id should return nothing.
        let other = substrate
            .recall(
                "hello",
                10,
                Some(MemoryFilter {
                    agent_id: Some(agent_id),
                    peer_id: Some("other-user".into()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            other.len(),
            0,
            "peer-scoped recall must NOT leak across users"
        );

        // Write with None peer_id, then recall without peer filter should
        // find it, but recall with a specific peer_id should not.
        remember_interaction_best_effort(
            substrate.as_ref(),
            None,
            agent_id,
            "[Past exchange]\nThem: world\nYou: done",
            false,
            None,
        )
        .await;

        let global = substrate
            .recall(
                "world",
                10,
                Some(MemoryFilter {
                    agent_id: Some(agent_id),
                    peer_id: None,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            global.len(),
            1,
            "NULL-peer row must be findable without peer filter"
        );
    }
}
