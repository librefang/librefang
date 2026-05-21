//! Channel infrastructure for the LibreFang Agent OS.
//!
//! Every channel adapter is out-of-process — see `librefang.sidecar.adapters.*`
//! in the SDK at `sdk/python/`. This crate owns the **trampoline** that
//! connects the kernel to those sidecars (`sidecar.rs`), the shared bridge
//! types every adapter speaks (`types`, `bridge`, `router`, `commands`,
//! `formatter`, `sanitizer`, `roster`, `rate_limiter`, `thread_ownership`,
//! `group_history`, `message_journal`, `message_truncator`,
//! `attachment_enrich`), and the shared HTTP client (`http_client`).
//!
//! No in-process channel adapters live here. Re-introducing one requires
//! editing `crates/librefang-channels/src/channels-allowlist.txt` — see
//! the file header and `xtask::ci::check_channel_policy`. New channels
//! ship as sidecars; the policy ratchet enforces it.

// Core infrastructure — always compiled
pub mod attachment_enrich;
pub mod bridge;
pub mod commands;
pub mod formatter;
pub mod group_history;
pub mod http_client;
pub mod message_journal;
pub mod message_truncator;
pub mod rate_limiter;
pub mod roster;
pub mod router;
pub mod sanitizer;
pub mod sidecar;
pub mod thread_ownership;
pub mod types;

pub use message_truncator::{
    split_to_utf16_chunks, truncate_to_utf16_limit, utf16_len, DISCORD_MESSAGE_LIMIT,
    TELEGRAM_CAPTION_LIMIT, TELEGRAM_MESSAGE_LIMIT,
};
