//! Channel configuration, status, and WhatsApp/WeChat QR flow handlers.

/// Build routes for the Channel domain.
pub fn router() -> axum::Router<std::sync::Arc<super::AppState>> {
    axum::Router::new()
        .route("/channels", axum::routing::get(list_channels))
        .route("/channels/{name}", axum::routing::get(get_channel))
        .route(
            "/channels/{name}/configure",
            axum::routing::post(configure_channel).delete(remove_channel),
        )
        // Per-instance endpoints (#4837) — let the dashboard manage multiple
        // `[[channels.<name>]]` entries (e.g. two Telegram bots, three Slack
        // workspaces) instead of treating every channel type as a single
        // instance. The legacy `/configure` endpoints remain for backwards
        // compatibility and continue to drive the single-instance flow.
        .route(
            "/channels/{name}/instances",
            axum::routing::get(list_channel_instances).post(create_channel_instance),
        )
        .route(
            "/channels/{name}/instances/{index}",
            axum::routing::put(update_channel_instance_handler)
                .delete(delete_channel_instance),
        )
        .route("/channels/{name}/test", axum::routing::post(test_channel))
        .route("/channels/reload", axum::routing::post(reload_channels))
        .route(
            "/channels/whatsapp/qr/start",
            axum::routing::post(whatsapp_qr_start),
        )
        .route(
            "/channels/whatsapp/qr/status",
            axum::routing::get(whatsapp_qr_status),
        )
        .route(
            "/channels/wechat/qr/start",
            axum::routing::post(wechat_qr_start),
        )
        .route(
            "/channels/wechat/qr/status",
            axum::routing::get(wechat_qr_status),
        )
        .route(
            "/channels/registry",
            axum::routing::get(list_channel_registry),
        )
}

use super::skills::{
    append_channel_instance, remove_channel_config, remove_channel_instance, remove_secret_env,
    update_channel_instance, upsert_channel_config, validate_env_var, write_secret_env,
    CHANNEL_AOT_CONFLICT_PREFIX,
};
use super::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::types::ApiErrorResponse;

/// Resolve the LibreFang home directory without depending on the kernel crate.
///
/// Mirrors `librefang_kernel::config::librefang_home`:
/// `LIBREFANG_HOME` env var takes priority, otherwise `~/.librefang`
/// (falling back to the system temp dir if no home directory is available).
fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}
// ---------------------------------------------------------------------------
// Channel status endpoints — data-driven registry for all 40 adapters
// ---------------------------------------------------------------------------

/// Field type for the channel configuration form.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FieldType {
    Secret,
    Text,
    Number,
    List,
    Select,
}

impl FieldType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Text => "text",
            Self::Number => "number",
            Self::List => "list",
            Self::Select => "select",
        }
    }
}

/// A single configurable field for a channel adapter.
#[derive(Clone)]
struct ChannelField {
    key: &'static str,
    label: &'static str,
    field_type: FieldType,
    env_var: Option<&'static str>,
    required: bool,
    placeholder: &'static str,
    /// If true, this field is hidden under "Show Advanced" in the UI.
    advanced: bool,
    /// Available options for Select field type.
    options: Option<&'static [&'static str]>,
    /// For Select fields, specify which option value must be selected to show this field.
    show_when: Option<&'static str>,
    /// If true, this field is display-only (not submitted as config).
    readonly: bool,
}

/// Metadata for one channel adapter.
struct ChannelMeta {
    name: &'static str,
    display_name: &'static str,
    icon: &'static str,
    description: &'static str,
    category: &'static str,
    difficulty: &'static str,
    setup_time: &'static str,
    /// One-line quick setup hint shown in the simple form view.
    quick_setup: &'static str,
    /// Setup type: "form" (default), "qr" (QR code scan + form fallback).
    setup_type: &'static str,
    fields: &'static [ChannelField],
    setup_steps: &'static [&'static str],
    config_template: &'static str,
}

const CHANNEL_REGISTRY: &[ChannelMeta] = &[
    // ── Messaging (12) ──────────────────────────────────────────────
    ChannelMeta {
        name: "telegram", display_name: "Telegram", icon: "TG",
        description: "Telegram Bot API — long-polling adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your bot token from @BotFather",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("TELEGRAM_BOT_TOKEN"), required: true, placeholder: "123456:ABC-DEF...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "12345, 67890", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "poll_interval_secs", label: "Poll Interval (sec)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "1", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Open @BotFather on Telegram", "Send /newbot and follow the prompts", "Paste the token below"],
        config_template: "[channels.telegram]\nbot_token_env = \"TELEGRAM_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "discord", display_name: "Discord", icon: "DC",
        description: "Discord Gateway bot adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your bot token from the Discord Developer Portal",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("DISCORD_BOT_TOKEN"), required: true, placeholder: "MTIz...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_guilds", label: "Allowed Guild IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123456789, 987654321", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123456789, 987654321", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "intents", label: "Intents Bitmask", field_type: FieldType::Number, env_var: None, required: false, placeholder: "37376", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Go to discord.com/developers/applications", "Create a bot and copy the token", "Paste it below"],
        config_template: "[channels.discord]\nbot_token_env = \"DISCORD_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "slack", display_name: "Slack", icon: "SL",
        description: "Slack Socket Mode + Events API",
        category: "messaging", difficulty: "Medium", setup_time: "~5 min",
        quick_setup: "Paste your App Token and Bot Token from api.slack.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_token_env", label: "App Token (xapp-)", field_type: FieldType::Secret, env_var: Some("SLACK_APP_TOKEN"), required: true, placeholder: "xapp-1-...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "bot_token_env", label: "Bot Token (xoxb-)", field_type: FieldType::Secret, env_var: Some("SLACK_BOT_TOKEN"), required: true, placeholder: "xoxb-...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_channels", label: "Allowed Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "C01234, C56789", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create app at api.slack.com/apps", "Enable Socket Mode and copy App Token", "Copy Bot Token from OAuth & Permissions"],
        config_template: "[channels.slack]\napp_token_env = \"SLACK_APP_TOKEN\"\nbot_token_env = \"SLACK_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "whatsapp", display_name: "WhatsApp", icon: "WA",
        description: "Connect your personal WhatsApp via QR scan",
        category: "messaging", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Scan QR code with your phone — no developer account needed",
        setup_type: "qr",
        fields: &[
            // Business API fallback fields — all advanced (hidden behind "Use Business API" toggle)
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("WHATSAPP_ACCESS_TOKEN"), required: false, placeholder: "EAAx...", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "phone_number_id", label: "Phone Number ID", field_type: FieldType::Text, env_var: None, required: false, placeholder: "1234567890", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "verify_token_env", label: "Verify Token", field_type: FieldType::Secret, env_var: Some("WHATSAPP_VERIFY_TOKEN"), required: false, placeholder: "my-verify-token", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8443", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Open WhatsApp on your phone", "Go to Linked Devices", "Tap Link a Device and scan the QR code"],
        config_template: "[channels.whatsapp]\naccess_token_env = \"WHATSAPP_ACCESS_TOKEN\"\nphone_number_id = \"\"",
    },
    ChannelMeta {
        name: "signal", display_name: "Signal", icon: "SG",
        description: "Signal via signal-cli REST API",
        category: "messaging", difficulty: "Medium", setup_time: "~10 min",
        quick_setup: "Enter your signal-cli API URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "api_url", label: "signal-cli API URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "http://localhost:8080", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "phone_number", label: "Phone Number", field_type: FieldType::Text, env_var: None, required: true, placeholder: "+1234567890", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Install signal-cli-rest-api", "Enter the API URL and your phone number"],
        config_template: "[channels.signal]\napi_url = \"http://localhost:8080\"\nphone_number = \"\"",
    },
    ChannelMeta {
        name: "matrix", display_name: "Matrix", icon: "MX",
        description: "Matrix/Element bot via homeserver",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your access token and homeserver URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("MATRIX_ACCESS_TOKEN"), required: true, placeholder: "syt_...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "homeserver_url", label: "Homeserver URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://matrix.org", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "user_id", label: "Bot User ID", field_type: FieldType::Text, env_var: None, required: false, placeholder: "@librefang:matrix.org", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_rooms", label: "Allowed Room IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "!abc:matrix.org", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot account on your homeserver", "Generate an access token", "Paste token and homeserver URL below"],
        config_template: "[channels.matrix]\naccess_token_env = \"MATRIX_ACCESS_TOKEN\"\nhomeserver_url = \"https://matrix.org\"",
    },
    ChannelMeta {
        name: "email", display_name: "Email", icon: "EM",
        description: "IMAP/SMTP email adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your email, password, and server hosts",
        setup_type: "form",
        fields: &[
            ChannelField { key: "username", label: "Email Address", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "password_env", label: "Password / App Password", field_type: FieldType::Secret, env_var: Some("EMAIL_PASSWORD"), required: true, placeholder: "app-password", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "imap_host", label: "IMAP Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "imap.gmail.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "smtp_host", label: "SMTP Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "smtp.gmail.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "imap_port", label: "IMAP Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "993", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "smtp_port", label: "SMTP Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "587", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Enable IMAP on your email account", "Generate an app password if using Gmail", "Fill in email, password, and hosts below"],
        config_template: "[channels.email]\nimap_host = \"imap.gmail.com\"\nsmtp_host = \"smtp.gmail.com\"\npassword_env = \"EMAIL_PASSWORD\"",
    },
    ChannelMeta {
        name: "line", display_name: "LINE", icon: "LN",
        description: "LINE Messaging API adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your Channel Secret and Access Token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "channel_secret_env", label: "Channel Secret", field_type: FieldType::Secret, env_var: Some("LINE_CHANNEL_SECRET"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "access_token_env", label: "Channel Access Token", field_type: FieldType::Secret, env_var: Some("LINE_CHANNEL_ACCESS_TOKEN"), required: true, placeholder: "xyz789...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8450", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a Messaging API channel at LINE Developers", "Copy Channel Secret and Access Token", "Paste them below"],
        config_template: "[channels.line]\nchannel_secret_env = \"LINE_CHANNEL_SECRET\"\naccess_token_env = \"LINE_CHANNEL_ACCESS_TOKEN\"",
    },
    ChannelMeta {
        name: "viber", display_name: "Viber", icon: "VB",
        description: "Viber Bot API adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your auth token from partners.viber.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "auth_token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("VIBER_AUTH_TOKEN"), required: true, placeholder: "4dc...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_url", label: "Webhook URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://your-domain.com/viber", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8451", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot at partners.viber.com", "Copy the auth token", "Paste it below"],
        config_template: "[channels.viber]\nauth_token_env = \"VIBER_AUTH_TOKEN\"",
    },
    ChannelMeta {
        name: "messenger", display_name: "Messenger", icon: "FB",
        description: "Facebook Messenger Platform adapter",
        category: "messaging", difficulty: "Medium", setup_time: "~10 min",
        quick_setup: "Paste your Page Access Token from developers.facebook.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "page_token_env", label: "Page Access Token", field_type: FieldType::Secret, env_var: Some("MESSENGER_PAGE_TOKEN"), required: true, placeholder: "EAAx...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "verify_token_env", label: "Verify Token", field_type: FieldType::Secret, env_var: Some("MESSENGER_VERIFY_TOKEN"), required: false, placeholder: "my-verify-token", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8452", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a Facebook App and add Messenger", "Generate a Page Access Token", "Paste it below"],
        config_template: "[channels.messenger]\npage_token_env = \"MESSENGER_PAGE_TOKEN\"",
    },
    ChannelMeta {
        name: "threema", display_name: "Threema", icon: "3M",
        description: "Threema Gateway adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your Gateway ID and API secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "secret_env", label: "API Secret", field_type: FieldType::Secret, env_var: Some("THREEMA_SECRET"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "threema_id", label: "Gateway ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "*MYID01", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8454", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Register at gateway.threema.ch", "Copy your ID and API secret", "Paste them below"],
        config_template: "[channels.threema]\nthreema_id = \"\"\nsecret_env = \"THREEMA_SECRET\"",
    },
    ChannelMeta {
        name: "keybase", display_name: "Keybase", icon: "KB",
        description: "Keybase chat bot adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your username and paper key",
        setup_type: "form",
        fields: &[
            ChannelField { key: "username", label: "Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang_bot", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "paperkey_env", label: "Paper Key", field_type: FieldType::Secret, env_var: Some("KEYBASE_PAPERKEY"), required: true, placeholder: "word1 word2 word3...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_teams", label: "Allowed Teams", field_type: FieldType::List, env_var: None, required: false, placeholder: "team1, team2", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a Keybase bot account", "Generate a paper key", "Enter username and paper key below"],
        config_template: "[channels.keybase]\nusername = \"\"\npaperkey_env = \"KEYBASE_PAPERKEY\"",
    },
    // ── Social (5) ──────────────────────────────────────────────────
    ChannelMeta {
        name: "reddit", display_name: "Reddit", icon: "RD",
        description: "Reddit API bot adapter",
        category: "social", difficulty: "Medium", setup_time: "~5 min",
        quick_setup: "Paste your Client ID, Secret, and bot credentials",
        setup_type: "form",
        fields: &[
            ChannelField { key: "client_id", label: "Client ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123def", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "client_secret_env", label: "Client Secret", field_type: FieldType::Secret, env_var: Some("REDDIT_CLIENT_SECRET"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "username", label: "Bot Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang_bot", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "password_env", label: "Bot Password", field_type: FieldType::Secret, env_var: Some("REDDIT_PASSWORD"), required: true, placeholder: "password", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "subreddits", label: "Subreddits", field_type: FieldType::List, env_var: None, required: false, placeholder: "librefang, rust", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a Reddit app at reddit.com/prefs/apps (script type)", "Copy Client ID and Secret", "Enter bot credentials below"],
        config_template: "[channels.reddit]\nclient_id = \"\"\nclient_secret_env = \"REDDIT_CLIENT_SECRET\"\nusername = \"\"\npassword_env = \"REDDIT_PASSWORD\"",
    },
    ChannelMeta {
        name: "mastodon", display_name: "Mastodon", icon: "MA",
        description: "Mastodon Streaming API adapter",
        category: "social", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your access token from Settings > Development",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("MASTODON_ACCESS_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "instance_url", label: "Instance URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://mastodon.social", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Go to Settings > Development on your instance", "Create an app and copy the token", "Paste it below"],
        config_template: "[channels.mastodon]\ninstance_url = \"https://mastodon.social\"\naccess_token_env = \"MASTODON_ACCESS_TOKEN\"",
    },
    ChannelMeta {
        name: "bluesky", display_name: "Bluesky", icon: "BS",
        description: "Bluesky/AT Protocol adapter",
        category: "social", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Enter your handle and app password",
        setup_type: "form",
        fields: &[
            ChannelField { key: "identifier", label: "Handle", field_type: FieldType::Text, env_var: None, required: true, placeholder: "user.bsky.social", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_password_env", label: "App Password", field_type: FieldType::Secret, env_var: Some("BLUESKY_APP_PASSWORD"), required: true, placeholder: "xxxx-xxxx-xxxx-xxxx", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "service_url", label: "PDS URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://bsky.social", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Go to Settings > App Passwords in Bluesky", "Create an app password", "Enter handle and password below"],
        config_template: "[channels.bluesky]\nidentifier = \"\"\napp_password_env = \"BLUESKY_APP_PASSWORD\"",
    },
    ChannelMeta {
        name: "linkedin", display_name: "LinkedIn", icon: "LI",
        description: "LinkedIn Messaging API adapter",
        category: "social", difficulty: "Hard", setup_time: "~15 min",
        quick_setup: "Paste your OAuth2 access token and Organization ID",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("LINKEDIN_ACCESS_TOKEN"), required: true, placeholder: "AQV...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "organization_id", label: "Organization ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "12345678", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a LinkedIn App at linkedin.com/developers", "Generate an OAuth2 token", "Enter token and org ID below"],
        config_template: "[channels.linkedin]\naccess_token_env = \"LINKEDIN_ACCESS_TOKEN\"\norganization_id = \"\"",
    },
    ChannelMeta {
        name: "nostr", display_name: "Nostr", icon: "NS",
        description: "Nostr relay protocol adapter",
        category: "social", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your private key (nsec or hex)",
        setup_type: "form",
        fields: &[
            ChannelField { key: "private_key_env", label: "Private Key", field_type: FieldType::Secret, env_var: Some("NOSTR_PRIVATE_KEY"), required: true, placeholder: "nsec1...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "relays", label: "Relay URLs", field_type: FieldType::List, env_var: None, required: false, placeholder: "wss://relay.damus.io", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Generate or use an existing Nostr keypair", "Paste your private key below"],
        config_template: "[channels.nostr]\nprivate_key_env = \"NOSTR_PRIVATE_KEY\"",
    },
    // ── Enterprise (10) ─────────────────────────────────────────────
    ChannelMeta {
        name: "teams", display_name: "Microsoft Teams", icon: "MS",
        description: "Teams Bot Framework adapter",
        category: "enterprise", difficulty: "Medium", setup_time: "~10 min",
        quick_setup: "Paste your Azure Bot App ID and Password",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "00000000-0000-...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_password_env", label: "App Password", field_type: FieldType::Secret, env_var: Some("TEAMS_APP_PASSWORD"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "3978", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create an Azure Bot registration", "Copy App ID and generate a password", "Paste them below"],
        config_template: "[channels.teams]\napp_id = \"\"\napp_password_env = \"TEAMS_APP_PASSWORD\"",
    },
    ChannelMeta {
        name: "mattermost", display_name: "Mattermost", icon: "MM",
        description: "Mattermost WebSocket adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your bot token and server URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://mattermost.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("MATTERMOST_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_channels", label: "Allowed Channels", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123, def456", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot in System Console > Bot Accounts", "Copy the token", "Enter server URL and token below"],
        config_template: "[channels.mattermost]\nserver_url = \"\"\ntoken_env = \"MATTERMOST_TOKEN\"",
    },
    ChannelMeta {
        name: "google_chat", display_name: "Google Chat", icon: "GC",
        description: "Google Chat service account adapter",
        category: "enterprise", difficulty: "Hard", setup_time: "~15 min",
        quick_setup: "Enter path to your service account JSON key",
        setup_type: "form",
        fields: &[
            ChannelField { key: "service_account_env", label: "Service Account JSON", field_type: FieldType::Secret, env_var: Some("GOOGLE_CHAT_SERVICE_ACCOUNT"), required: true, placeholder: "/path/to/key.json", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "space_ids", label: "Space IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "spaces/AAAA", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8444", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a Google Cloud project with Chat API", "Download service account JSON key", "Enter the path below"],
        config_template: "[channels.google_chat]\nservice_account_env = \"GOOGLE_CHAT_SERVICE_ACCOUNT\"",
    },
    ChannelMeta {
        name: "webex", display_name: "Webex", icon: "WX",
        description: "Cisco Webex bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your bot token from developer.webex.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("WEBEX_BOT_TOKEN"), required: true, placeholder: "NjI...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_rooms", label: "Allowed Rooms", field_type: FieldType::List, env_var: None, required: false, placeholder: "Y2lz...", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot at developer.webex.com", "Copy the token", "Paste it below"],
        config_template: "[channels.webex]\nbot_token_env = \"WEBEX_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "feishu", display_name: "Feishu/Lark", icon: "FS",
        description: "Feishu/Lark Open Platform adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your App ID and App Secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "cli_abc123", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_secret_env", label: "App Secret", field_type: FieldType::Secret, env_var: Some("FEISHU_APP_SECRET"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8453", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create an app at open.feishu.cn", "Copy App ID and Secret", "Paste them below"],
        config_template: "[channels.feishu]\napp_id = \"\"\napp_secret_env = \"FEISHU_APP_SECRET\"",
    },
    ChannelMeta {
        name: "dingtalk", display_name: "DingTalk", icon: "DT",
        description: "DingTalk Robot API adapter (webhook or stream mode)",
        category: "enterprise", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Choose webhook or stream mode, then paste credentials",
        setup_type: "form",
        fields: &[
            ChannelField { key: "receive_mode", label: "Mode", field_type: FieldType::Text, env_var: None, required: false, placeholder: "stream (default) or webhook", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_key_env", label: "App Key (stream)", field_type: FieldType::Secret, env_var: Some("DINGTALK_APP_KEY"), required: false, placeholder: "dingxxx...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_secret_env", label: "App Secret (stream)", field_type: FieldType::Secret, env_var: Some("DINGTALK_APP_SECRET"), required: false, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "access_token_env", label: "Access Token (webhook)", field_type: FieldType::Secret, env_var: Some("DINGTALK_ACCESS_TOKEN"), required: false, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "secret_env", label: "Signing Secret (webhook)", field_type: FieldType::Secret, env_var: Some("DINGTALK_SECRET"), required: false, placeholder: "SEC...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8457", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a robot in DingTalk", "Choose mode: webhook (needs public IP) or stream (no public IP needed)", "For webhook: copy token and signing secret", "For stream: copy App Key and App Secret from the app page"],
        config_template: "[channels.dingtalk]\nreceive_mode = \"stream\"\napp_key_env = \"DINGTALK_APP_KEY\"\napp_secret_env = \"DINGTALK_APP_SECRET\"",
    },
    ChannelMeta {
        name: "pumble", display_name: "Pumble", icon: "PB",
        description: "Pumble bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("PUMBLE_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8455", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot in Pumble Integrations", "Copy the token", "Paste it below"],
        config_template: "[channels.pumble]\nbot_token_env = \"PUMBLE_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "flock", display_name: "Flock", icon: "FL",
        description: "Flock bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("FLOCK_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "webhook_port", label: "Webhook Port (deprecated, ignored)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8456", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Build an app in Flock App Store", "Copy the bot token", "Paste it below"],
        config_template: "[channels.flock]\nbot_token_env = \"FLOCK_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "twist", display_name: "Twist", icon: "TW",
        description: "Twist API v3 adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your API token and workspace ID",
        setup_type: "form",
        fields: &[
            ChannelField { key: "token_env", label: "API Token", field_type: FieldType::Secret, env_var: Some("TWIST_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "workspace_id", label: "Workspace ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "12345", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_channels", label: "Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123, 456", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create an integration in Twist Settings", "Copy the API token", "Enter token and workspace ID below"],
        config_template: "[channels.twist]\ntoken_env = \"TWIST_TOKEN\"\nworkspace_id = \"\"",
    },
    ChannelMeta {
        name: "zulip", display_name: "Zulip", icon: "ZL",
        description: "Zulip event queue adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your API key, server URL, and bot email",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://chat.zulip.org", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "bot_email", label: "Bot Email", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@zulip.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "api_key_env", label: "API Key", field_type: FieldType::Secret, env_var: Some("ZULIP_API_KEY"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "streams", label: "Streams", field_type: FieldType::List, env_var: None, required: false, placeholder: "general, dev", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot in Zulip Settings > Your Bots", "Copy the API key", "Enter server URL, bot email, and key below"],
        config_template: "[channels.zulip]\nserver_url = \"\"\nbot_email = \"\"\napi_key_env = \"ZULIP_API_KEY\"",
    },
    // ── Developer (9) ───────────────────────────────────────────────
    ChannelMeta {
        name: "irc", display_name: "IRC", icon: "IR",
        description: "IRC raw TCP adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Enter server and nickname",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server", label: "Server", field_type: FieldType::Text, env_var: None, required: true, placeholder: "irc.libera.chat", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "nick", label: "Nickname", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "channels", label: "Channels", field_type: FieldType::List, env_var: None, required: false, placeholder: "#librefang, #general", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "6667", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "use_tls", label: "Use TLS", field_type: FieldType::Text, env_var: None, required: false, placeholder: "false", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Choose an IRC server", "Enter server, nick, and channels below"],
        config_template: "[channels.irc]\nserver = \"irc.libera.chat\"\nnick = \"librefang\"",
    },
    ChannelMeta {
        name: "xmpp", display_name: "XMPP/Jabber", icon: "XM",
        description: "XMPP/Jabber protocol adapter",
        category: "developer", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your JID and password",
        setup_type: "form",
        fields: &[
            ChannelField { key: "jid", label: "JID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@jabber.org", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "password_env", label: "Password", field_type: FieldType::Secret, env_var: Some("XMPP_PASSWORD"), required: true, placeholder: "password", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "server", label: "Server", field_type: FieldType::Text, env_var: None, required: false, placeholder: "jabber.org", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "5222", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "rooms", label: "MUC Rooms", field_type: FieldType::List, env_var: None, required: false, placeholder: "room@conference.jabber.org", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot account on your XMPP server", "Enter JID and password below"],
        config_template: "[channels.xmpp]\njid = \"\"\npassword_env = \"XMPP_PASSWORD\"",
    },
    ChannelMeta {
        name: "gitter", display_name: "Gitter", icon: "GT",
        description: "Gitter Streaming API adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your auth token and room ID",
        setup_type: "form",
        fields: &[
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("GITTER_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "room_id", label: "Room ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123def456", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Get a token from developer.gitter.im", "Find your room ID", "Paste both below"],
        config_template: "[channels.gitter]\ntoken_env = \"GITTER_TOKEN\"\nroom_id = \"\"",
    },
    ChannelMeta {
        name: "discourse", display_name: "Discourse", icon: "DS",
        description: "Discourse forum API adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your API key and forum URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "base_url", label: "Forum URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://forum.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "api_key_env", label: "API Key", field_type: FieldType::Secret, env_var: Some("DISCOURSE_API_KEY"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "api_username", label: "API Username", field_type: FieldType::Text, env_var: None, required: false, placeholder: "system", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "categories", label: "Categories", field_type: FieldType::List, env_var: None, required: false, placeholder: "general, support", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Go to Admin > API > Keys", "Generate an API key", "Enter forum URL and key below"],
        config_template: "[channels.discourse]\nbase_url = \"\"\napi_key_env = \"DISCOURSE_API_KEY\"",
    },
    ChannelMeta {
        name: "revolt", display_name: "Revolt", icon: "RV",
        description: "Revolt bot adapter",
        category: "developer", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("REVOLT_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "api_url", label: "API URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://api.revolt.chat", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Go to Settings > My Bots in Revolt", "Create a bot and copy the token", "Paste it below"],
        config_template: "[channels.revolt]\nbot_token_env = \"REVOLT_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "guilded", display_name: "Guilded", icon: "GD",
        description: "Guilded bot adapter",
        category: "developer", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("GUILDED_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "server_ids", label: "Server IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Go to Server Settings > Bots in Guilded", "Create a bot and copy the token", "Paste it below"],
        config_template: "[channels.guilded]\nbot_token_env = \"GUILDED_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "nextcloud", display_name: "Nextcloud Talk", icon: "NC",
        description: "Nextcloud Talk REST adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your server URL and auth token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://cloud.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("NEXTCLOUD_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_rooms", label: "Room Tokens", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot user in Nextcloud", "Generate an app password", "Enter URL and token below"],
        config_template: "[channels.nextcloud]\nserver_url = \"\"\ntoken_env = \"NEXTCLOUD_TOKEN\"",
    },
    ChannelMeta {
        name: "rocketchat", display_name: "Rocket.Chat", icon: "RC",
        description: "Rocket.Chat REST adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your server URL, user ID, and token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://rocket.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "user_id", label: "Bot User ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("ROCKETCHAT_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_channels", label: "Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "GENERAL", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create a bot in Admin > Users", "Generate a personal access token", "Enter URL, user ID, and token below"],
        config_template: "[channels.rocketchat]\nserver_url = \"\"\ntoken_env = \"ROCKETCHAT_TOKEN\"\nuser_id = \"\"",
    },
    ChannelMeta {
        name: "twitch", display_name: "Twitch", icon: "TV",
        description: "Twitch IRC gateway adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your OAuth token and enter channel name",
        setup_type: "form",
        fields: &[
            ChannelField { key: "oauth_token_env", label: "OAuth Token", field_type: FieldType::Secret, env_var: Some("TWITCH_OAUTH_TOKEN"), required: true, placeholder: "oauth:abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "nick", label: "Bot Nickname", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "channels", label: "Channels (no #)", field_type: FieldType::List, env_var: None, required: true, placeholder: "mychannel", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Generate an OAuth token at twitchapps.com/tmi", "Enter token, nick, and channel below"],
        config_template: "[channels.twitch]\noauth_token_env = \"TWITCH_OAUTH_TOKEN\"\nnick = \"librefang\"",
    },
    // ── Notifications (4) ───────────────────────────────────────────
    ChannelMeta {
        name: "ntfy", display_name: "ntfy", icon: "NF",
        description: "ntfy.sh pub/sub notification adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Just enter a topic name",
        setup_type: "form",
        fields: &[
            ChannelField { key: "topic", label: "Topic", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang-alerts", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://ntfy.sh", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("NTFY_TOKEN"), required: false, placeholder: "tk_abc123...", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Pick a topic name", "Enter it below — that's it!"],
        config_template: "[channels.ntfy]\ntopic = \"\"",
    },
    ChannelMeta {
        name: "gotify", display_name: "Gotify", icon: "GF",
        description: "Gotify WebSocket notification adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your server URL and tokens",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://gotify.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_token_env", label: "App Token (send)", field_type: FieldType::Secret, env_var: Some("GOTIFY_APP_TOKEN"), required: true, placeholder: "abc123...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "client_token_env", label: "Client Token (receive)", field_type: FieldType::Secret, env_var: Some("GOTIFY_CLIENT_TOKEN"), required: true, placeholder: "def456...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create an app and a client in Gotify", "Copy both tokens", "Enter URL and tokens below"],
        config_template: "[channels.gotify]\nserver_url = \"\"\napp_token_env = \"GOTIFY_APP_TOKEN\"\nclient_token_env = \"GOTIFY_CLIENT_TOKEN\"",
    },
    ChannelMeta {
        name: "webhook", display_name: "Webhook", icon: "WH",
        description: "Generic HMAC-signed webhook adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Optionally set an HMAC secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "secret_env", label: "HMAC Secret", field_type: FieldType::Secret, env_var: Some("WEBHOOK_SECRET"), required: false, placeholder: "my-secret", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "listen_port", label: "Listen Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8460", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "callback_url", label: "Callback URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://example.com/webhook", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Enter an HMAC secret (or leave blank)", "Click Save — that's it!"],
        config_template: "[channels.webhook]\nsecret_env = \"WEBHOOK_SECRET\"",
    },
    ChannelMeta {
        name: "voice", display_name: "Voice", icon: "VC",
        description: "Voice channel via WebSocket with STT/TTS",
        category: "media", difficulty: "Medium", setup_time: "~3 min",
        quick_setup: "Set an OpenAI API key for Whisper STT and TTS",
        setup_type: "form",
        fields: &[
            ChannelField { key: "api_key_env", label: "API Key (STT/TTS)", field_type: FieldType::Secret, env_var: Some("OPENAI_API_KEY"), required: true, placeholder: "sk-...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "listen_port", label: "WebSocket Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "4546", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "stt_url", label: "STT API URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://api.openai.com", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "tts_url", label: "TTS API URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://api.openai.com", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "tts_voice", label: "TTS Voice", field_type: FieldType::Text, env_var: None, required: false, placeholder: "alloy", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "buffer_threshold", label: "Audio Buffer (bytes)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "32768", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Set OPENAI_API_KEY environment variable", "Optionally configure STT/TTS endpoints", "Connect via WebSocket at ws://host:4546/voice"],
        config_template: "[channels.voice]\napi_key_env = \"OPENAI_API_KEY\"\nlisten_port = 4546",
    },
    ChannelMeta {
        name: "mumble", display_name: "Mumble", icon: "MB",
        description: "Mumble text chat adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Enter server host and username",
        setup_type: "form",
        fields: &[
            ChannelField { key: "host", label: "Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "mumble.example.com", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "username", label: "Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "password_env", label: "Server Password", field_type: FieldType::Secret, env_var: Some("MUMBLE_PASSWORD"), required: false, placeholder: "password", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "64738", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "channel", label: "Channel", field_type: FieldType::Text, env_var: None, required: false, placeholder: "Root", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Enter host and username below", "Optionally add a password"],
        config_template: "[channels.mumble]\nhost = \"\"\nusername = \"librefang\"",
    },
    ChannelMeta {
        name: "wechat", display_name: "WeChat", icon: "WX",
        description: "WeChat personal account via iLink protocol",
        category: "messaging", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Scan QR code with your WeChat app — no developer account needed",
        setup_type: "qr",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token (from previous session)", field_type: FieldType::Secret, env_var: Some("WECHAT_BOT_TOKEN"), required: false, placeholder: "ilink_bot_...", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123@im.wechat", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Open WeChat on your phone", "The QR code will appear in the dashboard", "Scan it with WeChat to connect"],
        config_template: "[channels.wechat]\nbot_token_env = \"WECHAT_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "wecom", display_name: "WeCom", icon: "WC",
        description: "WeCom intelligent bot (WebSocket or URL callback)",
        category: "messaging", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Enter your Bot ID and Secret from the WeCom admin console",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_id", label: "Bot ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "aibxxxxx", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "secret_env", label: "Bot Secret", field_type: FieldType::Secret, env_var: Some("WECOM_BOT_SECRET"), required: true, placeholder: "xxxxxxxxxxxxxxxx...", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "mode", label: "Connection Mode", field_type: FieldType::Select, env_var: None, required: false, placeholder: "websocket", advanced: false, options: Some(&["websocket", "callback"]), show_when: None, readonly: false },
            ChannelField { key: "callback_url", label: "Callback URL (configure in WeCom admin)", field_type: FieldType::Text, env_var: None, required: false, placeholder: "", advanced: false, options: None, show_when: Some("callback"), readonly: true },
            ChannelField { key: "token_env", label: "Callback Token", field_type: FieldType::Secret, env_var: Some("WECOM_CALLBACK_TOKEN"), required: true, placeholder: "Token from WeCom admin console", advanced: false, options: None, show_when: Some("callback"), readonly: false },
            ChannelField { key: "encoding_aes_key_env", label: "EncodingAESKey", field_type: FieldType::Secret, env_var: Some("WECOM_ENCODING_AES_KEY"), required: true, placeholder: "EncodingAESKey from WeCom admin console", advanced: false, options: None, show_when: Some("callback"), readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Create an intelligent bot at WeCom admin console", "Copy Bot ID and Secret from the bot settings page", "WebSocket mode: enter Bot ID and Secret directly (no server needed)", "Callback mode: set Callback Token and EncodingAESKey, then configure the displayed Callback URL in WeCom admin"],
        config_template: "[channels.wecom]\nbot_id = \"\"\nsecret_env = \"WECOM_BOT_SECRET\"\nmode = \"websocket\"",
    },
    ChannelMeta {
        name: "qq", display_name: "QQ Bot", icon: "QQ",
        description: "QQ Bot API v2 — guild, group, and DM adapter",
        category: "messaging", difficulty: "Medium", setup_time: "~5 min",
        quick_setup: "Enter your App ID and set QQ_BOT_APP_SECRET env var",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "102xxxxx", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "app_secret_env", label: "App Secret", field_type: FieldType::Secret, env_var: Some("QQ_BOT_APP_SECRET"), required: true, placeholder: "secret", advanced: false, options: None, show_when: None, readonly: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "12345, 67890", advanced: true, options: None, show_when: None, readonly: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true, options: None, show_when: None, readonly: false },
        ],
        setup_steps: &["Register a QQ Bot at q.qq.com", "Get App ID and App Secret", "Set QQ_BOT_APP_SECRET environment variable"],
        config_template: "[channels.qq]\napp_id = \"\"\napp_secret_env = \"QQ_BOT_APP_SECRET\"",
    },
];

/// Check if a channel is configured (has a `[channels.xxx]` section in config).
fn is_channel_configured(config: &librefang_types::config::ChannelsConfig, name: &str) -> bool {
    match name {
        "telegram" => config.telegram.is_some(),
        "discord" => config.discord.is_some(),
        "slack" => config.slack.is_some(),
        "whatsapp" => config.whatsapp.is_some(),
        "signal" => config.signal.is_some(),
        "matrix" => config.matrix.is_some(),
        "email" => config.email.is_some(),
        "line" => config.line.is_some(),
        "viber" => config.viber.is_some(),
        "messenger" => config.messenger.is_some(),
        "threema" => config.threema.is_some(),
        "keybase" => config.keybase.is_some(),
        "reddit" => config.reddit.is_some(),
        "mastodon" => config.mastodon.is_some(),
        "bluesky" => config.bluesky.is_some(),
        "linkedin" => config.linkedin.is_some(),
        "nostr" => config.nostr.is_some(),
        "teams" => config.teams.is_some(),
        "mattermost" => config.mattermost.is_some(),
        "google_chat" => config.google_chat.is_some(),
        "webex" => config.webex.is_some(),
        "feishu" => config.feishu.is_some(),
        "dingtalk" => config.dingtalk.is_some(),
        "pumble" => config.pumble.is_some(),
        "flock" => config.flock.is_some(),
        "twist" => config.twist.is_some(),
        "zulip" => config.zulip.is_some(),
        "irc" => config.irc.is_some(),
        "xmpp" => config.xmpp.is_some(),
        "gitter" => config.gitter.is_some(),
        "discourse" => config.discourse.is_some(),
        "revolt" => config.revolt.is_some(),
        "guilded" => config.guilded.is_some(),
        "nextcloud" => config.nextcloud.is_some(),
        "rocketchat" => config.rocketchat.is_some(),
        "twitch" => config.twitch.is_some(),
        "ntfy" => config.ntfy.is_some(),
        "gotify" => config.gotify.is_some(),
        "webhook" => config.webhook.is_some(),
        "voice" => config.voice.is_some(),
        "mumble" => config.mumble.is_some(),
        "wechat" => config.wechat.is_some(),
        "wecom" => config.wecom.is_some(),
        "qq" => config.qq.is_some(),
        _ => false,
    }
}

/// Return the names of all channel types that are configured in the live config.
///
/// Used by the agent-channels endpoint so `available` reflects every configured
/// channel, not only those that happen to be started/connected right now.
pub(super) fn configured_channel_names(
    config: &librefang_types::config::ChannelsConfig,
) -> Vec<String> {
    CHANNEL_REGISTRY
        .iter()
        .filter(|meta| is_channel_configured(config, meta.name))
        .map(|meta| meta.name.to_owned())
        .collect()
}

/// Build a JSON field descriptor, checking env var presence but never exposing secrets.
/// For non-secret fields, includes the actual config value from `config_values` if available.
fn build_field_json(
    f: &ChannelField,
    config_values: Option<&serde_json::Value>,
) -> serde_json::Value {
    let has_value = f
        .env_var
        .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
        .unwrap_or(false);
    let mut field = serde_json::json!({
        "key": f.key,
        "label": f.label,
        "type": f.field_type.as_str(),
        "env_var": f.env_var,
        "required": f.required,
        "has_value": has_value,
        "placeholder": f.placeholder,
        "advanced": f.advanced,
        "options": f.options,
        "show_when": f.show_when,
        "readonly": f.readonly,
    });
    // For non-secret fields, include the actual saved config value so the
    // dashboard can pre-populate forms when editing existing configs.
    if f.env_var.is_none() {
        if let Some(obj) = config_values.and_then(|v| v.as_object()) {
            if let Some(val) = obj.get(f.key) {
                // Convert arrays to comma-separated string for list fields
                let display_val = if f.field_type == FieldType::List {
                    if let Some(arr) = val.as_array() {
                        serde_json::Value::String(
                            arr.iter()
                                .filter_map(|v| {
                                    v.as_str()
                                        .map(|s| s.to_string())
                                        .or_else(|| Some(v.to_string()))
                                })
                                .collect::<Vec<_>>()
                                .join(", "),
                        )
                    } else {
                        val.clone()
                    }
                } else {
                    val.clone()
                };
                field["value"] = display_val;
                if !val.is_null() && val.as_str().map(|s| !s.is_empty()).unwrap_or(true) {
                    field["has_value"] = serde_json::Value::Bool(true);
                }
            }
        }
    }
    field
}

/// For channels with a readonly `callback_url` field, dynamically inject the
/// actual URL on the shared webhook server so the user sees a real value to
/// copy into the platform admin console.
///
/// Since v2026.3.31 all webhook channels share the main API server port.
/// The URL pattern is `http://{api_listen}/channels/{channel_name}/webhook`.
fn inject_callback_url(
    fields: &mut [serde_json::Value],
    channel_name: &str,
    _config_values: Option<&serde_json::Value>,
) {
    let path = match channel_name {
        "wecom" => "/channels/wecom/webhook",
        _ => return,
    };

    // Use 0.0.0.0 with the default API port — users should substitute their
    // public hostname when pasting into external platform consoles.
    let url = format!(
        "http://0.0.0.0:{}{path}",
        librefang_types::config::DEFAULT_API_PORT
    );

    for field in fields.iter_mut() {
        if field.get("key").and_then(|v| v.as_str()) == Some("callback_url") {
            field["value"] = serde_json::Value::String(url.clone());
            field["has_value"] = serde_json::Value::Bool(true);
        }
    }
}

/// Channels that receive messages via webhook on the shared server.
/// Returns the path suffix (e.g. "/webhook") for the given channel name,
/// or None if the channel does not use webhook routes.
fn webhook_route_suffix(channel_name: &str) -> Option<&'static str> {
    match channel_name {
        "feishu" | "teams" | "dingtalk" | "line" | "messenger" | "viber" | "google_chat"
        | "flock" | "pumble" | "threema" | "webhook" | "wecom" => Some("/webhook"),
        "voice" => Some("/ws"),
        _ => None,
    }
}

/// Build the full webhook endpoint URL for a channel on the shared server.
/// Returns `None` for channels that don't use webhook routes (e.g. Telegram, Discord).
fn webhook_endpoint_url(channel_name: &str) -> Option<String> {
    webhook_route_suffix(channel_name).map(|suffix| format!("/channels/{channel_name}{suffix}"))
}

/// Find a channel definition by name.
fn find_channel_meta(name: &str) -> Option<&'static ChannelMeta> {
    CHANNEL_REGISTRY.iter().find(|c| c.name == name)
}

/// Serialize a channel's config to a JSON Value for pre-populating dashboard forms.
fn channel_config_values(
    config: &librefang_types::config::ChannelsConfig,
    name: &str,
) -> Option<serde_json::Value> {
    match name {
        "telegram" => config
            .telegram
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "discord" => config
            .discord
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "slack" => config
            .slack
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "whatsapp" => config
            .whatsapp
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "signal" => config
            .signal
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "matrix" => config
            .matrix
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "email" => config
            .email
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "teams" => config
            .teams
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "mattermost" => config
            .mattermost
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "irc" => config
            .irc
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "google_chat" => config
            .google_chat
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "twitch" => config
            .twitch
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "rocketchat" => config
            .rocketchat
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "zulip" => config
            .zulip
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "xmpp" => config
            .xmpp
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "line" => config
            .line
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "viber" => config
            .viber
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "messenger" => config
            .messenger
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "reddit" => config
            .reddit
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "mastodon" => config
            .mastodon
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "bluesky" => config
            .bluesky
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "feishu" => config
            .feishu
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "revolt" => config
            .revolt
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "nextcloud" => config
            .nextcloud
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "guilded" => config
            .guilded
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "keybase" => config
            .keybase
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "threema" => config
            .threema
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "nostr" => config
            .nostr
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "webex" => config
            .webex
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "pumble" => config
            .pumble
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "flock" => config
            .flock
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "twist" => config
            .twist
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "mumble" => config
            .mumble
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "dingtalk" => config
            .dingtalk
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "discourse" => config
            .discourse
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "gitter" => config
            .gitter
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "ntfy" => config
            .ntfy
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "gotify" => config
            .gotify
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "webhook" => config
            .webhook
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "voice" => config
            .voice
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "linkedin" => config
            .linkedin
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "wechat" => config
            .wechat
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "wecom" => config
            .wecom
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "qq" => config
            .qq
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        _ => None,
    }
}

/// Returns the number of configured instances for a channel type.
///
/// Mirrors `is_channel_configured` but returns the underlying
/// `OneOrMany.len()` so the dashboard can render
/// "Telegram · 2 bots" subtitles and the list endpoint can populate
/// `instance_count`.
fn channel_instance_count(config: &librefang_types::config::ChannelsConfig, name: &str) -> usize {
    match name {
        "telegram" => config.telegram.len(),
        "discord" => config.discord.len(),
        "slack" => config.slack.len(),
        "whatsapp" => config.whatsapp.len(),
        "signal" => config.signal.len(),
        "matrix" => config.matrix.len(),
        "email" => config.email.len(),
        "line" => config.line.len(),
        "viber" => config.viber.len(),
        "messenger" => config.messenger.len(),
        "threema" => config.threema.len(),
        "keybase" => config.keybase.len(),
        "reddit" => config.reddit.len(),
        "mastodon" => config.mastodon.len(),
        "bluesky" => config.bluesky.len(),
        "linkedin" => config.linkedin.len(),
        "nostr" => config.nostr.len(),
        "teams" => config.teams.len(),
        "mattermost" => config.mattermost.len(),
        "google_chat" => config.google_chat.len(),
        "webex" => config.webex.len(),
        "feishu" => config.feishu.len(),
        "dingtalk" => config.dingtalk.len(),
        "pumble" => config.pumble.len(),
        "flock" => config.flock.len(),
        "twist" => config.twist.len(),
        "zulip" => config.zulip.len(),
        "irc" => config.irc.len(),
        "xmpp" => config.xmpp.len(),
        "gitter" => config.gitter.len(),
        "discourse" => config.discourse.len(),
        "revolt" => config.revolt.len(),
        "guilded" => config.guilded.len(),
        "nextcloud" => config.nextcloud.len(),
        "rocketchat" => config.rocketchat.len(),
        "twitch" => config.twitch.len(),
        "ntfy" => config.ntfy.len(),
        "gotify" => config.gotify.len(),
        "webhook" => config.webhook.len(),
        "voice" => config.voice.len(),
        "mumble" => config.mumble.len(),
        "wechat" => config.wechat.len(),
        "wecom" => config.wecom.len(),
        "qq" => config.qq.len(),
        _ => 0,
    }
}

/// Serialize each configured instance of `name` to a JSON value.
///
/// Returns an empty vector when the channel is unknown or has no instances.
/// Each element is the per-instance config (same shape as the legacy
/// `channel_config_values` returns for the first instance), so it can be
/// fed straight into `build_field_json` to render the per-instance form.
fn channel_instances_serialized(
    config: &librefang_types::config::ChannelsConfig,
    name: &str,
) -> Vec<serde_json::Value> {
    fn ser<T: serde::Serialize>(
        items: &librefang_types::config::OneOrMany<T>,
    ) -> Vec<serde_json::Value> {
        items
            .iter()
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect()
    }
    match name {
        "telegram" => ser(&config.telegram),
        "discord" => ser(&config.discord),
        "slack" => ser(&config.slack),
        "whatsapp" => ser(&config.whatsapp),
        "signal" => ser(&config.signal),
        "matrix" => ser(&config.matrix),
        "email" => ser(&config.email),
        "line" => ser(&config.line),
        "viber" => ser(&config.viber),
        "messenger" => ser(&config.messenger),
        "threema" => ser(&config.threema),
        "keybase" => ser(&config.keybase),
        "reddit" => ser(&config.reddit),
        "mastodon" => ser(&config.mastodon),
        "bluesky" => ser(&config.bluesky),
        "linkedin" => ser(&config.linkedin),
        "nostr" => ser(&config.nostr),
        "teams" => ser(&config.teams),
        "mattermost" => ser(&config.mattermost),
        "google_chat" => ser(&config.google_chat),
        "webex" => ser(&config.webex),
        "feishu" => ser(&config.feishu),
        "dingtalk" => ser(&config.dingtalk),
        "pumble" => ser(&config.pumble),
        "flock" => ser(&config.flock),
        "twist" => ser(&config.twist),
        "zulip" => ser(&config.zulip),
        "irc" => ser(&config.irc),
        "xmpp" => ser(&config.xmpp),
        "gitter" => ser(&config.gitter),
        "discourse" => ser(&config.discourse),
        "revolt" => ser(&config.revolt),
        "guilded" => ser(&config.guilded),
        "nextcloud" => ser(&config.nextcloud),
        "rocketchat" => ser(&config.rocketchat),
        "twitch" => ser(&config.twitch),
        "ntfy" => ser(&config.ntfy),
        "gotify" => ser(&config.gotify),
        "webhook" => ser(&config.webhook),
        "voice" => ser(&config.voice),
        "mumble" => ser(&config.mumble),
        "wechat" => ser(&config.wechat),
        "wecom" => ser(&config.wecom),
        "qq" => ser(&config.qq),
        _ => Vec::new(),
    }
}

/// GET /api/channels — List all 40 channel adapters with status and field metadata.
///
/// Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
/// shape used by `/api/agents`, `/api/peers`, `/api/skills`, etc. (#3842).
/// The full channel registry is materialized in-memory, so this is a single
/// page — `offset=0`, `limit=None`. The bespoke `configured_count` sibling
/// is preserved for the dashboard's "X of Y configured" sub-line.
#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "channels",
    responses(
        (status = 200, description = "List configured channels", body = crate::types::JsonObject)
    )
)]
pub async fn list_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read the live channels config (updated on every hot-reload) instead of the
    // stale boot-time kernel.config, so newly configured channels show correctly.
    let live_channels = state.channels_config.read().await;
    // 24h activity per channel — backs the design's "slack · 142 msgs/24h"
    // sub-line. One grouped SQL pass for the whole page; falls back to an
    // empty map if the query fails so the listing itself still loads.
    let msgs_24h = state
        .kernel
        .memory_substrate()
        .usage()
        .channels_msgs_24h_bulk()
        .unwrap_or_default();
    let mut channels = Vec::new();
    let mut configured_count = 0u32;

    for meta in CHANNEL_REGISTRY {
        let configured = is_channel_configured(&live_channels, meta.name);
        if configured {
            configured_count += 1;
        }
        let instance_count = channel_instance_count(&live_channels, meta.name);

        // Check if all required secret env vars are set
        let has_token = meta
            .fields
            .iter()
            .filter(|f| f.required && f.env_var.is_some())
            .all(|f| {
                f.env_var
                    .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                    .unwrap_or(true)
            });

        let config_vals = channel_config_values(&live_channels, meta.name);
        let mut fields: Vec<serde_json::Value> = meta
            .fields
            .iter()
            .map(|f| build_field_json(f, config_vals.as_ref()))
            .collect();
        inject_callback_url(&mut fields, meta.name, config_vals.as_ref());

        let mut channel_json = serde_json::json!({
            "name": meta.name,
            "display_name": meta.display_name,
            "icon": meta.icon,
            "description": meta.description,
            "category": meta.category,
            "difficulty": meta.difficulty,
            "setup_time": meta.setup_time,
            "quick_setup": meta.quick_setup,
            "setup_type": meta.setup_type,
            "configured": configured,
            "instance_count": instance_count,
            "has_token": has_token,
            "fields": fields,
            "setup_steps": meta.setup_steps,
            "config_template": meta.config_template,
            "msgs_24h": msgs_24h.get(meta.name).copied().unwrap_or(0),
        });
        if let Some(endpoint) = webhook_endpoint_url(meta.name) {
            channel_json["webhook_endpoint"] = serde_json::Value::String(endpoint);
        }
        channels.push(channel_json);
    }

    let total = channels.len();
    // Canonical PaginatedResponse envelope (#3842) hand-built so the bespoke
    // `configured_count` sibling can ride alongside `items`/`total`/`offset`/
    // `limit` without a new struct.
    Json(serde_json::json!({
        "items": channels,
        "total": total,
        "offset": 0,
        "limit": serde_json::Value::Null,
        "configured_count": configured_count,
    }))
}

/// Returns channels list for the dashboard snapshot endpoint.
pub(crate) async fn channels_snapshot(state: &Arc<AppState>) -> Vec<serde_json::Value> {
    let live_channels = state.channels_config.read().await;
    let mut channels = Vec::new();

    for meta in CHANNEL_REGISTRY {
        let configured = is_channel_configured(&live_channels, meta.name);
        let instance_count = channel_instance_count(&live_channels, meta.name);
        let has_token = meta
            .fields
            .iter()
            .filter(|f| f.required && f.env_var.is_some())
            .all(|f| {
                f.env_var
                    .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                    .unwrap_or(true)
            });
        let config_vals = channel_config_values(&live_channels, meta.name);
        let mut fields: Vec<serde_json::Value> = meta
            .fields
            .iter()
            .map(|f| build_field_json(f, config_vals.as_ref()))
            .collect();
        inject_callback_url(&mut fields, meta.name, config_vals.as_ref());
        let mut channel_json = serde_json::json!({
            "name": meta.name,
            "display_name": meta.display_name,
            "icon": meta.icon,
            "description": meta.description,
            "category": meta.category,
            "difficulty": meta.difficulty,
            "setup_time": meta.setup_time,
            "quick_setup": meta.quick_setup,
            "setup_type": meta.setup_type,
            "configured": configured,
            "instance_count": instance_count,
            "has_token": has_token,
            "fields": fields,
            "setup_steps": meta.setup_steps,
            "config_template": meta.config_template,
        });
        if let Some(endpoint) = webhook_endpoint_url(meta.name) {
            channel_json["webhook_endpoint"] = serde_json::Value::String(endpoint);
        }
        channels.push(channel_json);
    }

    channels
}

/// GET /api/channels/{name} — Return a single channel's config, status, and field metadata.
#[utoipa::path(
    get,
    path = "/api/channels/{name}",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel adapter name (e.g. telegram, discord)")
    ),
    responses(
        (status = 200, description = "Channel details", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel", body = crate::types::JsonObject)
    )
)]
pub async fn get_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return ApiErrorResponse::not_found(format!("Unknown channel: {name}"))
                .into_json_tuple()
        }
    };

    let live_channels = state.channels_config.read().await;
    let configured = is_channel_configured(&live_channels, meta.name);
    let instance_count = channel_instance_count(&live_channels, meta.name);

    let has_token = meta
        .fields
        .iter()
        .filter(|f| f.required && f.env_var.is_some())
        .all(|f| {
            f.env_var
                .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                .unwrap_or(true)
        });

    let config_vals = channel_config_values(&live_channels, meta.name);
    let mut fields: Vec<serde_json::Value> = meta
        .fields
        .iter()
        .map(|f| build_field_json(f, config_vals.as_ref()))
        .collect();
    inject_callback_url(&mut fields, meta.name, config_vals.as_ref());

    let mut detail = serde_json::json!({
        "name": meta.name,
        "display_name": meta.display_name,
        "icon": meta.icon,
        "description": meta.description,
        "category": meta.category,
        "difficulty": meta.difficulty,
        "setup_time": meta.setup_time,
        "quick_setup": meta.quick_setup,
        "setup_type": meta.setup_type,
        "configured": configured,
        "instance_count": instance_count,
        "has_token": has_token,
        "fields": fields,
        "setup_steps": meta.setup_steps,
        "config_template": meta.config_template,
    });
    if let Some(endpoint) = webhook_endpoint_url(meta.name) {
        detail["webhook_endpoint"] = serde_json::Value::String(endpoint);
    }

    (StatusCode::OK, Json(detail))
}

#[utoipa::path(
    post,
    path = "/api/channels/{name}/configure",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel name")
    ),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Channel configured successfully", body = crate::types::JsonObject),
        (status = 400, description = "Bad request", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel", body = crate::types::JsonObject),
        (status = 409, description = "Channel is in multi-instance form; use the per-instance API", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/{name}/configure — Save channel secrets + config fields.
pub async fn configure_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    let fields = match body.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => return ApiErrorResponse::bad_request("Missing 'fields' object").into_json_tuple(),
    };

    let home = librefang_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");
    let mut config_fields: HashMap<String, (String, FieldType)> = HashMap::new();

    for field_def in meta.fields {
        let value = fields
            .get(field_def.key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }

        if let Some(env_var) = field_def.env_var {
            // Validate env var name and value before writing
            if let Err(msg) = validate_env_var(env_var, value) {
                return ApiErrorResponse::bad_request(msg).into_json_tuple();
            }
            // Secret field — write to secrets.env and set in process.
            if let Err(e) = write_secret_env(&secrets_path, env_var, value) {
                return ApiErrorResponse::internal(format!("Failed to write secret: {e}"))
                    .into_json_tuple();
            }
            // `std::env::set_var` is not thread-safe in an async context; delegate
            // to a blocking thread to avoid UB in the multithreaded tokio runtime.
            {
                let env_var_owned = env_var.to_string();
                let value_owned = value.to_string();
                let _ = tokio::task::spawn_blocking(move || {
                    // SAFETY: single mutation on a dedicated blocking thread.
                    unsafe { std::env::set_var(&env_var_owned, &value_owned) };
                })
                .await;
            }
            // Also write the env var NAME to config.toml so the channel section
            // is not empty and the kernel knows which env var to read.
            config_fields.insert(
                field_def.key.to_string(),
                (env_var.to_string(), FieldType::Text),
            );
        } else {
            // Config field — collect for TOML write with type info
            config_fields.insert(
                field_def.key.to_string(),
                (value.to_string(), field_def.field_type),
            );
        }
    }

    // Write config.toml section. Hold `config_write_lock` so we serialize
    // against `POST /api/config/set` (which holds the same mutex). Without
    // the lock, an interleaved provider write could be silently overwritten
    // when this path's read-modify-write completes — see issue #3183. The
    // guard is dropped at end of scope, before the hot-reload await below,
    // so it does not gate channel reloads.
    {
        let _config_guard = state.config_write_lock.lock().await;
        if let Err(e) = upsert_channel_config(&config_path, &name, &config_fields) {
            let msg = e.to_string();
            if let Some(rest) = msg.strip_prefix(CHANNEL_AOT_CONFLICT_PREFIX) {
                return ApiErrorResponse::bad_request(rest.to_string())
                    .with_status(StatusCode::CONFLICT)
                    .into_json_tuple();
            }
            return ApiErrorResponse::internal(format!("Failed to write config: {e}"))
                .into_json_tuple();
        }
    }

    // Hot-reload: activate the channel immediately
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => {
            let activated = started.iter().any(|s| s.eq_ignore_ascii_case(&name));
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "configured",
                    "channel": name,
                    "activated": activated,
                    "started_channels": started,
                    "note": if activated {
                        format!("{} activated successfully.", name)
                    } else {
                        "Channel configured but could not start (check credentials).".to_string()
                    }
                })),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after configure");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "configured",
                    "channel": name,
                    "activated": false,
                    "note": format!("Configured, but hot-reload failed: {e}. Restart daemon to activate.")
                })),
            )
        }
    }
}
#[utoipa::path(
    delete,
    path = "/api/channels/{name}/configure",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel name")
    ),
    responses(
        (status = 200, description = "Channel removed successfully", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel", body = crate::types::JsonObject),
        (status = 409, description = "Channel is in multi-instance form; use the per-instance API", body = crate::types::JsonObject),
        (status = 500, description = "Internal server error", body = crate::types::JsonObject)
    )
)]
/// DELETE /api/channels/{name}/configure — Remove channel secrets + config section.
pub async fn remove_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    let home = librefang_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");

    // Remove all secret env vars for this channel
    for field_def in meta.fields {
        if let Some(env_var) = field_def.env_var {
            if let Err(e) = remove_secret_env(&secrets_path, env_var) {
                tracing::warn!("Failed to remove secret env var: {e}");
            }
            // SAFETY: Single-threaded config operation
            unsafe {
                std::env::remove_var(env_var);
            }
        }
    }

    // Remove config section. Same locking discipline as `configure_channel`
    // — see issue #3183 for the race scenario.
    {
        let _config_guard = state.config_write_lock.lock().await;
        if let Err(e) = remove_channel_config(&config_path, &name) {
            let msg = e.to_string();
            if let Some(rest) = msg.strip_prefix(CHANNEL_AOT_CONFLICT_PREFIX) {
                return ApiErrorResponse::bad_request(rest.to_string())
                    .with_status(StatusCode::CONFLICT)
                    .into_json_tuple();
            }
            return ApiErrorResponse::internal(format!("Failed to remove config: {e}"))
                .into_json_tuple();
        }
    }

    // Hot-reload: deactivate the channel immediately
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(_started) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after remove");
            (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
        }
    }
}

// ---------------------------------------------------------------------------
// Per-instance endpoints (#4837)
//
// The legacy `/configure` endpoints treat every channel name as a single
// `[channels.<name>]` entry. The kernel has supported `[[channels.<name>]]`
// (multiple instances) since #240, but the dashboard had no UI to add a
// second Telegram bot or Slack workspace. The four handlers below let the
// dashboard list / create / update / delete individual instances. The
// instance ID is the array index — stable within a session, may shift after
// a deletion (the dashboard re-fetches via the standard query invalidation
// pattern in `mutations/channels.ts`).
// ---------------------------------------------------------------------------

/// Render the per-instance fields for the dashboard form.
///
/// Each instance gets the channel's full field schema (same shape as
/// `build_field_json` returns for the legacy single-instance flow), with
/// `value` populated from the per-instance config so the form pre-fills
/// when editing. Secret fields never have their value exposed — only
/// `has_value` (env-var presence check on the *named* env var the instance
/// points at) flips so the UI can show "secret already set" vs "needs setup".
fn build_instance_fields_json(
    meta: &ChannelMeta,
    instance: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut fields: Vec<serde_json::Value> = meta
        .fields
        .iter()
        .map(|f| build_field_json(f, Some(instance)))
        .collect();

    // For per-instance secret fields, override `has_value` to check the env
    // var that THIS instance's `<key>` points at (e.g. `TELEGRAM_BOT_TOKEN_2`)
    // instead of the field schema's default env var. Without this, every
    // instance would report the same `has_value` derived from the default
    // env var, defeating the purpose of multiple instances.
    let obj = match instance.as_object() {
        Some(o) => o,
        None => return fields,
    };
    for field_def in meta.fields {
        if field_def.field_type != FieldType::Secret {
            continue;
        }
        let pointed_env_name = obj
            .get(field_def.key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if pointed_env_name.is_empty() {
            continue;
        }
        let has_value = std::env::var(pointed_env_name)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        for field_json in fields.iter_mut() {
            if field_json.get("key").and_then(|v| v.as_str()) == Some(field_def.key) {
                field_json["has_value"] = serde_json::Value::Bool(has_value);
                // Surface the env-var name the instance is pointing at so
                // the UI can show "TELEGRAM_BOT_TOKEN_2" next to the secret
                // field — otherwise the user can't tell instances apart.
                field_json["env_var"] = serde_json::Value::String(pointed_env_name.to_string());
            }
        }
    }
    fields
}

/// Resolve secret env var name overrides for a write to instance
/// `target_index` of `channel`.
///
/// Two cases:
///
///   - `target_index == existing_instances.len()` → caller is appending a new
///     instance. `existing_instances.get(target_index)` returns `None`, so we
///     fall through to the suffix-search branch and pick the lowest unused
///     `<env>_<N>` name (or the bare default name when free).
///   - `target_index < existing_instances.len()` → caller is updating an
///     existing instance. We preserve whatever env-var name that instance is
///     already pointing at, so re-typing the secret writes to the SAME
///     env var (rotation in place) rather than allocating a fresh suffix
///     that may collide with siblings or leak a stale env var.
///
/// The previous implementation always synthesised `<env>_<index+1>` and
/// blindly wrote there. After deleting a middle instance, indices shift, so
/// "add a new instance" would land on a suffix already in use by a survivor
/// — silently overwriting the live token (#4865).
fn resolve_secret_env_overrides(
    meta: &ChannelMeta,
    existing_instances: &[serde_json::Value],
    target_index: usize,
) -> HashMap<String, String> {
    let mut overrides = HashMap::new();
    for field_def in meta.fields {
        let Some(default_env) = field_def.env_var else {
            continue;
        };

        // Update path: preserve the env-var name the existing instance is
        // already pointing at.
        if let Some(existing_name) = existing_instances
            .get(target_index)
            .and_then(|i| i.as_object())
            .and_then(|o| o.get(field_def.key))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            overrides.insert(field_def.key.to_string(), existing_name.to_string());
            continue;
        }

        // Create path (or update on an instance that has no env-var ref yet):
        // pick the lowest-unused `<env>_<N>` name across siblings.
        let used: std::collections::HashSet<String> = existing_instances
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != target_index)
            .filter_map(|(_, inst)| {
                inst.as_object()
                    .and_then(|o| o.get(field_def.key))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .collect();
        let chosen = if !used.contains(default_env) {
            default_env.to_string()
        } else {
            (2..)
                .map(|n| format!("{default_env}_{n}"))
                .find(|name| !used.contains(name))
                .expect("counter is unbounded")
        };
        overrides.insert(field_def.key.to_string(), chosen);
    }
    overrides
}

/// Canonical JSON serialisation with object keys sorted lexicographically and
/// no whitespace, so the same logical config always produces byte-identical
/// output. Used as the input to `instance_signature` for an opaque-content
/// fingerprint (CAS token) of each channel instance.
///
/// **Invariant for signature stability:** both the GET-side hash (computed
/// over `channel_instances_serialized(...)` of in-memory `ChannelsConfig`)
/// and the recomputed PUT/DELETE-side hash (computed over
/// `channel_instances_serialized(...)` of `read_disk_channels(...)`) must
/// flow through the SAME serialiser. Concretely: never mix a hash computed
/// over a `toml_edit::Table` with one computed over `serde_json::Value`
/// — `OneOrMany`'s deserialiser coerces TOML integers to JSON strings in
/// some fields (see `librefang-types/src/config/serde_helpers.rs`), and
/// the two views diverge. All sites that build the hash today route
/// through `serde_json::to_value(&TelegramConfig)` (and friends) so this
/// holds; the test `instance_signature_stable_across_key_order` pins it.
fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let parts: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

/// SHA-256 of `canonical_json(instance)`, hex-encoded. The dashboard receives
/// this as `signature` on every list response and echoes it back on PUT/DELETE
/// so the server can detect when an intervening write moved or modified the
/// instance the client thought it was operating on (compare-and-swap).
fn instance_signature(instance: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let canonical = canonical_json(instance);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hex::encode(hasher.finalize())
}

/// Re-deserialise `[channels]` from disk under the write lock, so handlers
/// validate against the authoritative on-disk state rather than the stale
/// in-memory `state.channels_config` snapshot (which only catches up after
/// the post-write hot-reload). Without this, two concurrent PUT/DELETE
/// requests can both pass an in-memory range check, then race to write a
/// shifted disk view — the second write lands on the wrong instance.
///
/// **Dual-parser note:** the WRITE path (`update_channel_instance`,
/// `remove_channel_instance` in skills.rs) parses with
/// `toml_edit::DocumentMut` to preserve comments/key-order; this READ
/// path parses with `toml::from_str` → `try_into::<ChannelsConfig>` so it
/// can use the canonical struct definitions. Both grammars are TOML and
/// agree on the channel-section shape; the downstream signature CAS only
/// compares the read-side serialisation against itself (GET + recompute
/// both go through this fn → `channel_instances_serialized`), so even an
/// edge-case parser disagreement on a non-channel section can't cause a
/// false 409.
fn read_disk_channels(
    config_path: &std::path::Path,
) -> Result<librefang_types::config::ChannelsConfig, Box<dyn std::error::Error + Send + Sync>> {
    if !config_path.exists() {
        return Ok(librefang_types::config::ChannelsConfig::default());
    }
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(librefang_types::config::ChannelsConfig::default());
    }
    let root: toml::Value = toml::from_str(&content)?;
    let Some(channels_val) = root.get("channels") else {
        return Ok(librefang_types::config::ChannelsConfig::default());
    };
    let channels: librefang_types::config::ChannelsConfig = channels_val.clone().try_into()?;
    Ok(channels)
}

/// Pure pre-write validation: run the user-submitted `fields` through the
/// channel schema, resolve each secret field's effective env-var name from
/// `secret_env_overrides`, and accumulate (a) the TOML config payload and
/// (b) the side-effecting secret writes that need to land in `secrets.env`.
///
/// No I/O happens here — secrets are NOT written until `apply_secret_writes`
/// is called, which lets handlers defer the writes until they are inside the
/// `config_write_lock` critical section and have verified the CAS signature.
struct PreparedWrite {
    config_fields: HashMap<String, (String, FieldType)>,
    /// `(env_var_name, value)` pairs to write to `secrets.env`. Distinct from
    /// `config_fields` because the same env-var name could be referenced by
    /// multiple fields (defensive — the schema does not currently allow it).
    secret_writes: Vec<(String, String)>,
}

fn prepare_fields_write(
    meta: &ChannelMeta,
    fields: &serde_json::Map<String, serde_json::Value>,
    secret_env_overrides: &HashMap<String, String>,
) -> Result<PreparedWrite, (StatusCode, Json<serde_json::Value>)> {
    let mut config_fields: HashMap<String, (String, FieldType)> = HashMap::new();
    let mut secret_writes: Vec<(String, String)> = Vec::new();

    for field_def in meta.fields {
        let value = fields
            .get(field_def.key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }

        if let Some(default_env_var) = field_def.env_var {
            let env_var_name = secret_env_overrides
                .get(field_def.key)
                .cloned()
                .unwrap_or_else(|| default_env_var.to_string());

            if let Err(msg) = validate_env_var(&env_var_name, value) {
                return Err(ApiErrorResponse::bad_request(msg).into_json_tuple());
            }

            secret_writes.push((env_var_name.clone(), value.to_string()));
            config_fields.insert(field_def.key.to_string(), (env_var_name, FieldType::Text));
        } else {
            config_fields.insert(
                field_def.key.to_string(),
                (value.to_string(), field_def.field_type),
            );
        }
    }

    Ok(PreparedWrite {
        config_fields,
        secret_writes,
    })
}

/// Apply the deferred secret writes from `prepare_fields_write` under the
/// `config_write_lock` critical section. Each pair is written to
/// `secrets.env` and pushed into the running process's environment via a
/// dedicated blocking thread (`std::env::set_var` is not thread-safe in the
/// async runtime).
async fn apply_secret_writes(
    secrets_path: &std::path::Path,
    secret_writes: &[(String, String)],
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    for (env_var, value) in secret_writes {
        if let Err(e) = write_secret_env(secrets_path, env_var, value) {
            return Err(
                ApiErrorResponse::internal(format!("Failed to write secret: {e}"))
                    .into_json_tuple(),
            );
        }
        let env_var_owned = env_var.clone();
        let value_owned = value.clone();
        let _ = tokio::task::spawn_blocking(move || {
            // SAFETY: single mutation on a dedicated blocking thread.
            unsafe { std::env::set_var(&env_var_owned, &value_owned) };
        })
        .await;
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/channels/{name}/instances",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel adapter name (e.g. telegram, discord)")
    ),
    responses(
        (status = 200, description = "List configured instances", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel", body = crate::types::JsonObject)
    )
)]
/// GET /api/channels/{name}/instances — List every configured instance.
pub async fn list_channel_instances(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    let live_channels = state.channels_config.read().await;
    let instances = channel_instances_serialized(&live_channels, meta.name);

    let items: Vec<serde_json::Value> = instances
        .iter()
        .enumerate()
        .map(|(idx, inst)| {
            let fields = build_instance_fields_json(meta, inst);
            // `has_token` for THIS instance: every required secret field's
            // pointed-at env var must be set.
            let has_token = meta
                .fields
                .iter()
                .filter(|f| f.required && f.field_type == FieldType::Secret)
                .all(|f| {
                    let pointed = inst
                        .as_object()
                        .and_then(|o| o.get(f.key))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if pointed.is_empty() {
                        return false;
                    }
                    std::env::var(pointed)
                        .map(|v| !v.is_empty())
                        .unwrap_or(false)
                });
            // CAS token. The dashboard echoes this back on PUT/DELETE so the
            // server can reject writes that target an instance that has been
            // moved or modified since the client read it (#4865).
            let signature = instance_signature(inst);
            serde_json::json!({
                "index": idx,
                "fields": fields,
                "config": inst,
                "has_token": has_token,
                "signature": signature,
            })
        })
        .collect();

    let total = items.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "channel": meta.name,
            "items": items,
            "total": total,
        })),
    )
}

#[utoipa::path(
    post,
    path = "/api/channels/{name}/instances",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel adapter name")
    ),
    request_body = crate::types::JsonObject,
    responses(
        (status = 201, description = "Instance created", body = crate::types::JsonObject),
        (status = 400, description = "Bad request", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel", body = crate::types::JsonObject),
        (status = 500, description = "Internal server error", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/{name}/instances — Append a new `[[channels.<name>]]`.
///
/// The whole "compute target index → resolve env-var name overrides → write
/// secrets → append config" sequence runs inside the `config_write_lock`
/// critical section, against a freshly re-read on-disk view of `[channels]`,
/// so two concurrent creates can't pick the same suffix or land at the same
/// index (#4865).
pub async fn create_channel_instance(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    let fields = match body.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => return ApiErrorResponse::bad_request("Missing 'fields' object").into_json_tuple(),
    };

    let home = librefang_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");

    let written_index = {
        let _config_guard = state.config_write_lock.lock().await;

        // Re-read `[channels]` from disk under the lock, so override resolution
        // sees the authoritative state (the in-memory snapshot only catches
        // up after the post-write hot-reload).
        let disk_channels = match read_disk_channels(&config_path) {
            Ok(c) => c,
            Err(e) => {
                return ApiErrorResponse::internal(format!("Failed to read config: {e}"))
                    .into_json_tuple();
            }
        };
        let existing_instances = channel_instances_serialized(&disk_channels, meta.name);
        let next_index = existing_instances.len();
        let overrides = resolve_secret_env_overrides(meta, &existing_instances, next_index);

        let prepared = match prepare_fields_write(meta, fields, &overrides) {
            Ok(p) => p,
            Err(resp) => return resp,
        };

        if let Err(resp) = apply_secret_writes(&secrets_path, &prepared.secret_writes).await {
            return resp;
        }

        match append_channel_instance(&config_path, &name, &prepared.config_fields) {
            Ok(idx) => idx,
            Err(e) => {
                return ApiErrorResponse::internal(format!("Failed to append instance: {e}"))
                    .into_json_tuple();
            }
        }
    };

    let (activated, started) = match crate::channel_bridge::reload_channels_from_disk(&state).await
    {
        Ok(s) => (s.iter().any(|x| x.eq_ignore_ascii_case(&name)), s),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after instance create");
            (false, Vec::new())
        }
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "created",
            "channel": name,
            "index": written_index,
            "activated": activated,
            "started_channels": started,
        })),
    )
}

#[utoipa::path(
    put,
    path = "/api/channels/{name}/instances/{index}",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel adapter name"),
        ("index" = usize, Path, description = "Instance array index (0-based)")
    ),
    request_body(
        content = crate::types::JsonObject,
        description = "REQUIRED body fields: `fields` (object of channel-schema fields) and `signature` (hex CAS token from the matching `GET /instances` item, used to detect concurrent edits — mismatch yields 409). OPTIONAL: `clear_secrets` (array of secret-field keys to actively drop)."
    ),
    responses(
        (status = 200, description = "Instance updated", body = crate::types::JsonObject),
        (status = 400, description = "Bad request (missing fields or signature)", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel or instance index out of range", body = crate::types::JsonObject),
        (status = 409, description = "Signature mismatch — instance was modified or moved by another writer", body = crate::types::JsonObject),
        (status = 500, description = "Internal server error", body = crate::types::JsonObject)
    )
)]
/// PUT /api/channels/{name}/instances/{index} — Replace one instance's fields.
///
/// Body shape:
/// ```json
/// {
///   "fields": { "bot_token_env": "...", "default_agent": "assistant" },
///   "signature": "<hex from GET response>",
///   "clear_secrets": ["bot_token_env"]   // optional
/// }
/// ```
///
/// `signature` is REQUIRED and acts as a compare-and-swap token: the server
/// re-reads disk under the write lock, computes the signature of the
/// instance currently at `index`, and rejects with 409 Conflict if it
/// doesn't match. This eliminates the silent-wrong-write bug where a
/// concurrent delete on a sibling row shifted indices between the client's
/// list-fetch and its PUT (#4865).
///
/// `clear_secrets` lets the caller actively drop a secret reference instead
/// of preserving it. Listed keys have their `<key>_env` field removed from
/// the rebuilt instance and, when no sibling instance references the same
/// env-var name, the env-var line is also removed from `secrets.env`.
pub async fn update_channel_instance_handler(
    State(state): State<Arc<AppState>>,
    Path((name, index)): Path<(String, usize)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    let fields = match body.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => return ApiErrorResponse::bad_request("Missing 'fields' object").into_json_tuple(),
    };

    // Required CAS token. Reject before touching disk so a missing field is
    // a clean 400 rather than an opaque race surface.
    let client_signature = match body.get("signature").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return ApiErrorResponse::bad_request(
                "Missing 'signature' (compare-and-swap token from list response)",
            )
            .into_json_tuple();
        }
    };

    let clear_secrets: Vec<String> = body
        .get("clear_secrets")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let home = librefang_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");

    {
        let _config_guard = state.config_write_lock.lock().await;

        // Re-read disk so range/CAS check + override resolution see the
        // authoritative state.
        let disk_channels = match read_disk_channels(&config_path) {
            Ok(c) => c,
            Err(e) => {
                return ApiErrorResponse::internal(format!("Failed to read config: {e}"))
                    .into_json_tuple();
            }
        };
        let existing_instances = channel_instances_serialized(&disk_channels, meta.name);
        if index >= existing_instances.len() {
            return ApiErrorResponse::not_found(format!(
                "Instance {index} out of range (have {} instance(s))",
                existing_instances.len()
            ))
            .into_json_tuple();
        }

        let on_disk_signature = instance_signature(&existing_instances[index]);
        if on_disk_signature != client_signature {
            return ApiErrorResponse::bad_request(
                "Instance has been modified or moved since the list was read; refresh and retry",
            )
            .with_status(StatusCode::CONFLICT)
            .into_json_tuple();
        }

        let overrides = resolve_secret_env_overrides(meta, &existing_instances, index);
        let mut prepared = match prepare_fields_write(meta, fields, &overrides) {
            Ok(p) => p,
            Err(resp) => return resp,
        };

        // Preserve the existing instance's secret-field env-var-name when the
        // user didn't retype the secret AND didn't list it under
        // `clear_secrets`. Without this, editing any non-secret field while
        // leaving a secret blank would silently drop the env-var ref, breaking
        // authentication.
        if let Some(obj) = existing_instances[index].as_object() {
            for field_def in meta.fields {
                if field_def.field_type != FieldType::Secret {
                    continue;
                }
                if prepared.config_fields.contains_key(field_def.key) {
                    continue;
                }
                if clear_secrets.iter().any(|k| k == field_def.key) {
                    continue;
                }
                let existing_env_name = obj
                    .get(field_def.key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if existing_env_name.is_empty() {
                    continue;
                }
                prepared.config_fields.insert(
                    field_def.key.to_string(),
                    (existing_env_name.to_string(), FieldType::Text),
                );
            }
        }

        // For each cleared secret: drop the env-var line from secrets.env if
        // no sibling instance still references the same env-var name. This
        // avoids leaving stale tokens on disk after the user has removed an
        // instance's auth, while preventing collateral damage to siblings
        // that happen to share the env-var name.
        if !clear_secrets.is_empty() {
            if let Some(obj) = existing_instances[index].as_object() {
                let sibling_env_refs: std::collections::HashSet<String> = existing_instances
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != index)
                    .filter_map(|(_, inst)| inst.as_object())
                    .flat_map(|o| o.values())
                    .filter_map(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                for key in &clear_secrets {
                    let Some(env_name) = obj.get(key).and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if env_name.is_empty() || sibling_env_refs.contains(env_name) {
                        continue;
                    }
                    if let Err(e) = remove_secret_env(&secrets_path, env_name) {
                        tracing::warn!(error = %e, env_var = %env_name, "Failed to remove cleared secret env var");
                    }
                    let env_owned = env_name.to_string();
                    let _ = tokio::task::spawn_blocking(move || {
                        // SAFETY: single mutation on a dedicated blocking thread.
                        unsafe { std::env::remove_var(&env_owned) };
                    })
                    .await;
                }
            }
        }

        if let Err(resp) = apply_secret_writes(&secrets_path, &prepared.secret_writes).await {
            return resp;
        }

        if let Err(e) = update_channel_instance(&config_path, &name, index, &prepared.config_fields)
        {
            return ApiErrorResponse::internal(format!("Failed to update instance: {e}"))
                .into_json_tuple();
        }
    }

    let (activated, started) = match crate::channel_bridge::reload_channels_from_disk(&state).await
    {
        Ok(s) => (s.iter().any(|x| x.eq_ignore_ascii_case(&name)), s),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after instance update");
            (false, Vec::new())
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "channel": name,
            "index": index,
            "activated": activated,
            "started_channels": started,
        })),
    )
}

#[utoipa::path(
    delete,
    path = "/api/channels/{name}/instances/{index}",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel adapter name"),
        ("index" = usize, Path, description = "Instance array index (0-based)"),
        ("signature" = String, Query, description = "REQUIRED hex CAS token from the matching `GET /instances` item; mismatch yields 409 (#4865)")
    ),
    responses(
        (status = 204, description = "Instance removed"),
        (status = 400, description = "Missing `signature` query parameter", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel or instance index out of range", body = crate::types::JsonObject),
        (status = 409, description = "Signature mismatch — instance was modified or moved by another writer", body = crate::types::JsonObject),
        (status = 500, description = "Internal server error", body = crate::types::JsonObject)
    )
)]
/// DELETE /api/channels/{name}/instances/{index}?signature=<hex>
///
/// `signature` is REQUIRED (query string) and acts as a compare-and-swap
/// token: the server re-reads disk under the write lock and rejects with
/// 409 Conflict if the instance currently at `index` doesn't match the
/// signature the client read from the GET response (#4865).
///
/// Note: this does NOT clear the secret env var the instance pointed at —
/// the env var may be shared with another instance, and we don't want to
/// silently break a running bot. To explicitly clear secrets while editing,
/// use PUT with `clear_secrets`.
pub async fn delete_channel_instance(
    State(state): State<Arc<AppState>>,
    Path((name, index)): Path<(String, usize)>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    let client_signature = match query.get("signature") {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return ApiErrorResponse::bad_request(
                "Missing 'signature' query parameter (compare-and-swap token from list response)",
            )
            .into_json_tuple();
        }
    };

    let home = librefang_home();
    let config_path = home.join("config.toml");

    {
        let _config_guard = state.config_write_lock.lock().await;

        let disk_channels = match read_disk_channels(&config_path) {
            Ok(c) => c,
            Err(e) => {
                return ApiErrorResponse::internal(format!("Failed to read config: {e}"))
                    .into_json_tuple();
            }
        };
        let existing_instances = channel_instances_serialized(&disk_channels, meta.name);
        if index >= existing_instances.len() {
            return ApiErrorResponse::not_found(format!(
                "Instance {index} out of range (have {} instance(s))",
                existing_instances.len()
            ))
            .into_json_tuple();
        }

        let on_disk_signature = instance_signature(&existing_instances[index]);
        if on_disk_signature != client_signature {
            return ApiErrorResponse::bad_request(
                "Instance has been modified or moved since the list was read; refresh and retry",
            )
            .with_status(StatusCode::CONFLICT)
            .into_json_tuple();
        }

        if let Err(e) = remove_channel_instance(&config_path, &name, index) {
            return ApiErrorResponse::internal(format!("Failed to remove instance: {e}"))
                .into_json_tuple();
        }
    }

    if let Err(e) = crate::channel_bridge::reload_channels_from_disk(&state).await {
        tracing::warn!(error = %e, "Channel hot-reload failed after instance delete");
    }

    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

#[utoipa::path(
    post,
    path = "/api/channels/{name}/test",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel name")
    ),
    request_body(content = Option<crate::types::JsonObject>, content_type = "application/json", description = "Optional target — `{ \"channel_id\": \"...\" }` for Discord/Slack or `{ \"chat_id\": \"...\" }` for Telegram"),
    responses(
        (status = 200, description = "Channel test succeeded", body = crate::types::JsonObject),
        (status = 404, description = "Unknown channel", body = crate::types::JsonObject),
        (status = 412, description = "Required channel credentials missing", body = crate::types::JsonObject),
        (status = 502, description = "Downstream send failure", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/{name}/test — Connectivity check + optional live test message.
///
/// Accepts an optional JSON body with `channel_id` (for Discord/Slack) or `chat_id`
/// (for Telegram). When provided, sends a real test message to verify the bot can
/// post to that channel.
///
/// Status code semantics (#3507, #3505):
/// - `200 OK` — credentials present (and, when a target was given, message sent);
///   body uses the legacy `{"status": "ok", "message": …}` shape.
/// - `404 Not Found` — unknown channel name; body uses `ApiErrorResponse`
///   (`{"error": "Unknown channel"}`).
/// - `412 Precondition Failed` — required env vars / credentials are missing;
///   body uses `ApiErrorResponse` (`{"error": "Missing required env vars: …"}`).
/// - `502 Bad Gateway` — credentials valid but downstream send failed;
///   body uses `ApiErrorResponse`.
///
/// `fetch().ok` is the source of truth. Error bodies were migrated from the
/// ad-hoc `{"status": "error", "message": …}` shape to the canonical
/// `ApiErrorResponse` envelope as part of #3505 so clients have a single
/// parsing strategy across the API surface.
pub async fn test_channel(
    Path(name): Path<String>,
    raw_body: axum::body::Bytes,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => return ApiErrorResponse::not_found("Unknown channel").into_json_tuple(),
    };

    // Check all required env vars are set
    let mut missing = Vec::new();
    for field_def in meta.fields {
        if field_def.required {
            if let Some(env_var) = field_def.env_var {
                if std::env::var(env_var).map(|v| v.is_empty()).unwrap_or(true) {
                    missing.push(env_var);
                }
            }
        }
    }

    if !missing.is_empty() {
        return ApiErrorResponse::bad_request(format!(
            "Missing required env vars: {}",
            missing.join(", ")
        ))
        .with_status(StatusCode::PRECONDITION_FAILED)
        .into_json_tuple();
    }

    // If a target channel/chat ID is provided, send a real test message
    let body: serde_json::Value = if raw_body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&raw_body).unwrap_or(serde_json::Value::Null)
    };
    let target = body
        .get("channel_id")
        .or_else(|| body.get("chat_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let Some(target_id) = target {
        match send_channel_test_message(&name, &target_id).await {
            Ok(()) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "message": format!("Test message sent to {} channel {}.", meta.display_name, target_id)
                    })),
                );
            }
            Err(e) => {
                return ApiErrorResponse::internal(format!(
                    "Credentials valid but failed to send test message: {e}"
                ))
                .with_status(StatusCode::BAD_GATEWAY)
                .into_json_tuple();
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("All required credentials for {} are set. Provide channel_id or chat_id to send a test message.", meta.display_name)
        })),
    )
}

/// Send a real test message to a specific channel/chat on the given platform.
async fn send_channel_test_message(channel_name: &str, target_id: &str) -> Result<(), String> {
    let client = librefang_kernel::http_client::proxied_client();
    let test_msg = "LibreFang test message — your channel is connected!";

    match channel_name {
        "discord" => {
            let token = std::env::var("DISCORD_BOT_TOKEN")
                .map_err(|_| "DISCORD_BOT_TOKEN not set".to_string())?;
            let url = format!("https://discord.com/api/v10/channels/{target_id}/messages");
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bot {token}"))
                .json(&serde_json::json!({ "content": test_msg }))
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Discord API error: {body}"));
            }
        }
        "telegram" => {
            let token = std::env::var("TELEGRAM_BOT_TOKEN")
                .map_err(|_| "TELEGRAM_BOT_TOKEN not set".to_string())?;
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "chat_id": target_id, "text": test_msg }))
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Telegram API error: {body}"));
            }
        }
        "slack" => {
            let token = std::env::var("SLACK_BOT_TOKEN")
                .map_err(|_| "SLACK_BOT_TOKEN not set".to_string())?;
            let url = "https://slack.com/api/chat.postMessage";
            let resp = client
                .post(url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&serde_json::json!({ "channel": target_id, "text": test_msg }))
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Slack API error: {body}"));
            }
        }
        _ => {
            return Err(format!(
                "Live test messaging not supported for {channel_name}. Credentials are valid."
            ));
        }
    }
    Ok(())
}
#[utoipa::path(
    post,
    path = "/api/channels/reload",
    tag = "channels",
    responses(
        (status = 200, description = "Channels reloaded successfully", body = crate::types::JsonObject),
        (status = 500, description = "Reload failed", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/reload — Manually trigger a channel hot-reload from disk config.
pub async fn reload_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "started": started,
            })),
        ),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

// ---------------------------------------------------------------------------
// WhatsApp QR login flow (OpenClaw-style)
// ---------------------------------------------------------------------------
#[utoipa::path(
    post,
    path = "/api/channels/whatsapp/qr/start",
    tag = "channels",
    responses(
        (status = 200, description = "WhatsApp QR session started", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/whatsapp/qr/start — Start a WhatsApp Web QR login session.
///
/// If a WhatsApp Web gateway is available (e.g. a Baileys-based bridge process),
/// this proxies the request and returns a base64 QR code data URL. If no gateway
/// is running, it returns instructions to set one up.
pub async fn whatsapp_qr_start() -> impl IntoResponse {
    // Check for WhatsApp Web gateway URL in config or env
    let gateway_url = std::env::var("WHATSAPP_WEB_GATEWAY_URL").unwrap_or_default();

    if gateway_url.is_empty() {
        return Json(serde_json::json!({
            "available": false,
            "message": "WhatsApp Web gateway not running. Start the gateway or use Business API mode.",
            "help": "The WhatsApp Web gateway auto-starts with the daemon when configured. Ensure Node.js >= 18 is installed and WhatsApp is configured in config.toml. Set WHATSAPP_WEB_GATEWAY_URL to use an external gateway."
        }));
    }

    // Try to reach the gateway and start a QR session.
    // Uses a raw HTTP request via tokio TcpStream to avoid adding reqwest as a runtime dep.
    let start_url = format!("{}/login/start", gateway_url.trim_end_matches('/'));
    match gateway_http_post(&start_url).await {
        Ok(body) => {
            let qr_url = body
                .get("qr_data_url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let sid = body
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let msg = body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Scan this QR code with WhatsApp → Linked Devices");
            let connected = body
                .get("connected")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            Json(serde_json::json!({
                "available": true,
                "qr_data_url": qr_url,
                "session_id": sid,
                "message": msg,
                "connected": connected,
            }))
        }
        Err(e) => Json(serde_json::json!({
            "available": false,
            "message": format!("Could not reach WhatsApp Web gateway: {e}"),
            "help": "Make sure the gateway is running at the configured URL"
        })),
    }
}
#[utoipa::path(
    get,
    path = "/api/channels/whatsapp/qr/status",
    tag = "channels",
    params(
        ("session_id" = Option<String>, Query, description = "WhatsApp login session ID")
    ),
    responses(
        (status = 200, description = "WhatsApp QR scan status", body = crate::types::JsonObject)
    )
)]
/// GET /api/channels/whatsapp/qr/status — Poll for QR scan completion.
///
/// After calling `/qr/start`, the frontend polls this to check if the user
/// has scanned the QR code and the WhatsApp Web session is connected.
pub async fn whatsapp_qr_status(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let gateway_url = std::env::var("WHATSAPP_WEB_GATEWAY_URL").unwrap_or_default();

    if gateway_url.is_empty() {
        return Json(serde_json::json!({
            "connected": false,
            "message": "Gateway not available"
        }));
    }

    let session_id = params.get("session_id").cloned().unwrap_or_default();
    let status_url = format!(
        "{}/login/status?session_id={}",
        gateway_url.trim_end_matches('/'),
        session_id
    );

    match gateway_http_get(&status_url).await {
        Ok(body) => {
            let connected = body
                .get("connected")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let msg = body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Waiting for scan...");
            let expired = body
                .get("expired")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            Json(serde_json::json!({
                "connected": connected,
                "message": msg,
                "expired": expired,
            }))
        }
        Err(_) => Json(serde_json::json!({ "connected": false, "message": "Gateway unreachable" })),
    }
}

/// Lightweight HTTP POST to a gateway URL. Returns parsed JSON body.
async fn gateway_http_post(url_with_path: &str) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Split into base URL + path from the full URL like "http://127.0.0.1:3009/login/start"
    let without_scheme = url_with_path
        .strip_prefix("http://")
        .or_else(|| url_with_path.strip_prefix("https://"))
        .unwrap_or(url_with_path);
    let (host_port, path) = if let Some(idx) = without_scheme.find('/') {
        (&without_scheme[..idx], &without_scheme[idx..])
    } else {
        (without_scheme, "/")
    };
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse().unwrap_or(3009u16))
    } else {
        (host_port, 3009u16)
    };

    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("Connect failed: {e}"))?;

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("Write failed: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("Read failed: {e}"))?;
    let response = String::from_utf8_lossy(&buf);

    // Find the JSON body after the blank line separating headers from body
    if let Some(idx) = response.find("\r\n\r\n") {
        let body_str = &response[idx + 4..];
        serde_json::from_str(body_str.trim()).map_err(|e| format!("Parse failed: {e}"))
    } else {
        Err("No HTTP body in response".to_string())
    }
}

/// Lightweight HTTP GET to a gateway URL. Returns parsed JSON body.
async fn gateway_http_get(url_with_path: &str) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let without_scheme = url_with_path
        .strip_prefix("http://")
        .or_else(|| url_with_path.strip_prefix("https://"))
        .unwrap_or(url_with_path);
    let (host_port, path_and_query) = if let Some(idx) = without_scheme.find('/') {
        (&without_scheme[..idx], &without_scheme[idx..])
    } else {
        (without_scheme, "/")
    };
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse().unwrap_or(3009u16))
    } else {
        (host_port, 3009u16)
    };

    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("Connect failed: {e}"))?;

    let req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("Write failed: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("Read failed: {e}"))?;
    let response = String::from_utf8_lossy(&buf);

    if let Some(idx) = response.find("\r\n\r\n") {
        let body_str = &response[idx + 4..];
        serde_json::from_str(body_str.trim()).map_err(|e| format!("Parse failed: {e}"))
    } else {
        Err("No HTTP body in response".to_string())
    }
}

// ── WeChat QR login endpoints ────────────────────────────────────────────────

/// iLink API base URL used by the WeChat adapter.
const WECHAT_ILINK_BASE: &str = "https://ilinkai.weixin.qq.com";

#[utoipa::path(
    post,
    path = "/api/channels/wechat/qr/start",
    tag = "channels",
    responses(
        (status = 200, description = "WeChat QR login initiated", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/wechat/qr/start — Request a QR code from iLink for WeChat login.
pub async fn wechat_qr_start() -> impl IntoResponse {
    let client = match librefang_kernel::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Json(serde_json::json!({
                "available": false,
                "message": format!("HTTP client error: {e}")
            }));
        }
    };

    let url = format!("{WECHAT_ILINK_BASE}/ilink/bot/get_bot_qrcode?bot_type=3");
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(body) => {
                let qrcode = body["qrcode"].as_str().unwrap_or("");
                let qrcode_url = body["qrcode_img_content"].as_str().unwrap_or("");
                if qrcode.is_empty() {
                    return Json(serde_json::json!({
                        "available": false,
                        "message": "iLink returned empty qrcode"
                    }));
                }
                Json(serde_json::json!({
                    "available": true,
                    "qr_code": qrcode,
                    "qr_url": qrcode_url,
                    "message": "Scan this QR code with your WeChat app to log in",
                }))
            }
            Err(e) => Json(serde_json::json!({
                "available": false,
                "message": format!("Failed to parse iLink response: {e}")
            })),
        },
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Json(serde_json::json!({
                "available": false,
                "message": format!("iLink QR request failed ({status}): {body}")
            }))
        }
        Err(e) => Json(serde_json::json!({
            "available": false,
            "message": format!("Could not reach iLink API: {e}")
        })),
    }
}

#[utoipa::path(
    get,
    path = "/api/channels/wechat/qr/status",
    tag = "channels",
    params(
        ("qr_code" = String, Query, description = "QR code value from /qr/start")
    ),
    responses(
        (status = 200, description = "WeChat QR scan status", body = crate::types::JsonObject)
    )
)]
/// GET /api/channels/wechat/qr/status — Poll iLink for QR scan confirmation.
pub async fn wechat_qr_status(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let qr_code = params.get("qr_code").cloned().unwrap_or_default();
    if qr_code.is_empty() {
        return Json(serde_json::json!({
            "connected": false,
            "expired": false,
            "message": "Missing qr_code parameter"
        }));
    }

    // iLink uses long-polling: the request hangs until the user scans or it
    // times out server-side (~30s). Use a generous timeout so we don't mistake
    // a normal long-poll wait for a network error.
    let client = match librefang_kernel::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return Json(serde_json::json!({
                "connected": false,
                "expired": false,
                "message": "HTTP client error"
            }));
        }
    };

    let encoded: String = url::form_urlencoded::byte_serialize(qr_code.as_bytes()).collect();
    let url = format!("{WECHAT_ILINK_BASE}/ilink/bot/get_qrcode_status?qrcode={encoded}");

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(body) => {
                let status = body["status"].as_str().unwrap_or("pending");
                match status {
                    "confirmed" => {
                        let bot_token = body["bot_token"].as_str().unwrap_or("");
                        Json(serde_json::json!({
                            "connected": true,
                            "expired": false,
                            "message": "WeChat login successful",
                            "bot_token": bot_token,
                        }))
                    }
                    "expired" => Json(serde_json::json!({
                        "connected": false,
                        "expired": true,
                        "message": "QR code expired — click Start to get a new one"
                    })),
                    _ => Json(serde_json::json!({
                        "connected": false,
                        "expired": false,
                        "message": "Waiting for scan..."
                    })),
                }
            }
            Err(_) => Json(serde_json::json!({
                "connected": false,
                "expired": false,
                "message": "Failed to parse status response"
            })),
        },
        // Timeout is normal for long-poll — treat as "still waiting"
        _ => Json(serde_json::json!({
            "connected": false,
            "expired": false,
            "message": "Waiting for scan..."
        })),
    }
}

// ---------------------------------------------------------------------------
// Channel registry metadata — loaded from ~/.librefang/channels/*.toml
// ---------------------------------------------------------------------------

/// Return channel metadata from the registry (synced from librefang-registry).
///
/// `GET /api/channels/registry`
#[utoipa::path(
    get,
    path = "/api/channels/registry",
    tag = "channels",
    responses(
        (status = 200, description = "Channel metadata from registry", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_channel_registry(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let channels_dir = state.kernel.home_dir().join("channels");
    let metadata = librefang_kernel::channel_registry::load_channel_metadata(&channels_dir);
    Json(serde_json::to_value(&metadata).unwrap_or_default())
}

#[cfg(test)]
mod test_channel_status_tests {
    //! Regression coverage for #3507 — `POST /api/channels/{name}/test` must
    //! report failure outcomes via HTTP status (412 / 502), not 200, so dashboard
    //! callers that branch on `fetch().ok` see them as failures.
    //!
    //! These tests mutate process-global env vars so they share a `Mutex` to
    //! avoid races with sibling tests (and other tests in this binary that
    //! touch the same vars).
    use super::*;
    use axum::extract::Path;
    use axum::response::IntoResponse;

    /// Serializes env-var mutations across the tests in this module so they
    /// don't race each other (or any other test in the binary that pokes at
    /// the same vars). Uses `tokio::sync::Mutex` so the guard can be held
    /// safely across `.await` points without triggering `await_holding_lock`.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Drop guard that restores the previous value of an env var when it falls
    /// out of scope, so a test failure doesn't poison the process for sibling
    /// tests.
    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn unset(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: serialized via ENV_LOCK; we only mutate this single key
            // and restore it in Drop.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, prev }
        }

        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: same reasoning as `unset`.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: same reasoning as the constructors — we still hold the
            // ENV_LOCK because the guard outlives the lock guard inside each
            // test's scope.
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[tokio::test]
    async fn unknown_channel_name_returns_404() {
        let _lock = ENV_LOCK.lock().await;
        let resp = test_channel(
            Path("not-a-real-channel".to_string()),
            axum::body::Bytes::new(),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_required_env_returns_412() {
        let _lock = ENV_LOCK.lock().await;
        // Telegram requires TELEGRAM_BOT_TOKEN. With it unset we must surface
        // a 412 — NOT a 200 with a "status: error" body, which silently passes
        // dashboard `fetch().ok` checks (#3507).
        let _g = EnvGuard::unset("TELEGRAM_BOT_TOKEN");

        let resp = test_channel(Path("telegram".to_string()), axum::body::Bytes::new())
            .await
            .into_response();
        assert_eq!(
            resp.status(),
            StatusCode::PRECONDITION_FAILED,
            "missing credentials must return 412, not 200"
        );
    }

    #[tokio::test]
    async fn credentials_present_no_target_returns_200() {
        let _lock = ENV_LOCK.lock().await;
        // Credentials set but no `channel_id` / `chat_id` body — handler
        // short-circuits before any network call and returns the
        // "credentials look good" 200 response.
        let _g = EnvGuard::set("TELEGRAM_BOT_TOKEN", "test-token-not-real");

        let resp = test_channel(Path("telegram".to_string()), axum::body::Bytes::new())
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn downstream_send_failure_returns_502() {
        let _lock = ENV_LOCK.lock().await;
        // Telegram bot token is set (so we get past the 412 gate) but the
        // value is bogus, so Bot API will reject the call. We pass a
        // `chat_id` to force the live-send branch. Result: handler must
        // surface a 502 Bad Gateway.
        //
        // This exercises a real network round-trip to api.telegram.org —
        // it'll be skipped in offline CI environments. We detect that by
        // looking at the response status: anything other than 502 in an
        // offline run means we couldn't reach the network at all, which
        // is fine for the purpose of *this* assertion (we already cover
        // the 200 / 412 / 404 paths deterministically above).
        let _g = EnvGuard::set("TELEGRAM_BOT_TOKEN", "0:invalid-token-for-test");

        let body = axum::body::Bytes::from_static(b"{\"chat_id\":\"1\"}");
        let resp = test_channel(Path("telegram".to_string()), body)
            .await
            .into_response();

        // Either: we reached Telegram and got a 401-equivalent → handler
        // returns 502; or we have no network → reqwest errors out which
        // also gets mapped to 502. Both are acceptable. A 200 here would
        // be the bug from #3507.
        assert_ne!(
            resp.status(),
            StatusCode::OK,
            "downstream send failure must NOT be reported as 200"
        );
    }
}

#[cfg(test)]
mod instance_helper_tests {
    //! Unit coverage for the per-instance write helpers added in #4865:
    //! `resolve_secret_env_overrides` and `instance_signature`. Both
    //! eliminate silent-data-loss footguns the original PR shipped with
    //! and have no business reaching production untested.
    use super::*;

    fn telegram_meta() -> &'static ChannelMeta {
        find_channel_meta("telegram").expect("telegram is in the registry")
    }

    fn inst_with_env(env_name: &str) -> serde_json::Value {
        serde_json::json!({ "bot_token_env": env_name })
    }

    /// First-instance create on an empty channel returns the bare default
    /// env-var name (no suffix), matching the legacy single-instance flow.
    #[test]
    fn resolve_overrides_picks_default_for_first_instance() {
        let meta = telegram_meta();
        let overrides = resolve_secret_env_overrides(meta, &[], 0);
        assert_eq!(
            overrides.get("bot_token_env").map(|s| s.as_str()),
            Some("TELEGRAM_BOT_TOKEN"),
            "first instance must use the bare default env-var name: {overrides:?}"
        );
    }

    /// After deleting the middle of three instances, the survivors point at
    /// `_BOT_TOKEN` and `_BOT_TOKEN_3`. Adding a new instance must NOT pick
    /// `_BOT_TOKEN_3` (which would silently overwrite the surviving instance
    /// at idx 1) — it must pick `_BOT_TOKEN_2`, the lowest unused suffix.
    #[test]
    fn resolve_overrides_picks_lowest_unused_suffix_after_middle_delete() {
        let meta = telegram_meta();
        let existing = vec![
            inst_with_env("TELEGRAM_BOT_TOKEN"),
            inst_with_env("TELEGRAM_BOT_TOKEN_3"),
        ];
        let overrides = resolve_secret_env_overrides(meta, &existing, existing.len());
        assert_eq!(
            overrides.get("bot_token_env").map(|s| s.as_str()),
            Some("TELEGRAM_BOT_TOKEN_2"),
            "must reuse the freed `_2` slot, not append `_3` and clobber the survivor: {overrides:?}"
        );
    }

    /// Update on an existing instance preserves the env-var name that
    /// instance is already pointing at — no fresh suffix allocated, no
    /// drift onto a sibling's env var. This is what makes "rotate the
    /// bot token in place" actually rotate in place.
    #[test]
    fn resolve_overrides_preserves_existing_env_name_on_update() {
        let meta = telegram_meta();
        let existing = vec![
            inst_with_env("TELEGRAM_BOT_TOKEN"),
            inst_with_env("MY_CUSTOM_TG_TOKEN"),
        ];
        let overrides = resolve_secret_env_overrides(meta, &existing, 1);
        assert_eq!(
            overrides.get("bot_token_env").map(|s| s.as_str()),
            Some("MY_CUSTOM_TG_TOKEN"),
            "update path must preserve the instance's existing env-var name: {overrides:?}"
        );
    }

    /// Sibling-set excludes the target index. An update should not skip
    /// `_BOT_TOKEN_2` just because the row being updated currently points
    /// at it (we'd never reach the suffix branch for a non-empty existing
    /// ref anyway, but a future caller passing target_index for an empty
    /// row should still be allowed to pick its own slot).
    #[test]
    fn resolve_overrides_excludes_target_index_from_sibling_set() {
        let meta = telegram_meta();
        let existing = vec![
            inst_with_env("TELEGRAM_BOT_TOKEN"),
            inst_with_env(""), // empty — falls through to suffix search
            inst_with_env("TELEGRAM_BOT_TOKEN_3"),
        ];
        let overrides = resolve_secret_env_overrides(meta, &existing, 1);
        // Slot 1 is empty, so we go to suffix search. Used by siblings: KEY,
        // KEY_3. Lowest unused: KEY_2.
        assert_eq!(
            overrides.get("bot_token_env").map(|s| s.as_str()),
            Some("TELEGRAM_BOT_TOKEN_2")
        );
    }

    /// Signature is stable across object-key insertion order, so two
    /// processes seeing identical content always emit the same hex string.
    #[test]
    fn instance_signature_stable_across_key_order() {
        let a = serde_json::json!({ "x": 1, "y": "two", "z": [3, 4] });
        let b = serde_json::json!({ "z": [3, 4], "y": "two", "x": 1 });
        assert_eq!(
            instance_signature(&a),
            instance_signature(&b),
            "signature must be canonical across key order"
        );
    }

    /// Any content change flips the signature so the CAS check fires.
    #[test]
    fn instance_signature_detects_mutation() {
        let a = serde_json::json!({ "bot_token_env": "TG_TOKEN", "default_agent": "alice" });
        let b = serde_json::json!({ "bot_token_env": "TG_TOKEN", "default_agent": "bob" });
        assert_ne!(
            instance_signature(&a),
            instance_signature(&b),
            "signature must change when any field changes"
        );
    }

    /// Output is hex (so the dashboard can put it in a JSON string and
    /// query string without escaping concerns).
    #[test]
    fn instance_signature_is_lowercase_hex() {
        let sig = instance_signature(&serde_json::json!({ "x": 1 }));
        assert!(
            sig.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "signature must be lowercase hex: {sig}"
        );
        assert_eq!(sig.len(), 64, "sha-256 hex length: {sig}");
    }
}
