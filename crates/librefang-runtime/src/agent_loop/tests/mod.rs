use super::history::MIN_HISTORY_MESSAGES;
use super::message::{sanitize_for_memory, ACCUMULATED_TEXT_MAX_BYTES};
use super::model::needs_qualified_model_id;
use super::retry::{BASE_RETRY_DELAY_MS, MAX_RETRIES};
use super::text_recovery::{
    looks_like_hallucinated_action, parse_dash_dash_args, parse_json_tool_call_object,
    user_message_has_action_intent,
};
use super::tool_call::{finalize_tool_use_results, record_tool_call_metric, StagedToolUseTurn};
use super::tool_resolution::{resolve_request_tools, LAZY_TOOLS_THRESHOLD};
use super::web_augment::{should_augment_web_search, SEARCH_QUERY_GEN_PROMPT};
use super::*;
use crate::llm_driver::{CompletionResponse, LlmError};
use crate::silent_response::{ENVELOPE_LINE_PREFIXES, ENVELOPE_STANDALONE_MARKERS};
use async_trait::async_trait;
use librefang_memory::session::SessionStore;
use librefang_types::tool::ToolCall;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn test_max_iterations_constant() {
    assert_eq!(
        MAX_ITERATIONS,
        librefang_types::agent::AutonomousConfig::DEFAULT_MAX_ITERATIONS
    );
}

// ── push_accumulated_text bounded growth ──────────────────────────────

#[test]
fn test_push_accumulated_text_appends_with_separator() {
    let mut buf = String::new();
    push_accumulated_text(&mut buf, "first");
    assert_eq!(buf, "first");

    push_accumulated_text(&mut buf, "second");
    assert_eq!(buf, "first\n\nsecond");
}

#[test]
fn test_push_accumulated_text_caps_at_max_bytes() {
    let mut buf = String::new();
    // First push: well within cap
    let small = "a".repeat(1024);
    push_accumulated_text(&mut buf, &small);
    assert_eq!(buf.len(), 1024);

    // Second push: would exceed the cap → buffer is sealed at exactly the cap
    let huge = "b".repeat(ACCUMULATED_TEXT_MAX_BYTES);
    push_accumulated_text(&mut buf, &huge);
    assert_eq!(
        buf.len(),
        ACCUMULATED_TEXT_MAX_BYTES,
        "buffer must be sealed exactly at the cap (no overflow)"
    );
    // The original 'a' prefix must be preserved — that's the whole point
    // of the "preserve buffered prefix" guarantee.
    assert!(buf.starts_with(&small));

    // Third push: short-circuits, no growth, no panic
    push_accumulated_text(&mut buf, "ignored");
    assert_eq!(buf.len(), ACCUMULATED_TEXT_MAX_BYTES);
    assert!(!buf.contains("ignored"));
}

#[test]
fn test_push_accumulated_text_under_cap_unchanged() {
    // Sanity: many small pushes under the cap accumulate normally.
    let mut buf = String::new();
    for i in 0..100 {
        push_accumulated_text(&mut buf, &format!("turn {i}"));
    }
    assert!(buf.len() < ACCUMULATED_TEXT_MAX_BYTES);
    assert!(buf.starts_with("turn 0"));
    assert!(buf.contains("turn 99"));
}

#[test]
fn test_push_accumulated_text_empty_initial_no_separator() {
    // First-push must not start with the "\n\n" separator.
    let mut buf = String::new();
    push_accumulated_text(&mut buf, "hello");
    assert_eq!(buf, "hello");
    assert!(!buf.starts_with("\n\n"));
}

/// Resolve the iteration cap the same way `run_agent_loop` does: per-agent
/// manifest > operator LoopOptions > library default.
fn resolve_max_iterations(manifest_cap: Option<u32>, opts_cap: Option<u32>) -> u32 {
    manifest_cap.or(opts_cap).unwrap_or(MAX_ITERATIONS)
}

#[test]
fn max_iterations_resolution_prefers_manifest_over_opts() {
    assert_eq!(resolve_max_iterations(Some(7), Some(100)), 7);
}

#[test]
fn max_iterations_resolution_falls_back_to_opts() {
    assert_eq!(resolve_max_iterations(None, Some(100)), 100);
}

#[test]
fn max_iterations_resolution_falls_back_to_default_when_nothing_set() {
    assert_eq!(
        resolve_max_iterations(None, None),
        librefang_types::agent::AutonomousConfig::DEFAULT_MAX_ITERATIONS
    );
}

// --- finalize_end_turn_text fallback tests ------------------------------
//
// The helper is the single funnel for empty-response handling on both
// sync and streaming paths. These tests pin the three-way contract:
//   1. Final text non-empty → use it (accumulated buffer ignored).
//   2. Final text empty + accumulated non-empty → use accumulated buffer.
//   3. Final text empty + accumulated empty → emit canned guard message.

#[test]
fn finalize_end_turn_text_uses_final_text_when_present() {
    let usage = TokenUsage::default();
    let out = finalize_end_turn_text(
        "final answer".to_string(),
        true,
        "agent",
        3,
        &usage,
        5,
        "log msg",
        "leftover from earlier turn",
    );
    // Final text wins — accumulated buffer must NOT leak into output.
    assert_eq!(out, "final answer");
}

#[test]
fn finalize_end_turn_text_falls_back_to_accumulated_when_final_empty() {
    let usage = TokenUsage::default();
    let out = finalize_end_turn_text(
        "   ".to_string(), // whitespace-only counts as empty
        true,
        "agent",
        3,
        &usage,
        5,
        "log msg",
        "I looked that up for you.",
    );
    assert_eq!(out, "I looked that up for you.");
}

#[test]
fn finalize_end_turn_text_emits_guard_when_both_empty_with_tools() {
    let usage = TokenUsage::default();
    let out = finalize_end_turn_text(String::new(), true, "agent", 3, &usage, 5, "log msg", "");
    assert!(
        out.contains("Task completed"),
        "expected tools-executed guard message, got: {out}"
    );
}

#[test]
fn finalize_end_turn_text_emits_guard_when_both_empty_no_tools() {
    let usage = TokenUsage::default();
    let out = finalize_end_turn_text(String::new(), false, "agent", 0, &usage, 1, "log msg", "");
    assert!(
        out.contains("empty response"),
        "expected no-tools guard message, got: {out}"
    );
}

fn fake_tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("fake {name}"),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

#[test]
fn test_resolve_request_tools_falls_back_to_eager_when_tool_load_missing() {
    // Regression for PR #3047 codex review P1: if an agent's allowlist
    // is over the threshold but does NOT include `tool_load`, we must
    // return the full eager list. Otherwise non-native tools get
    // stripped with no recovery path and silently disappear.
    let mut pool: Vec<ToolDefinition> = (0..LAZY_TOOLS_THRESHOLD + 5)
        .map(|i| fake_tool(&format!("tool_{i}")))
        .collect();
    // Sanity: tool_load is definitely not in the list.
    assert!(!pool.iter().any(|t| t.name == "tool_load"));
    let resolved = resolve_request_tools(&pool, &[], true);
    assert_eq!(
        resolved.len(),
        pool.len(),
        "lazy mode must bypass when tool_load is absent — got trimmed list"
    );

    // And with tool_load in the pool, lazy mode kicks in (as designed).
    pool.push(fake_tool("tool_load"));
    let resolved = resolve_request_tools(&pool, &[], true);
    assert!(
        resolved.len() < pool.len(),
        "lazy mode should trim when tool_load is present"
    );
}

#[test]
fn test_resolved_tools_cache_reuses_arc_when_input_is_stable() {
    // The whole point of #3586 is that an idle iteration (no new tools
    // loaded via `tool_load`) MUST hand back the same `Arc` rather than
    // rebuild the resolved tool list. Pin that with `Arc::ptr_eq` so a
    // future regression that reverts the cache to a no-op fails here
    // instead of silently in a profiler.
    let pool: Vec<ToolDefinition> = (0..LAZY_TOOLS_THRESHOLD + 5)
        .map(|i| fake_tool(&format!("tool_{i}")))
        .chain(std::iter::once(fake_tool("tool_load")))
        .collect();

    let mut cache = ResolvedToolsCache::new(&pool, &[], true);
    let a = cache.get(&pool, &[]);
    let b = cache.get(&pool, &[]);
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "stable input must reuse the cached Arc"
    );
}

#[test]
fn test_resolved_tools_cache_rebuilds_when_session_loaded_grows() {
    // Lazy mode + a new tool_load redemption mid-turn: the cache must
    // rebuild so the LLM sees the just-loaded tool on the next turn.
    let pool: Vec<ToolDefinition> = (0..LAZY_TOOLS_THRESHOLD + 5)
        .map(|i| fake_tool(&format!("tool_{i}")))
        .chain(std::iter::once(fake_tool("tool_load")))
        .collect();
    let mut session_loaded: Vec<ToolDefinition> = Vec::new();

    let mut cache = ResolvedToolsCache::new(&pool, &session_loaded, true);
    let before = cache.get(&pool, &session_loaded);

    session_loaded.push(fake_tool("late_arrival"));
    let after = cache.get(&pool, &session_loaded);

    assert!(
        !std::sync::Arc::ptr_eq(&before, &after),
        "growing session_loaded_tools must rebuild the cache"
    );
    assert!(
        after.iter().any(|t| t.name == "late_arrival"),
        "rebuilt cache must include the newly loaded tool"
    );
}

#[test]
fn test_resolved_tools_cache_no_rebuild_when_lazy_mode_off() {
    // In non-lazy mode `resolve_request_tools` ignores `session_loaded`,
    // so the cache should never rebuild — even if the (unused) loaded
    // vec grows. Guards against an over-eager invalidation that would
    // re-clone the full eager list every iteration.
    let pool: Vec<ToolDefinition> = (0..3).map(|i| fake_tool(&format!("t{i}"))).collect();
    let mut session_loaded: Vec<ToolDefinition> = Vec::new();

    let mut cache = ResolvedToolsCache::new(&pool, &session_loaded, false);
    let before = cache.get(&pool, &session_loaded);

    session_loaded.push(fake_tool("ignored"));
    let after = cache.get(&pool, &session_loaded);

    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "non-lazy mode must never rebuild on session_loaded growth"
    );
}

#[test]
fn test_is_no_reply() {
    // Canonical token
    assert!(is_no_reply("NO_REPLY"));
    assert!(is_no_reply("  NO_REPLY  "));
    assert!(is_no_reply("Let me think.\nNO_REPLY"));
    assert!(is_no_reply("I'll stay quiet. NO_REPLY"));

    // Bracketed placeholder (synthetic marker written back into sessions)
    assert!(is_no_reply("[no reply needed]"));
    assert!(is_no_reply("Some context. [no reply needed]"));

    // Unbracketed variant — exact match only (ends_with dropped to avoid prose false-positives)
    assert!(is_no_reply("no reply needed"));

    // Negatives — real responses must never be silenced
    assert!(!is_no_reply(""));
    assert!(!is_no_reply("Just replying normally."));
    assert!(!is_no_reply("NO_REPLY is my favorite token")); // prefix, not suffix
    assert!(!is_no_reply("no reply needed? let me check")); // doesn't end with marker
    assert!(!is_no_reply("I filed the bug; no reply needed")); // prose ending — not a sentinel
    assert!(!is_no_reply("context here\nno reply needed")); // multi-line prose ending
}

#[test]
fn test_is_progress_text_leak() {
    // Real production leak — ellipsis-terminated preamble with no tool_use
    assert!(is_progress_text_leak(
        "Waiting for the script to complete..."
    ));
    assert!(is_progress_text_leak("Let me check that..."));
    assert!(is_progress_text_leak("Processing..."));
    assert!(is_progress_text_leak("One moment…"));
    assert!(is_progress_text_leak("   Checking...   ")); // whitespace

    // Negatives — real replies must never be flagged as leaks
    assert!(!is_progress_text_leak(""));
    assert!(!is_progress_text_leak("Done."));
    assert!(!is_progress_text_leak("Here is the result."));
    // Two-dot `..` is intentionally not a trigger (too broad, catches
    // truncated abbreviations). See the `is_progress_text_leak` doc.
    assert!(!is_progress_text_leak("Running.."));
    assert!(!is_progress_text_leak("See p.."));
    // Not an ellipsis, real reply
    assert!(!is_progress_text_leak("The script ran successfully."));
    // Over 120 chars — even ending with ellipsis, treat as real content
    let long =
        "This is a much longer response where the model actually produced a full explanation of what it did and the ellipsis at the end is just stylistic...";
    assert!(long.chars().count() > 120);
    assert!(!is_progress_text_leak(long));
}

#[test]
fn sanitize_for_memory_strips_known_envelopes() {
    assert_eq!(
        sanitize_for_memory("[Group message from Alice]\n[In risposta a: \"hi\"]\nciao tutti")
            .as_deref(),
        Some("ciao tutti"),
    );
}

#[test]
fn sanitize_for_memory_strips_stranger_and_forwarded() {
    assert_eq!(
        sanitize_for_memory("[Stranger from +393331234567]\n[Forwarded]\nhey there").as_deref(),
        Some("hey there"),
    );
    assert_eq!(
        sanitize_for_memory("[Stranger]\nplain inbound").as_deref(),
        Some("plain inbound"),
    );
    assert_eq!(sanitize_for_memory("[User]\nfoo").as_deref(), Some("foo"),);
}

#[test]
fn sanitize_for_memory_preserves_inline_brackets_and_clean_input() {
    // Square brackets that don't start a line as an envelope prefix
    // must be preserved — they are legitimate user content.
    assert_eq!(
        sanitize_for_memory("[Alice]: ciao [meet at 5pm]").as_deref(),
        Some("[Alice]: ciao [meet at 5pm]"),
    );
    assert_eq!(
        sanitize_for_memory("plain message").as_deref(),
        Some("plain message"),
    );
    // Empty input collapses to None so the caller skips persistence.
    assert_eq!(sanitize_for_memory(""), None);
    // English variant of the WhatsApp reply marker.
    assert_eq!(
        sanitize_for_memory("[Replying to: \"hi\"]\nhello").as_deref(),
        Some("hello"),
    );
}

#[test]
fn sanitize_for_memory_tolerates_leading_whitespace() {
    // Some clients forward with leading whitespace before the envelope.
    assert_eq!(
        sanitize_for_memory("  [Group message from Alice]\nhello").as_deref(),
        Some("hello"),
    );
    assert_eq!(
        sanitize_for_memory("\t[Forwarded]\nbody").as_deref(),
        Some("body"),
    );
}

#[test]
fn sanitize_for_memory_envelope_only_input_returns_none() {
    // No body after the envelope — refuse to persist a half-empty
    // memory row that would itself trip the cascade-leak guard.
    assert_eq!(sanitize_for_memory("[Forwarded]\n"), None);
    assert_eq!(sanitize_for_memory("[Stranger from +393331234567]\n"), None);
    assert_eq!(sanitize_for_memory("[Group message from Alice]"), None);
}

#[test]
fn sanitize_for_memory_accepts_quotereply_without_space_after_colon() {
    // Some JS template literals emit `[In risposta a:"hi"]` (no space
    // after colon). The sanitiser must strip the same prefix the leak
    // guard sees so legacy memories don't keep tripping the guard.
    assert_eq!(
        sanitize_for_memory("[In risposta a:\"hi\"]\nbody").as_deref(),
        Some("body"),
    );
}

#[test]
fn sanitize_for_memory_preserves_body_when_marker_is_inline_not_standalone() {
    // "[User] follow-up question" is NOT a standalone marker — a
    // hypothetical adapter could emit this shape; body must stay.
    assert_eq!(
        sanitize_for_memory("[User] follow-up question").as_deref(),
        Some("[User] follow-up question"),
    );
}

#[test]
fn envelope_prefixes_are_a_subset_of_cascade_structural_markers() {
    // Invariant: every envelope the sanitiser strips must also be
    // detectable as a structural marker by is_cascade_leak. Otherwise
    // a legacy memory row containing that envelope would keep tripping
    // the leak guard without ever being repaired by the sanitiser.
    for prefix in ENVELOPE_LINE_PREFIXES {
        let probe = format!("{prefix}X]\nUser asked: foo");
        assert!(
            is_cascade_leak(&probe),
            "prefix {prefix:?} not detected by is_cascade_leak",
        );
    }
    for marker in ENVELOPE_STANDALONE_MARKERS {
        // Standalone marker + a thematic header alone is not enough
        // (thematic-only doesn't trip); pair with a turn frame so the
        // 2-structural threshold trips deterministically.
        let probe = format!("{marker}\nUser asked: foo");
        assert!(
            is_cascade_leak(&probe),
            "standalone marker {marker:?} not detected by is_cascade_leak",
        );
    }
}

#[test]
fn is_cascade_leak_trips_on_two_or_more_markers() {
    // Two structural envelopes co-occurring.
    assert!(is_cascade_leak(
        "[Group message from X]\n[In risposta a: \"y\"]\ntext"
    ));
    // Two turn frames.
    assert!(is_cascade_leak("User asked: foo\nI responded: bar"));
    // 1 structural + 1 thematic.
    assert!(is_cascade_leak("## Calendar\n[Group message from X]\nbar"));
    // Real-world incident shape — envelope + turn frame.
    let real_incident = "[User]\n[Group message from ALESSANDRO Liva]\nGrande Ambrogio\nUser asked: foo\nI responded: bar";
    assert!(is_cascade_leak(real_incident));
}

#[test]
fn thematic_headers_alone_are_legitimate() {
    // Two-or-more THEMATIC headers without any structural marker is
    // a legitimate help reply (e.g. "what does my day look like"
    // → calendar + tasks summary). This was a houko-flagged false
    // positive in the original any-2-marker design.
    assert!(!is_cascade_leak(
        "## Calendar\n- meeting at 5pm\n\n## Tasks\n- send follow-up",
    ));
    assert!(!is_cascade_leak(
        "## Today\nWednesday\n## Calendar\nno events\n## Tasks\npending",
    ));
}

#[test]
fn is_cascade_leak_does_not_trip_on_single_marker_or_clean_text() {
    // One legitimate self-reference is not a cascade.
    assert!(!is_cascade_leak(
        "The phrase 'User asked:' is from training data."
    ));
    assert!(!is_cascade_leak("normal reply with no markers"));
    assert!(!is_cascade_leak(""));
    // Single quote-reply envelope mentioned in a reply (rare but valid).
    assert!(!is_cascade_leak(
        "I noticed you wrote `[In risposta a: ...]` in your message."
    ));
    // Single thematic header is fine.
    assert!(!is_cascade_leak("## Calendar\n- meeting at 5pm"));
}

#[test]
fn hallucinated_action_detects_english_dev_claims() {
    // Regression: original EN dev/file claims must keep firing.
    assert!(looks_like_hallucinated_action(
        "I've created the file in src/utils.rs"
    ));
    assert!(looks_like_hallucinated_action(
        "I have updated the configuration."
    ));
    assert!(looks_like_hallucinated_action(
        "The file has been written successfully."
    ));
    assert!(looks_like_hallucinated_action(
        "Successfully modified the schema."
    ));
}

#[test]
fn hallucinated_action_detects_english_transactional_claims() {
    // Domain-action claims that previously slipped through (channel send,
    // YNAB record, calendar booking, etc.).
    assert!(looks_like_hallucinated_action(
        "I've sent the message to your contact."
    ));
    assert!(looks_like_hallucinated_action(
        "I've scheduled the appointment for tomorrow."
    ));
    assert!(looks_like_hallucinated_action("I've booked the flight."));
    assert!(looks_like_hallucinated_action(
        "I've registered the transaction in YNAB."
    ));
    assert!(looks_like_hallucinated_action(
        "I've transferred €100 to your savings account."
    ));
    assert!(looks_like_hallucinated_action("Order has been placed."));
    assert!(looks_like_hallucinated_action(
        "Message has been sent successfully."
    ));
}

#[test]
fn hallucinated_action_detects_italian_present_perfect_claims() {
    // Italian "ho + past participle" — the form Ambrogio falls into when
    // it lies about completing a domain operation.
    assert!(looks_like_hallucinated_action(
        "Ho registrato la spesa di 12 euro al supermercato."
    ));
    assert!(looks_like_hallucinated_action(
        "Ho inviato il messaggio a Jessica come richiesto."
    ));
    assert!(looks_like_hallucinated_action(
        "Ho allegato il PDF alla mail."
    ));
    assert!(looks_like_hallucinated_action(
        "Ho prenotato il ristorante per le 20:00."
    ));
    assert!(looks_like_hallucinated_action(
        "Ho schedulato il bonifico per domani."
    ));
    assert!(looks_like_hallucinated_action(
        "Ho bonificato 500 euro sul conto risparmio."
    ));
    assert!(looks_like_hallucinated_action(
        "Ho aggiornato la nota sul calendario."
    ));
}

#[test]
fn hallucinated_action_detects_italian_impersonal_claims() {
    assert!(looks_like_hallucinated_action(
        "Il messaggio è stato inviato al destinatario."
    ));
    assert!(looks_like_hallucinated_action(
        "La transazione è stata registrata correttamente."
    ));
    assert!(looks_like_hallucinated_action(
        "L'appuntamento è stato programmato."
    ));
    assert!(looks_like_hallucinated_action("Messaggio inviato."));
    assert!(looks_like_hallucinated_action("Operazione completata."));
    assert!(looks_like_hallucinated_action(
        "Bonifico effettuato con successo."
    ));
}

#[test]
fn hallucinated_action_does_not_fire_on_neutral_text() {
    // Plain replies must never trigger a corrective retry — a false
    // positive burns one in-loop iteration.
    assert!(!looks_like_hallucinated_action(""));
    assert!(!looks_like_hallucinated_action("Hello, how can I help?"));
    assert!(!looks_like_hallucinated_action(
        "Vuoi che registri questa spesa? Confermami pure."
    ));
    assert!(!looks_like_hallucinated_action(
        "Posso inviare il messaggio se mi confermi il numero."
    ));
    // Bare "fatto" intentionally NOT in the trigger list — too noisy
    // ("non ho fatto in tempo a chiamarti" should not retry).
    assert!(!looks_like_hallucinated_action(
        "Non ho fatto in tempo a chiamarti."
    ));
}

#[test]
fn test_retry_constants() {
    assert_eq!(MAX_RETRIES, 3);
    assert_eq!(BASE_RETRY_DELAY_MS, 1000);
}

/// Invariant: when the silent flag is set on an AgentLoopResult, the
/// response field MUST be empty. No sentinel string ever escapes the
/// runtime as visible text. The shared constructor enforces this.
#[test]
fn silent_result_has_empty_response() {
    let result = build_silent_agent_loop_result(
        TokenUsage::default(),
        1,
        crate::reply_directives::DirectiveSet::default(),
        Vec::new(),
        Vec::new(),
        None,
        0,
    );
    assert!(result.silent);
    assert_eq!(
        result.response, "",
        "silent=true must imply response==\"\" (no sentinel leaks as text)"
    );
}

/// Grep-guard: enforce that `silent_response.rs` is the SOLE owner of
/// the literal `NO_REPLY` token in `crates/`. Any new occurrence outside
/// the allow-list must be either delegated to the canonical detector or
/// (if it is documentation / a prompt-injection sentinel comment) added
/// to the allow-list with rationale.
///
/// Allow-list rationale:
/// - silent_response.rs — canonical detector + tests
/// - agent_loop.rs — kept for the heartbeat back-write
///   ("[no reply needed]") and tests
/// - session_repair.rs — heartbeat-prune predicate (delegates)
/// - reply_directives.rs — back-compat parse-through test
/// - prompt_builder.rs — explanatory prompt text (post-rewrite
///   references the token internally)
/// - drivers/claude_code.rs — driver-side suppression (delegates)
#[test]
fn silent_response_single_source_of_truth() {
    use std::process::Command;
    let crates_dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.parent().map(|q| q.to_path_buf()));
    let Some(crates_dir) = crates_dir else {
        eprintln!("skipping grep-guard: cannot locate crates/");
        return;
    };
    let output = Command::new("grep")
        .args(["-rln", "--include=*.rs", "NO_REPLY"])
        .arg(&crates_dir)
        .output();
    let Ok(output) = output else {
        eprintln!("skipping grep-guard: grep unavailable");
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let allow = [
        "silent_response.rs",      // canonical detector + tests
        "agent_loop.rs",           // heartbeat back-write [no reply needed]
        "session_repair.rs",       // delegates to canonical detector
        "reply_directives.rs",     // back-compat parse-through test
        "prompt_builder.rs",       // post-rewrite prompt mentions internal token
        "claude_code.rs",          // driver-side stream suppression (cycle barrier)
        "agent.rs",                // librefang-types: doc comment only
        "channel_bridge.rs",       // librefang-api: doc comment, consumes silent flag
        "agents.rs",               // librefang-api routes: doc comment only
        "ws.rs",                   // librefang-api ws: doc comment only
        "purge_sentinels.rs", // CLI binary that *removes* the literal — delegates to canonical detector
        "purge_sentinels_test.rs", // fixtures for the CLI
        "lib.rs",             // librefang-types: legacy is_no_reply_sentinel compat shim
        "mod.rs",             // librefang-kernel: inline comment only
        "cron_tick.rs", // librefang-kernel #4713 phase 3: split out of kernel/mod.rs, comment only
        // #3710 god-file split: the literal moved out of `agent_loop.rs`
        // and `session_repair.rs` into new submodule siblings. Each
        // entry is a path suffix (matched via `ends_with`) so no
        // unrelated `types.rs` / `message.rs` is silently exempted by
        // a bare-filename match. (`agent_loop/mod.rs` and the new
        // `agent_loop/tests/mod.rs` are already covered by the `mod.rs`
        // entry above.)
        "agent_loop/types.rs", // post-split: AgentLoopResult shape + small helpers
        "agent_loop/message.rs", // post-split: assistant-message construction helpers
        "session_repair/tests.rs", // session_repair tests moved into module subdir
    ];
    let offenders: Vec<&str> = stdout
        .lines()
        .filter(|line| !allow.iter().any(|a| line.ends_with(a)))
        .collect();
    assert!(
        offenders.is_empty(),
        "NO_REPLY literal found outside allow-list — delegate to silent_response::is_silent_response: {offenders:?}"
    );
}

mod integration;
mod recovery;
mod sender;
mod utilities;
