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
                        librefang_types::memory::EPISODIC_SCOPE,
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
                        librefang_types::memory::EPISODIC_SCOPE,
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
            librefang_types::memory::EPISODIC_SCOPE,
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

    // Both of these were previously flattened, and the class predicate the prompt memory section
    // budgets by rests on exactly the two of them (#7920).
    // `MemoryItem::from_fragment` folds every storage scope it does not recognise — `episodic`
    // included — into `MemoryLevel::Session`, so reconstructing the scope from `level` relabelled a
    // raw-dialogue row as `session_memory` and the section then filed it as an extracted fact and
    // gave it a fact-sized share of the budget.
    // `MemoryItem.scope` carries the real one; `level.scope_str()` remains the fallback for items
    // that never came from a stored fragment.
    let scope = item
        .scope
        .clone()
        .unwrap_or_else(|| item.level.scope_str().to_string());
    // Likewise the source: hard-coding `Conversation` made an imported or system-written episodic
    // row indistinguishable from one this agent's per-turn writer produced.
    let source = item
        .source
        .as_deref()
        .and_then(|s| {
            serde_json::from_value::<librefang_types::memory::MemorySource>(
                serde_json::Value::String(s.to_string()),
            )
            .ok()
        })
        .unwrap_or(librefang_types::memory::MemorySource::Conversation);

    MemoryFragment {
        id: memory_id,
        agent_id,
        content: item.content,
        embedding: None,
        metadata: item.metadata,
        source,
        confidence: 1.0,
        created_at: item.created_at,
        accessed_at: chrono::Utc::now(),
        access_count: 0,
        scope,
        image_url: None,
        image_embedding: None,
        modality: Default::default(),
        // Carried through so the recall's own ranking survives the conversion
        // (#7808). It is `None` unless the recall that produced this item was
        // embedding-ranked, which is exactly when a score exists.
        similarity: item.similarity,
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
    /// Identity of the session this turn belongs to, rendered as a UUID string, used for the #7605 cross-session memory filter.
    ///
    /// `Some` whenever session-scoped recall is in effect for this agent (`[proactive_memory] session_scoped_recall`, overridable per agent); `None` restores the pre-#7605 behaviour where every memory of an agent is a candidate for every one of that agent's turns.
    /// Resolved by `session_recall_scope` in `end_turn.rs` from the session the loop is actually reading and writing — there is no separate notion of a session here.
    pub(super) session_scope: Option<&'a str>,
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
    /// Operator override for the share of the prompt memory section's character budget reserved for extracted facts, as a percentage.
    ///
    /// Kernel populates this from `KernelConfig.memory_fact_budget_percent`; `None` uses `prompt_builder::MEMORY_FACT_BUDGET_PERCENT`.
    pub(super) memory_fact_budget_percent: Option<u8>,
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

/// Raw-dialogue candidates carried out of recall into the prompt memory section.
///
/// Unchanged from the historical class-blind window, and deliberately so: raw dialogue's 30 % share of the section budget renders about three rows, and even the whole budget renders about eleven, so a wider dialogue window is candidates the section could never print.
const MEMORY_RECALL_LIMIT_DIALOGUE: usize = 5;

/// Extracted-fact candidates carried out of recall into the prompt memory section.
///
/// Sized to what the fact share of the section budget can actually render — about 29 bullets at the 133-character mean measured in #7920 — so that the character budget, not the candidate window, is what decides how many facts appear.
/// Five was the old shared window, and a fact had to outrank raw dialogue nine times its size to claim one of those five slots.
const MEMORY_RECALL_LIMIT_FACTS: usize = 25;

/// Take the top-N of each memory class from one ranked list, preserving rank order within each.
///
/// The two classes are budgeted separately in the prompt (`prompt_builder::format_memory_items_by_class`), and that split is only as good as the candidates it is handed: a class-blind cap applied here decides the section's class mix before the budget ever gets a say.
///
/// No spill between the quotas. The section's own fill already spills an unused share from either class to the other, and a dialogue quota above `MEMORY_RECALL_LIMIT_DIALOGUE` could not be rendered even when facts are absent, so spilling here would only widen the "further remembered details are not shown" count.
fn select_recall_candidates(
    memories: Vec<MemoryFragment>,
    dialogue_limit: usize,
    fact_limit: usize,
) -> Vec<MemoryFragment> {
    let mut dialogue_kept = 0usize;
    let mut facts_kept = 0usize;
    memories
        .into_iter()
        .filter(|frag| {
            let (kept, limit) = if frag.is_raw_dialogue() {
                (&mut dialogue_kept, dialogue_limit)
            } else {
                (&mut facts_kept, fact_limit)
            };
            if *kept < limit {
                *kept += 1;
                true
            } else {
                false
            }
        })
        .collect()
}

pub(super) async fn setup_recalled_memories(ctx: RecallSetupContext<'_>) -> RecallSetup {
    // #5227: ask the substrate for a wider candidate window than the section will use, so the
    // per-scope post-filters below have enough headroom to keep a full set of legitimate results.
    // Without the inflation a substrate query that returns ~5 memories all stamped for the *other*
    // chat would leave zero results after filtering.
    // Matches the inflation factor used by `auto_retrieve`.
    //
    // #7920 widened it to every turn rather than only scope-filtered ones, and raised what it is a
    // multiple of. The window is now the candidate budget of both memory classes together, because
    // the selection below takes the top-N *per class*: on a store that is 4:1 raw dialogue, a
    // 5-row window contains a fact only by luck, and in 29 % of the measured turns it contained
    // none at all.
    const MEMORY_RECALL_LIMIT: usize = MEMORY_RECALL_LIMIT_DIALOGUE + MEMORY_RECALL_LIMIT_FACTS;
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
    let recall_fetch_limit = (MEMORY_RECALL_LIMIT * 4).max(50);
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
        // #7920 ungated this. `DefaultContextEngine::ingest` runs its own recall bounded by
        // `ContextEngineConfig::max_recall_results` (5 by default) and class-blind, and the kernel
        // always builds an engine (`boot.rs`: the binding is `Some(engine)` unconditionally), so on
        // a turn with no chat or session scope — dashboard, REST, cron — the engine's top-5 *was*
        // the section's entire candidate pool. Selecting per class out of five rows that are 4:1
        // raw dialogue cannot produce facts that are not there.
        if !ctx.stable_prefix_mode {
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
    // #7605: the same treatment for the session that owns this turn.
    // This path is the substrate/context-engine recall, which is a second way memories reach the prompt — gating only `auto_retrieve` below would leave one visitor's rows arriving here instead.
    if let Some(want) = ctx.session_scope {
        memories.retain(|frag| {
            librefang_types::memory::memory_session_scope_allows_recall(&frag.metadata, want)
        });
    }
    // Cap AFTER the scope filters, not before.
    // The fetch widened to `recall_fetch_limit = max(MEMORY_RECALL_LIMIT * 4, 50)` above
    // specifically so the filters have something to throw away; capping here is what restores the
    // prompt's expected candidate window.
    //
    // The cap is per class (#7920). A class-blind `truncate` handed the section whatever the top-N
    // happened to be, and on a store that is 4:1 raw dialogue the top-N is raw dialogue: in 29 % of
    // the measured turns not one extracted fact survived it, which no amount of budgeting
    // downstream can repair — the section cannot render a candidate it was never given.
    // Neither class can lose here: a class-blind top-`MEMORY_RECALL_LIMIT_DIALOGUE` could never
    // have contained more dialogue rows than the dialogue quota allows, so this selection returns
    // at least as many of each class as the old one did, and usually many more facts.
    memories = select_recall_candidates(
        memories,
        MEMORY_RECALL_LIMIT_DIALOGUE,
        MEMORY_RECALL_LIMIT_FACTS,
    );

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
                                ctx.session_scope,
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
                            ctx.session_scope,
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
        // Split by class before the section is built.
        // Recall ranks both classes in one list and that ranking is sound — raw dialogue takes slightly *under* its base rate of the slots — but a dialogue row inlines a whole exchange and outweighs an extracted fact by roughly nine to one in characters, so one shared budget hands the section to dialogue on size alone and leaves 29 % of turns with no fact in the prompt at all (#7920).
        // `partition` preserves rank order within each class, so the section stays a fixed arrangement of a fixed input (#3298).
        let (dialogue, facts): (Vec<&MemoryFragment>, Vec<&MemoryFragment>) =
            ctx.memories.iter().partition(|m| m.is_raw_dialogue());
        let to_pairs = |frags: &[&MemoryFragment]| -> Vec<(String, String)> {
            frags
                .iter()
                .map(|m| (String::new(), m.content.clone()))
                .collect()
        };
        let fact_pairs = to_pairs(&facts);
        let dialogue_pairs = to_pairs(&dialogue);
        if ctx.stable_prefix_mode {
            let personal_ctx = crate::prompt_builder::format_memory_items_by_class(
                &fact_pairs,
                &dialogue_pairs,
                ctx.memory_fact_budget_percent,
            );
            Some(personal_ctx)
        } else {
            let section = crate::prompt_builder::build_memory_section_by_class(
                &fact_pairs,
                &dialogue_pairs,
                ctx.memory_fact_budget_percent,
            );
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
            session_scope: None,
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

    /// #7605 — the substrate/context-engine recall path is a second way memories reach a prompt, alongside `auto_retrieve`.
    /// Gating only the latter would leave one visitor's rows arriving here instead, so this asserts the session filter applies to fragments recalled from the substrate too.
    #[tokio::test]
    async fn recall_setup_drops_other_sessions_memories_7605() {
        use librefang_types::memory::SESSION_SCOPE_METADATA_KEY;

        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let agent_id = AgentId::new();
        let session_a = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let session_b = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

        let write_for_session = |content: &str, session: Option<&str>| {
            let mut meta = std::collections::HashMap::new();
            if let Some(s) = session {
                meta.insert(
                    SESSION_SCOPE_METADATA_KEY.to_string(),
                    serde_json::Value::String(s.to_string()),
                );
            }
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

        write_for_session("project Atlas ships Friday", Some(session_a));
        write_for_session("project Atlas has a legacy note", None);

        let session = empty_session(agent_id);
        let opts = LoopOptions::default();
        let setup = setup_recalled_memories(RecallSetupContext {
            session: &session,
            user_message: "project Atlas",
            memory: substrate.as_ref(),
            embedding_driver: None,
            proactive_memory: None,
            context_engine: None,
            sender_user_id: None,
            sender_channel: None,
            sender_chat_scope: None,
            session_scope: Some(session_b),
            kernel: None,
            stable_prefix_mode: false,
            streaming: false,
            opts: &opts,
        })
        .await;

        assert!(
            !setup
                .memories
                .iter()
                .any(|f| f.content.contains("ships Friday")),
            "regression #7605: a memory written in session A reached session B's prompt: {:?}",
            setup
                .memories
                .iter()
                .map(|f| &f.content)
                .collect::<Vec<_>>()
        );
        assert!(
            setup
                .memories
                .iter()
                .any(|f| f.content.contains("legacy note")),
            "untagged rows must still surface, or upgrading would blank out an existing store"
        );
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

    /// Deterministic stand-in for a real embedding model.
    ///
    /// Two dimensions, chosen by a marker in the text, so cosine rank order is fixed by the fixture rather than by whatever a tokenizer happens to do: the query and every raw-dialogue row sit on the same axis (cosine 1.0), every fact sits off it (cosine ~0.9).
    /// Every dialogue row therefore outranks every fact, which is the shape #7920 measured — 29 % of turns whose top-ranked window held no fact at all.
    struct AxisEmbedding;

    #[async_trait::async_trait]
    impl crate::embedding::EmbeddingDriver for AxisEmbedding {
        async fn embed(
            &self,
            texts: &[&str],
        ) -> Result<Vec<Vec<f32>>, crate::embedding::EmbeddingError> {
            Ok(texts
                .iter()
                .map(|t| {
                    if t.contains("zzfact") {
                        vec![0.9, 0.436]
                    } else {
                        vec![1.0, 0.0]
                    }
                })
                .collect())
        }

        fn dimensions(&self) -> usize {
            2
        }
    }

    /// Write a corpus of the shape #7920 measured: raw dialogue outnumbering extracted facts about
    /// 4:1, and every dialogue row ranking above every fact.
    fn write_class_mixed_corpus(substrate: &MemorySubstrate, agent_id: AgentId) {
        for i in 0..40 {
            substrate
                .remember_with_embedding(
                    agent_id,
                    &format!("[Past exchange]\nThem: atlas {i}?\nYou: atlas shipped."),
                    MemorySource::Conversation,
                    librefang_types::memory::EPISODIC_SCOPE,
                    std::collections::HashMap::new(),
                    Some(&[1.0, 0.0]),
                    None,
                )
                .unwrap();
        }
        for i in 0..10 {
            let mut meta = std::collections::HashMap::new();
            meta.insert(
                librefang_types::memory::MEMORY_CATEGORY_METADATA_KEY.to_string(),
                serde_json::json!("preference"),
            );
            substrate
                .remember_with_embedding(
                    agent_id,
                    &format!("zzfact {i}: the user prefers atlas rollouts announced in advance."),
                    MemorySource::Conversation,
                    librefang_types::memory::MemoryLevel::User.scope_str(),
                    meta,
                    Some(&[0.9, 0.436]),
                    None,
                )
                .unwrap();
        }
    }

    /// #7920 — the section can only budget what recall hands it.
    ///
    /// Both recall producers rank the two memory classes in one list and cut it class-blind, so on
    /// a store that is 4:1 raw dialogue the candidates that reach the prompt are raw dialogue and
    /// the per-class budget downstream has no facts to place. This asserts the candidate set
    /// itself, not the rendered section, because that is where the loss happens.
    #[tokio::test]
    async fn recall_setup_carries_both_memory_classes_7920() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let agent_id = AgentId::new();
        write_class_mixed_corpus(&substrate, agent_id);

        let session = empty_session(agent_id);
        let opts = LoopOptions::default();
        let embedding = AxisEmbedding;
        let setup = setup_recalled_memories(RecallSetupContext {
            session: &session,
            user_message: "atlas",
            memory: substrate.as_ref(),
            embedding_driver: Some(&embedding),
            proactive_memory: None,
            context_engine: None,
            sender_user_id: None,
            sender_channel: None,
            sender_chat_scope: None,
            session_scope: None,
            kernel: None,
            stable_prefix_mode: false,
            streaming: false,
            opts: &opts,
        })
        .await;

        let facts = setup
            .memories
            .iter()
            .filter(|f| !f.is_raw_dialogue())
            .count();
        let dialogue = setup
            .memories
            .iter()
            .filter(|f| f.is_raw_dialogue())
            .count();
        assert_eq!(
            facts, 10,
            "every extracted fact in the store must reach the section: got {facts} facts, {dialogue} dialogue rows"
        );
        assert_eq!(
            dialogue, MEMORY_RECALL_LIMIT_DIALOGUE,
            "raw dialogue must still fill its own quota, not be excluded"
        );
    }

    /// The same, on the branch production actually takes.
    ///
    /// The kernel always builds a context engine (`boot.rs` binds it unconditionally as `Some`), so
    /// the engine branch — whose `ingest` runs its own class-blind recall bounded by
    /// `max_recall_results` — is the live path. Before #7920 the supplemental substrate fetch that
    /// widens that window ran only when a chat or session scope was active, so a dashboard / REST /
    /// cron turn handed the section the engine's top five rows and nothing else.
    #[tokio::test]
    async fn recall_setup_carries_both_classes_through_the_context_engine_7920() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let agent_id = AgentId::new();
        write_class_mixed_corpus(&substrate, agent_id);

        let engine_cfg = ContextEngineConfig {
            max_recall_results: 5,
            ..Default::default()
        };
        let engine = DefaultContextEngine::new(
            engine_cfg,
            Arc::clone(&substrate),
            Some(Arc::new(AxisEmbedding)),
        );

        let session = empty_session(agent_id);
        let opts = LoopOptions::default();
        let embedding = AxisEmbedding;
        let setup = setup_recalled_memories(RecallSetupContext {
            session: &session,
            user_message: "atlas",
            memory: substrate.as_ref(),
            embedding_driver: Some(&embedding),
            proactive_memory: None,
            context_engine: Some(&engine),
            sender_user_id: None,
            sender_channel: None,
            sender_chat_scope: None,
            session_scope: None,
            kernel: None,
            stable_prefix_mode: false,
            streaming: false,
            opts: &opts,
        })
        .await;

        let facts = setup
            .memories
            .iter()
            .filter(|f| !f.is_raw_dialogue())
            .count();
        assert!(
            facts > 0,
            "no extracted fact survived the engine path's candidate selection; \
             candidates = {:?}",
            setup
                .memories
                .iter()
                .map(|f| (
                    f.scope.clone(),
                    f.content.chars().take(24).collect::<String>()
                ))
                .collect::<Vec<_>>()
        );
    }

    /// A quota is a ceiling on its own class, never a floor taken from the other.
    #[test]
    fn recall_candidate_selection_caps_each_class_independently() {
        let agent_id = AgentId::new();
        let dialogue = |i: usize| MemoryFragment {
            id: librefang_types::memory::MemoryId(uuid::Uuid::new_v4()),
            agent_id,
            content: format!("[Past exchange] {i}"),
            embedding: None,
            metadata: std::collections::HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: librefang_types::memory::EPISODIC_SCOPE.to_string(),
            image_url: None,
            image_embedding: None,
            modality: Default::default(),
            similarity: None,
        };
        let fact = |i: usize| MemoryFragment {
            scope: librefang_types::memory::MemoryLevel::User
                .scope_str()
                .to_string(),
            content: format!("fact {i}"),
            ..dialogue(i)
        };

        // Dialogue first, the ranking that starved facts in production.
        let mixed: Vec<MemoryFragment> = (0..20).map(dialogue).chain((0..20).map(fact)).collect();
        let kept = select_recall_candidates(mixed, 5, 25);
        assert_eq!(kept.iter().filter(|f| f.is_raw_dialogue()).count(), 5);
        assert_eq!(kept.iter().filter(|f| !f.is_raw_dialogue()).count(), 20);
        // Rank order is preserved inside each class.
        assert!(kept[0].content.starts_with("[Past exchange] 0"));

        // A class that is absent costs the other class nothing, and takes nothing from it.
        let facts_only: Vec<MemoryFragment> = (0..40).map(fact).collect();
        assert_eq!(select_recall_candidates(facts_only, 5, 25).len(), 25);
        let dialogue_only: Vec<MemoryFragment> = (0..40).map(dialogue).collect();
        assert_eq!(select_recall_candidates(dialogue_only, 5, 25).len(), 5);
    }

    /// A raw-dialogue row that reaches the prompt through proactive memory must still read as raw
    /// dialogue.
    ///
    /// `MemoryItem::from_fragment` maps the storage scope through `MemoryLevel`, whose catch-all arm
    /// folds `episodic` into `Session`, so before #7920 the round trip relabelled dialogue as
    /// `session_memory` — and the section's class split, which reads the scope, handed it a
    /// fact-sized share of the budget.
    #[test]
    fn proactive_conversion_preserves_the_memory_class() {
        let agent_id = AgentId::new();
        let frag = MemoryFragment {
            id: librefang_types::memory::MemoryId(uuid::Uuid::new_v4()),
            agent_id,
            content: "[Past exchange]\nThem: hi\nYou: hello".to_string(),
            embedding: None,
            metadata: std::collections::HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: librefang_types::memory::EPISODIC_SCOPE.to_string(),
            image_url: None,
            image_embedding: None,
            modality: Default::default(),
            similarity: None,
        };
        assert!(frag.is_raw_dialogue(), "fixture is not raw dialogue");

        let item = librefang_types::memory::MemoryItem::from_fragment(frag);
        assert_eq!(
            item.level,
            librefang_types::memory::MemoryLevel::Session,
            "the lossy fold this guards against is gone; revisit the conversion"
        );
        let round_tripped = proactive_item_to_fragment(item, agent_id);
        assert!(
            round_tripped.is_raw_dialogue(),
            "proactive round trip relabelled raw dialogue as an extracted fact: scope={}",
            round_tripped.scope
        );
    }
}
