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
    pub(super) sender_channel: Option<&'a str>,
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
        if let Ok(Some(exp)) = kernel.get_running_experiment(&agent_id) {
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
                experiment_context = Some(ExperimentContext {
                    experiment_id: exp.id,
                    variant_id: variant.id,
                    variant_name: variant.name.clone(),
                    request_start: std::time::Instant::now(),
                });
            }
        }
    }

    PromptExperimentSelection {
        experiment_context,
        running_experiment,
    }
}

pub(super) async fn setup_recalled_memories(ctx: RecallSetupContext<'_>) -> RecallSetup {
    let mut memories = if let Some(engine) = ctx.context_engine {
        recall_or_default(
            engine
                .ingest(ctx.session.agent_id, ctx.user_message, ctx.sender_user_id)
                .await
                .map(|r| r.recalled_memories),
            if ctx.streaming {
                "Context engine ingest failed (streaming); continuing without recalled memories"
            } else {
                "Context engine ingest failed; continuing without recalled memories"
            },
        )
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
                            5,
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
                            5,
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
                    5,
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
                            .auto_retrieve(&user_id, ctx.user_message, ctx.sender_user_id)
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
                        .auto_retrieve(&user_id, ctx.user_message, ctx.sender_user_id)
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
        let _ = kernel.auto_track_prompt_version(ctx.session.agent_id, &system_prompt);
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
