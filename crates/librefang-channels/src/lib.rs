//! Channel Bridge Layer for the LibreFang Agent OS.
//!
//! Provides 40+ pluggable messaging integrations that convert platform messages
//! into unified `ChannelMessage` events for the kernel.
//!
//! Channels are gated behind cargo feature flags (`channel-xxx`).
//! The `default` feature enables popular channels; use `all-channels` for everything.

// Core infrastructure — always compiled
pub mod bridge;
pub mod formatter;
pub mod router;
pub mod types;

// Individual channel adapters — feature-gated
#[cfg(feature = "channel-telegram")]
pub mod telegram;
#[cfg(feature = "channel-discord")]
pub mod discord;
#[cfg(feature = "channel-slack")]
pub mod slack;
#[cfg(feature = "channel-matrix")]
pub mod matrix;
#[cfg(feature = "channel-email")]
pub mod email;
#[cfg(feature = "channel-webhook")]
pub mod webhook;
#[cfg(feature = "channel-whatsapp")]
pub mod whatsapp;
#[cfg(feature = "channel-signal")]
pub mod signal;
#[cfg(feature = "channel-teams")]
pub mod teams;
#[cfg(feature = "channel-mattermost")]
pub mod mattermost;
#[cfg(feature = "channel-irc")]
pub mod irc;
#[cfg(feature = "channel-google-chat")]
pub mod google_chat;
#[cfg(feature = "channel-twitch")]
pub mod twitch;
#[cfg(feature = "channel-rocketchat")]
pub mod rocketchat;
#[cfg(feature = "channel-zulip")]
pub mod zulip;
#[cfg(feature = "channel-xmpp")]
pub mod xmpp;
// Wave 3 — High-value channels
#[cfg(feature = "channel-bluesky")]
pub mod bluesky;
#[cfg(feature = "channel-feishu")]
pub mod feishu;
#[cfg(feature = "channel-line")]
pub mod line;
#[cfg(feature = "channel-mastodon")]
pub mod mastodon;
#[cfg(feature = "channel-messenger")]
pub mod messenger;
#[cfg(feature = "channel-reddit")]
pub mod reddit;
#[cfg(feature = "channel-revolt")]
pub mod revolt;
#[cfg(feature = "channel-viber")]
pub mod viber;
// Wave 4 — Enterprise & community channels
#[cfg(feature = "channel-flock")]
pub mod flock;
#[cfg(feature = "channel-guilded")]
pub mod guilded;
#[cfg(feature = "channel-keybase")]
pub mod keybase;
#[cfg(feature = "channel-nextcloud")]
pub mod nextcloud;
#[cfg(feature = "channel-nostr")]
pub mod nostr;
#[cfg(feature = "channel-pumble")]
pub mod pumble;
#[cfg(feature = "channel-threema")]
pub mod threema;
#[cfg(feature = "channel-twist")]
pub mod twist;
#[cfg(feature = "channel-webex")]
pub mod webex;
// Wave 5 — Niche & differentiating channels
#[cfg(feature = "channel-dingtalk")]
pub mod dingtalk;
#[cfg(feature = "channel-discourse")]
pub mod discourse;
#[cfg(feature = "channel-gitter")]
pub mod gitter;
#[cfg(feature = "channel-gotify")]
pub mod gotify;
#[cfg(feature = "channel-linkedin")]
pub mod linkedin;
#[cfg(feature = "channel-mumble")]
pub mod mumble;
#[cfg(feature = "channel-ntfy")]
pub mod ntfy;
#[cfg(feature = "channel-qq")]
pub mod qq;
#[cfg(feature = "channel-wecom")]
pub mod wecom;
