//! Regression test for audit issue
//! `docs/issues/cron-channel-name-not-reserved.md`.
//!
//! `SYSTEM_CHANNEL_*` constants in `crates/librefang-kernel/src/kernel/mod.rs`
//! are named but historically were not enforced at every ingress. A custom
//! channel adapter or HTTP caller could pass `channel = "cron"`
//! (case-insensitive — `SessionId::for_channel` lowercases internally) and
//! derive the SAME `SessionId` as the kernel-internal cron-fire path. Result:
//! two independent write streams interleaving into one persistent history.
//!
//! Fix (option 1, defense-in-depth ingress validation): every external
//! `SenderContext` construction site now funnels through
//! `librefang_channels::types::sanitize_channel_name`, which renames any
//! reserved name to `ext-<name>` BEFORE it can reach `SessionId::for_channel`.
//! The kernel's `send_message_full` channel-derived branch re-applies the
//! sanitizer as defense-in-depth: even if a future construction site is
//! added that forgets the helper, the kernel ingress will still rewrite the
//! collision unless the trusted `is_internal_system` flag is set. That flag is
//! set by the kernel's own system constructors (cron tick, autonomous
//! background tick, web UI) so their reserved channel names derive the legacy
//! `for_channel(agent, "<name>")` SessionId and existing persistent history
//! stays continuous. (`is_internal_cron` is deliberately NOT reused for this:
//! it is cron-only because it also gates `[SILENT]` marker stripping, so the
//! autonomous internal path — a reserved channel without `is_internal_cron` —
//! would otherwise be wrongly rewritten to `ext-autonomous`.)
//!
//! This file pins the contract at the public types-crate boundary so the
//! fix cannot silently regress.

use librefang_channels::types::{
    is_reserved_system_channel, sanitize_channel_name, RESERVED_SYSTEM_CHANNEL_NAMES,
};
use librefang_types::agent::{AgentId, SessionId};

#[test]
fn external_cron_channel_does_not_collide_with_internal_cron_session() {
    let agent = AgentId::new();
    // Kernel-internal cron dispatcher uses the raw "cron" literal —
    // see `crates/librefang-kernel/src/kernel/cron_tick.rs:217`. The
    // persistent session id is therefore stable as `for_channel(agent, "cron")`.
    let internal_cron = SessionId::for_channel(agent, "cron");

    // Every variant a hostile / misconfigured external caller can try.
    // The sanitizer at the SenderContext construction site MUST rewrite
    // them so the derived SessionId is disjoint from `internal_cron`.
    for variant in ["cron", "CRON", "Cron", "  cron  ", "CRON\t"] {
        let sanitized = sanitize_channel_name(variant);
        let external = SessionId::for_channel(agent, &sanitized);
        assert_ne!(
            external, internal_cron,
            "external channel {variant:?} sanitized to {sanitized:?} must NOT \
             derive the internal cron SessionId (audit: cron-channel-name-not-reserved)"
        );
    }
}

#[test]
fn external_autonomous_and_webui_channels_also_disjoint() {
    let agent = AgentId::new();
    let internal_autonomous = SessionId::for_channel(agent, "autonomous");
    let internal_webui = SessionId::for_channel(agent, "webui");

    for variant in ["autonomous", "Autonomous", "AUTONOMOUS"] {
        let sanitized = sanitize_channel_name(variant);
        let external = SessionId::for_channel(agent, &sanitized);
        assert_ne!(
            external, internal_autonomous,
            "external {variant:?} must not derive internal autonomous session"
        );
    }
    for variant in ["webui", "WebUI", "WEBUI"] {
        let sanitized = sanitize_channel_name(variant);
        let external = SessionId::for_channel(agent, &sanitized);
        assert_ne!(
            external, internal_webui,
            "external {variant:?} must not derive internal webui session"
        );
    }
}

#[test]
fn sanitizer_renames_only_reserved_names() {
    // Sanity check: legitimate channel names pass through unchanged.
    for benign in [
        "telegram",
        "slack",
        "discord",
        "api",
        "ext-cron",
        "custom-bot",
    ] {
        assert_eq!(
            sanitize_channel_name(benign),
            benign,
            "non-reserved channel {benign:?} must pass through unchanged"
        );
        assert!(
            !is_reserved_system_channel(benign),
            "{benign:?} must not be flagged as reserved"
        );
    }

    // Every name listed in the reservation list must round-trip to `ext-<name>`.
    for &reserved in RESERVED_SYSTEM_CHANNEL_NAMES {
        let sanitized = sanitize_channel_name(reserved);
        assert_eq!(
            sanitized,
            format!("ext-{reserved}"),
            "reserved channel {reserved:?} must be renamed to ext-<name>"
        );
    }
}

#[test]
fn sanitized_external_channel_remains_stable_across_invocations() {
    // Two adapters that both pass `Custom("CRON")` must land on the SAME
    // external session (so they share history with each other) — they
    // just must not share it with the internal cron path. This pins the
    // "rename, not reject" choice: the alternative of generating a unique
    // suffix per call would silently fragment legitimate external channel
    // history.
    let agent = AgentId::new();
    let a = SessionId::for_channel(agent, &sanitize_channel_name("CRON"));
    let b = SessionId::for_channel(agent, &sanitize_channel_name("cron"));
    let c = SessionId::for_channel(agent, &sanitize_channel_name("  Cron  "));
    assert_eq!(a, b, "two external `cron` adapters must share a session");
    assert_eq!(a, c, "lowercase + trim normalization must apply");
}
