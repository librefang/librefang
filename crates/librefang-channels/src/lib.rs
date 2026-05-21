//! Channel Bridge Layer for the LibreFang Agent OS.
//!
//! Provides 40+ pluggable messaging integrations that convert platform messages
//! into unified `ChannelMessage` events for the kernel.
//!
//! Channels are gated behind cargo feature flags (`channel-xxx`).
//! The `default` feature enables popular channels; use `all-channels` for everything.

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

// Individual channel adapters — feature-gated (alphabetical order)
// dingtalk migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.dingtalk); no longer an in-process channel.
// discord migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.discord); no longer an in-process channel.
// email migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.email); no longer an in-process channel.
// feishu migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.feishu); no longer an in-process channel.
#[cfg(feature = "channel-google-chat")]
pub mod google_chat;
// line migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.line); no longer an in-process channel.
// matrix migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.matrix); no longer an in-process channel.
// mattermost migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.mattermost); no longer an in-process channel.
// qq migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.qq); no longer an in-process channel.
// signal migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.signal); no longer an in-process channel.
// slack migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.slack); no longer an in-process channel.
#[cfg(feature = "channel-teams")]
pub mod teams;
// webex migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.webex); no longer an in-process channel.
#[cfg(feature = "channel-webhook")]
pub mod webhook;
#[cfg(feature = "channel-wechat")]
pub mod wechat;
// wecom migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.wecom); no longer an in-process channel.
#[cfg(feature = "channel-whatsapp")]
pub mod whatsapp;
// zulip migrated to an out-of-process sidecar adapter
// (librefang.sidecar.adapters.zulip); no longer an in-process channel.
