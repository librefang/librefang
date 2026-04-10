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
        .route("/channels/{name}/test", axum::routing::post(test_channel))
        .route("/channels/reload", axum::routing::post(reload_channels))
        .route(
            "/channels/whatsapp/{instance_key}/bootstrap/start",
            axum::routing::post(whatsapp_bootstrap_start),
        )
        .route(
            "/channels/whatsapp/{instance_key}/bootstrap/status",
            axum::routing::get(whatsapp_bootstrap_status),
        )
        .route(
            "/channels/whatsapp/{instance_key}/bootstrap/cancel",
            axum::routing::post(whatsapp_bootstrap_cancel),
        )
        .route(
            "/channels/wechat/{instance_key}/bootstrap/start",
            axum::routing::post(wechat_bootstrap_start),
        )
        .route(
            "/channels/wechat/{instance_key}/bootstrap/status",
            axum::routing::get(wechat_bootstrap_status),
        )
        .route(
            "/channels/wechat/{instance_key}/bootstrap/cancel",
            axum::routing::post(wechat_bootstrap_cancel),
        )
        .route(
            "/channels/registry",
            axum::routing::get(list_channel_registry),
        )
}

use super::shared::require_admin;
use super::skills::{remove_secret_env, validate_env_var, write_secret_env};
use super::AppState;
use crate::middleware::AccountId;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

use crate::types::ApiErrorResponse;
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
fn is_channel_configured(
    config: &librefang_types::config::ChannelsConfig,
    name: &str,
    account_id: &str,
) -> bool {
    match name {
        "telegram" => config
            .telegram
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "discord" => config
            .discord
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "slack" => config
            .slack
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "whatsapp" => config
            .whatsapp
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "signal" => config
            .signal
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "matrix" => config
            .matrix
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "email" => config
            .email
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "line" => config
            .line
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "viber" => config
            .viber
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "messenger" => config
            .messenger
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "threema" => config
            .threema
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "keybase" => config
            .keybase
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "reddit" => config
            .reddit
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "mastodon" => config
            .mastodon
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "bluesky" => config
            .bluesky
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "linkedin" => config
            .linkedin
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "nostr" => config
            .nostr
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "teams" => config
            .teams
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "mattermost" => config
            .mattermost
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "google_chat" => config
            .google_chat
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "webex" => config
            .webex
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "feishu" => config
            .feishu
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "dingtalk" => config
            .dingtalk
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "pumble" => config
            .pumble
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "flock" => config
            .flock
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "twist" => config
            .twist
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "zulip" => config
            .zulip
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "irc" => config
            .irc
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "xmpp" => config
            .xmpp
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "gitter" => config
            .gitter
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "discourse" => config
            .discourse
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "revolt" => config
            .revolt
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "guilded" => config
            .guilded
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "nextcloud" => config
            .nextcloud
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "rocketchat" => config
            .rocketchat
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "twitch" => config
            .twitch
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "ntfy" => config
            .ntfy
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "gotify" => config
            .gotify
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "webhook" => config
            .webhook
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "voice" => config
            .voice
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "mumble" => config
            .mumble
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "wechat" => config
            .wechat
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "wecom" => config
            .wecom
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        "qq" => config
            .qq
            .iter()
            .any(|c| c.account_id.as_deref() == Some(account_id)),
        _ => false,
    }
}

fn require_tenant_account_id(account: &AccountId) -> Result<&str, axum::response::Response> {
    account.0.as_deref().ok_or_else(|| {
        ApiErrorResponse::bad_request("X-Account-Id header required for channel operations")
            .with_status(StatusCode::UNAUTHORIZED)
            .into_json_tuple()
            .into_response()
    })
}

fn encode_account_component(account_id: &str) -> String {
    if account_id.is_empty() {
        return "ACCOUNT_00".to_string();
    }

    let mut out = String::with_capacity(account_id.len() * 3);
    for byte in account_id.as_bytes() {
        out.push('_');
        out.push_str(&format!("{byte:02X}"));
    }
    format!("ACCOUNT{out}")
}

fn scoped_secret_env_var(base: &str, account_id: &str) -> String {
    format!("{base}__{}", encode_account_component(account_id))
}

fn account_id_from_toml_table(table: &toml::map::Map<String, toml::Value>) -> Option<&str> {
    table.get("account_id").and_then(|v| v.as_str())
}

fn build_channel_toml_table(
    fields: &HashMap<String, (String, FieldType)>,
    account_id: &str,
) -> toml::map::Map<String, toml::Value> {
    let mut ch_table = toml::map::Map::new();
    ch_table.insert(
        "account_id".to_string(),
        toml::Value::String(account_id.to_string()),
    );
    for (k, (v, ft)) in fields {
        let toml_val = match ft {
            FieldType::Number => {
                if let Ok(n) = v.parse::<i64>() {
                    toml::Value::Integer(n)
                } else {
                    toml::Value::String(v.clone())
                }
            }
            FieldType::List => toml::Value::Array(
                v.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| toml::Value::String(s.to_string()))
                    .collect(),
            ),
            _ => toml::Value::String(v.clone()),
        };
        ch_table.insert(k.clone(), toml_val);
    }
    ch_table
}

fn upsert_account_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    account_id: &str,
    fields: &HashMap<String, (String, FieldType)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };
    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("channels") {
        root.insert(
            "channels".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let channels_table = root
        .get_mut("channels")
        .and_then(|v| v.as_table_mut())
        .ok_or("channels is not a table")?;

    let new_entry = toml::Value::Table(build_channel_toml_table(fields, account_id));
    match channels_table.get_mut(channel_name) {
        Some(existing) if existing.is_table() => {
            let table = existing
                .as_table()
                .ok_or("channel config entry is not a TOML table")?;
            if account_id_from_toml_table(table) == Some(account_id) {
                *existing = new_entry;
            } else {
                let legacy = existing.clone();
                *existing = toml::Value::Array(vec![legacy, new_entry]);
            }
        }
        Some(toml::Value::Array(entries)) => {
            for entry in entries.iter_mut() {
                if let Some(table) = entry.as_table() {
                    if account_id_from_toml_table(table) == Some(account_id) {
                        *entry = new_entry.clone();
                        if let Some(parent) = config_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
                        return Ok(());
                    }
                }
            }
            entries.push(new_entry);
        }
        Some(_) => return Err("channel config entry is not a TOML table or array".into()),
        None => {
            channels_table.insert(channel_name.to_string(), new_entry);
        }
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

fn remove_account_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    account_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }

    let mut doc: toml::Value = toml::from_str(&content)?;
    if let Some(channels) = doc
        .as_table_mut()
        .and_then(|r| r.get_mut("channels"))
        .and_then(|c| c.as_table_mut())
    {
        let mut remove_key = false;
        if let Some(value) = channels.get_mut(channel_name) {
            match value {
                toml::Value::Table(table) => {
                    if account_id_from_toml_table(table) == Some(account_id) {
                        remove_key = true;
                    }
                }
                toml::Value::Array(entries) => {
                    entries.retain(|entry| {
                        entry
                            .as_table()
                            .map(|table| account_id_from_toml_table(table) != Some(account_id))
                            .unwrap_or(true)
                    });
                    match entries.len() {
                        0 => remove_key = true,
                        1 => *value = entries[0].clone(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if remove_key {
            channels.remove(channel_name);
        }
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

fn stored_secret_env_name_for_field(
    config_values: Option<&serde_json::Value>,
    field: &ChannelField,
) -> Option<String> {
    config_values
        .and_then(|v| v.as_object())
        .and_then(|obj| obj.get(field.key))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn configured_secret_env_names(
    meta: &ChannelMeta,
    config_values: Option<&serde_json::Value>,
) -> Vec<String> {
    meta.fields
        .iter()
        .filter_map(|field| stored_secret_env_name_for_field(config_values, field))
        .collect()
}

fn existing_field_value_as_string(
    config_values: &serde_json::Value,
    field: &ChannelField,
) -> Option<String> {
    let value = config_values.as_object()?.get(field.key)?;
    match field.field_type {
        FieldType::List => value.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| Some(item.to_string()))
                })
                .collect::<Vec<_>>()
                .join(", ")
        }),
        FieldType::Number => value
            .as_i64()
            .map(|v| v.to_string())
            .or_else(|| value.as_u64().map(|v| v.to_string()))
            .or_else(|| value.as_f64().map(|v| v.to_string()))
            .or_else(|| value.as_str().map(ToOwned::to_owned)),
        _ => value.as_str().map(ToOwned::to_owned),
    }
}

fn merged_channel_config_fields(
    meta: &ChannelMeta,
    existing_config: Option<&serde_json::Value>,
    request_fields: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, (String, FieldType)> {
    let mut merged = HashMap::new();

    if let Some(existing_config) = existing_config {
        for field in meta.fields {
            if let Some(value) = existing_field_value_as_string(existing_config, field) {
                merged.insert(field.key.to_string(), (value, field.field_type));
            }
        }
    }

    for field in meta.fields {
        let value = request_fields
            .get(field.key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }
        merged.insert(field.key.to_string(), (value.to_string(), field.field_type));
    }

    merged
}

/// Build a JSON field descriptor, checking env var presence but never exposing secrets.
/// For non-secret fields, includes the actual config value from `config_values` if available.
fn build_field_json(
    f: &ChannelField,
    config_values: Option<&serde_json::Value>,
) -> serde_json::Value {
    let has_value = stored_secret_env_name_for_field(config_values, f)
        .map(|ev| std::env::var(&ev).map(|v| !v.is_empty()).unwrap_or(false))
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
    account_id: &str,
) -> Option<serde_json::Value> {
    match name {
        "telegram" => config
            .telegram
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "discord" => config
            .discord
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "slack" => config
            .slack
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "whatsapp" => config
            .whatsapp
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "signal" => config
            .signal
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "matrix" => config
            .matrix
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "email" => config
            .email
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "teams" => config
            .teams
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "mattermost" => config
            .mattermost
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "irc" => config
            .irc
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "google_chat" => config
            .google_chat
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "twitch" => config
            .twitch
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "rocketchat" => config
            .rocketchat
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "zulip" => config
            .zulip
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "xmpp" => config
            .xmpp
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "line" => config
            .line
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "viber" => config
            .viber
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "messenger" => config
            .messenger
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "reddit" => config
            .reddit
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "mastodon" => config
            .mastodon
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "bluesky" => config
            .bluesky
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "feishu" => config
            .feishu
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "revolt" => config
            .revolt
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "nextcloud" => config
            .nextcloud
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "guilded" => config
            .guilded
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "keybase" => config
            .keybase
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "threema" => config
            .threema
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "nostr" => config
            .nostr
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "webex" => config
            .webex
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "pumble" => config
            .pumble
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "flock" => config
            .flock
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "twist" => config
            .twist
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "mumble" => config
            .mumble
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "dingtalk" => config
            .dingtalk
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "discourse" => config
            .discourse
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "gitter" => config
            .gitter
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "ntfy" => config
            .ntfy
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "gotify" => config
            .gotify
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "webhook" => config
            .webhook
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "voice" => config
            .voice
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "linkedin" => config
            .linkedin
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "wechat" => config
            .wechat
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "wecom" => config
            .wecom
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        "qq" => config
            .qq
            .iter()
            .find(|c| c.account_id.as_deref() == Some(account_id))
            .and_then(|c| serde_json::to_value(c).ok()),
        _ => None,
    }
}

/// GET /api/channels — List all 40 channel adapters with status and field metadata.
#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "channels",
    responses(
        (status = 200, description = "List configured channels", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_channels(
    State(state): State<Arc<AppState>>,
    account: AccountId,
) -> axum::response::Response {
    let account_id = match require_tenant_account_id(&account) {
        Ok(account_id) => account_id,
        Err(response) => return response,
    };
    // Read the live channels config (updated on every hot-reload) instead of the
    // stale boot-time kernel.config, so newly configured channels show correctly.
    let live_channels = state.channels_config.read().await;
    let mut channels = Vec::new();
    let mut configured_count = 0u32;

    for meta in CHANNEL_REGISTRY {
        let config_vals = channel_config_values(&live_channels, meta.name, account_id);
        let configured = is_channel_configured(&live_channels, meta.name, account_id);
        if configured {
            configured_count += 1;
        }

        // Check if all required secret env vars are set
        let has_token = meta
            .fields
            .iter()
            .filter(|f| f.required && f.env_var.is_some())
            .all(|f| {
                stored_secret_env_name_for_field(config_vals.as_ref(), f)
                    .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                    .unwrap_or(false)
            });

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

    Json(serde_json::json!({
        "channels": channels,
        "total": channels.len(),
        "configured_count": configured_count,
    }))
    .into_response()
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
        (status = 200, description = "Channel details", body = serde_json::Value),
        (status = 404, description = "Unknown channel", body = serde_json::Value)
    )
)]
pub async fn get_channel(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(name): Path<String>,
) -> axum::response::Response {
    let account_id = match require_tenant_account_id(&account) {
        Ok(account_id) => account_id,
        Err(response) => return response,
    };
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return ApiErrorResponse::not_found(format!("Unknown channel: {name}"))
                .into_json_tuple()
                .into_response()
        }
    };

    let live_channels = state.channels_config.read().await;
    let config_vals = channel_config_values(&live_channels, meta.name, account_id);
    let configured = is_channel_configured(&live_channels, meta.name, account_id);

    let has_token = meta
        .fields
        .iter()
        .filter(|f| f.required && f.env_var.is_some())
        .all(|f| {
            stored_secret_env_name_for_field(config_vals.as_ref(), f)
                .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                .unwrap_or(false)
        });

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
        "has_token": has_token,
        "fields": fields,
        "setup_steps": meta.setup_steps,
        "config_template": meta.config_template,
    });
    if let Some(endpoint) = webhook_endpoint_url(meta.name) {
        detail["webhook_endpoint"] = serde_json::Value::String(endpoint);
    }

    (StatusCode::OK, Json(detail)).into_response()
}

#[utoipa::path(
    post,
    path = "/api/channels/{name}/configure",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel name")
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Channel configured successfully", body = serde_json::Value),
        (status = 400, description = "Bad request", body = serde_json::Value),
        (status = 404, description = "Unknown channel", body = serde_json::Value)
    )
)]
/// POST /api/channels/{name}/configure — Save channel secrets + config fields.
pub async fn configure_channel(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = match require_tenant_account_id(&account) {
        Ok(account_id) => account_id,
        Err(response) => return response,
    };
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return ApiErrorResponse::not_found("Unknown channel")
                .into_json_tuple()
                .into_response()
        }
    };

    let fields = match body.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => {
            return ApiErrorResponse::bad_request("Missing 'fields' object")
                .into_json_tuple()
                .into_response()
        }
    };

    let home = state.kernel.home_dir().to_path_buf();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");
    let live_channels = state.channels_config.read().await;
    let existing_config = channel_config_values(&live_channels, meta.name, account_id);
    drop(live_channels);
    let mut config_fields = merged_channel_config_fields(meta, existing_config.as_ref(), fields);

    for field_def in meta.fields {
        let value = fields
            .get(field_def.key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }

        if let Some(env_var) = field_def.env_var {
            let scoped_env_var = scoped_secret_env_var(env_var, account_id);
            // Validate env var name and value before writing
            if let Err(msg) = validate_env_var(&scoped_env_var, value) {
                return ApiErrorResponse::bad_request(msg)
                    .into_json_tuple()
                    .into_response();
            }
            // Secret field — write to secrets.env and set in process
            if let Err(e) = write_secret_env(&secrets_path, &scoped_env_var, value) {
                return ApiErrorResponse::internal(format!("Failed to write secret: {e}"))
                    .into_json_tuple()
                    .into_response();
            }
            // SAFETY: We are the only writer; this is a single-threaded config operation
            unsafe {
                std::env::set_var(&scoped_env_var, value);
            }
            // Also write the env var NAME to config.toml so the channel section
            // is not empty and the kernel knows which env var to read.
            config_fields.insert(field_def.key.to_string(), (scoped_env_var, FieldType::Text));
        } else {
            // Non-secret fields are already merged from existing config above.
        }
    }

    // Write config.toml section
    if let Err(e) = upsert_account_channel_config(&config_path, &name, account_id, &config_fields) {
        return ApiErrorResponse::internal(format!("Failed to write config: {e}"))
            .into_json_tuple()
            .into_response();
    }

    // Hot-reload: activate the channel immediately
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => {
            let expected_started_key = format!("{name}:{account_id}");
            let activated = started
                .iter()
                .any(|started_name| started_name.eq_ignore_ascii_case(&expected_started_key));
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
                .into_response()
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
            ).into_response()
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
        (status = 200, description = "Channel removed successfully", body = serde_json::Value),
        (status = 404, description = "Unknown channel", body = serde_json::Value),
        (status = 500, description = "Internal server error", body = serde_json::Value)
    )
)]
/// DELETE /api/channels/{name}/configure — Remove channel secrets + config section.
pub async fn remove_channel(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(name): Path<String>,
) -> axum::response::Response {
    let account_id = match require_tenant_account_id(&account) {
        Ok(account_id) => account_id,
        Err(response) => return response,
    };
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return ApiErrorResponse::not_found("Unknown channel")
                .into_json_tuple()
                .into_response()
        }
    };

    let home = state.kernel.home_dir().to_path_buf();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");
    let live_channels = state.channels_config.read().await;
    let config_vals = channel_config_values(&live_channels, meta.name, account_id);

    // Remove only this account's secret env vars for this channel.
    for env_var in configured_secret_env_names(meta, config_vals.as_ref()) {
        if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
            tracing::warn!("Failed to remove secret env var: {e}");
        }
        // SAFETY: Single-threaded config operation
        unsafe {
            std::env::remove_var(&env_var);
        }
    }
    drop(live_channels);

    // Remove only this account's config entry.
    if let Err(e) = remove_account_channel_config(&config_path, &name, account_id) {
        return ApiErrorResponse::internal(format!("Failed to remove config: {e}"))
            .into_json_tuple()
            .into_response();
    }

    // Hot-reload: deactivate the channel immediately
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "removed",
                "channel": name,
                "remaining_channels": started,
                "note": format!("{} deactivated.", name)
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after remove");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "removed",
                    "channel": name,
                    "note": format!("Removed, but hot-reload failed: {e}. Restart daemon to fully deactivate.")
                })),
            ).into_response()
        }
    }
}
#[utoipa::path(
    post,
    path = "/api/channels/{name}/test",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel name")
    ),
    request_body(content = Option<serde_json::Value>, content_type = "application/json"),
    responses(
        (status = 200, description = "Channel test result", body = serde_json::Value),
        (status = 404, description = "Unknown channel", body = serde_json::Value)
    )
)]
/// POST /api/channels/{name}/test — Connectivity check + optional live test message.
///
/// Accepts an optional JSON body with `channel_id` (for Discord/Slack) or `chat_id`
/// (for Telegram). When provided, sends a real test message to verify the bot can
/// post to that channel.
pub async fn test_channel(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    raw_body: axum::body::Bytes,
) -> axum::response::Response {
    let account_id = match require_tenant_account_id(&account) {
        Ok(account_id) => account_id,
        Err(response) => return response,
    };
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"status": "error", "message": "Unknown channel"})),
            )
                .into_response()
        }
    };
    let live_channels = state.channels_config.read().await;
    let config_vals = channel_config_values(&live_channels, meta.name, account_id);
    drop(live_channels);

    // Check all required env vars are set
    let mut missing = Vec::new();
    for field_def in meta.fields {
        if field_def.required {
            match stored_secret_env_name_for_field(config_vals.as_ref(), field_def) {
                Some(env_var) => {
                    if std::env::var(&env_var)
                        .map(|v| v.is_empty())
                        .unwrap_or(true)
                    {
                        missing.push(env_var);
                    }
                }
                None if field_def.env_var.is_some() => missing.push(field_def.key.to_string()),
                None => {}
            }
        }
    }

    if !missing.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Missing required env vars: {}", missing.join(", "))
            })),
        )
            .into_response();
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
        match send_channel_test_message(&name, &target_id, config_vals.as_ref()).await {
            Ok(()) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "message": format!("Test message sent to {} channel {}.", meta.display_name, target_id)
                    })),
                ).into_response();
            }
            Err(e) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": format!("Credentials valid but failed to send test message: {e}")
                    })),
                )
                    .into_response();
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("All required credentials for {} are set. Provide channel_id or chat_id to send a test message.", meta.display_name)
        })),
    ).into_response()
}

/// Send a real test message to a specific channel/chat on the given platform.
async fn send_channel_test_message(
    channel_name: &str,
    target_id: &str,
    config_values: Option<&serde_json::Value>,
) -> Result<(), String> {
    let client = librefang_runtime::http_client::proxied_client();
    let test_msg = "LibreFang test message — your channel is connected!";

    match channel_name {
        "discord" => {
            let token_env = config_values
                .and_then(|v| v.get("bot_token_env"))
                .and_then(|v| v.as_str())
                .unwrap_or("DISCORD_BOT_TOKEN");
            let token = std::env::var(token_env).map_err(|_| format!("{token_env} not set"))?;
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
            let token_env = config_values
                .and_then(|v| v.get("bot_token_env"))
                .and_then(|v| v.as_str())
                .unwrap_or("TELEGRAM_BOT_TOKEN");
            let token = std::env::var(token_env).map_err(|_| format!("{token_env} not set"))?;
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
            let token_env = config_values
                .and_then(|v| v.get("bot_token_env"))
                .and_then(|v| v.as_str())
                .unwrap_or("SLACK_BOT_TOKEN");
            let token = std::env::var(token_env).map_err(|_| format!("{token_env} not set"))?;
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

#[cfg(test)]
mod tests {
    use super::{
        merged_channel_config_fields, remove_account_channel_config, router, scoped_secret_env_var,
        stored_secret_env_name_for_field, upsert_account_channel_config, ChannelField, FieldType,
    };
    use crate::routes::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use librefang_kernel::LibreFangKernel;
    use librefang_types::config::{
        ChannelsConfig, DefaultModelConfig, KernelConfig, WeChatConfig, WhatsAppConfig,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;
    use tower::ServiceExt;

    #[test]
    fn scoped_secret_env_var_encodes_account_id_without_collisions() {
        assert_eq!(
            scoped_secret_env_var("TELEGRAM_BOT_TOKEN", "acct-prod/us-east-1"),
            "TELEGRAM_BOT_TOKEN__ACCOUNT_61_63_63_74_2D_70_72_6F_64_2F_75_73_2D_65_61_73_74_2D_31"
        );
        assert_ne!(
            scoped_secret_env_var("TELEGRAM_BOT_TOKEN", "acct-prod/us-east-1"),
            scoped_secret_env_var("TELEGRAM_BOT_TOKEN", "acct_prod_us-east-1")
        );
    }

    #[test]
    fn upsert_account_channel_config_preserves_other_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut acct_a = HashMap::new();
        acct_a.insert(
            "bot_token_env".to_string(),
            ("TELEGRAM_BOT_TOKEN__A".to_string(), FieldType::Text),
        );
        acct_a.insert(
            "default_agent".to_string(),
            ("assistant-a".to_string(), FieldType::Text),
        );
        upsert_account_channel_config(&path, "telegram", "acct_a", &acct_a).unwrap();

        let mut acct_b = HashMap::new();
        acct_b.insert(
            "bot_token_env".to_string(),
            ("TELEGRAM_BOT_TOKEN__B".to_string(), FieldType::Text),
        );
        acct_b.insert(
            "default_agent".to_string(),
            ("assistant-b".to_string(), FieldType::Text),
        );
        upsert_account_channel_config(&path, "telegram", "acct_b", &acct_b).unwrap();

        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entries = parsed["channels"]["telegram"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["account_id"].as_str(), Some("acct_a"));
        assert_eq!(entries[1]["account_id"].as_str(), Some("acct_b"));
    }

    #[test]
    fn remove_account_channel_config_only_removes_target_account() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut acct_a = HashMap::new();
        acct_a.insert(
            "bot_token_env".to_string(),
            ("TELEGRAM_BOT_TOKEN__A".to_string(), FieldType::Text),
        );
        upsert_account_channel_config(&path, "telegram", "acct_a", &acct_a).unwrap();

        let mut acct_b = HashMap::new();
        acct_b.insert(
            "bot_token_env".to_string(),
            ("TELEGRAM_BOT_TOKEN__B".to_string(), FieldType::Text),
        );
        upsert_account_channel_config(&path, "telegram", "acct_b", &acct_b).unwrap();

        remove_account_channel_config(&path, "telegram", "acct_a").unwrap();

        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entry = &parsed["channels"]["telegram"];
        assert_eq!(entry["account_id"].as_str(), Some("acct_b"));
        assert_eq!(
            entry["bot_token_env"].as_str(),
            Some("TELEGRAM_BOT_TOKEN__B")
        );
    }

    #[test]
    fn stored_secret_env_name_requires_persisted_mapping() {
        let field = ChannelField {
            key: "bot_token_env",
            label: "Bot Token",
            field_type: FieldType::Secret,
            env_var: Some("TELEGRAM_BOT_TOKEN"),
            required: true,
            placeholder: "",
            advanced: false,
            options: None,
            show_when: None,
            readonly: false,
        };

        assert_eq!(stored_secret_env_name_for_field(None, &field), None);
    }

    #[test]
    fn partial_update_preserves_existing_scoped_secret_mapping() {
        let meta = super::find_channel_meta("telegram").unwrap();
        let existing = serde_json::json!({
            "account_id": "acct_a",
            "bot_token_env": "TELEGRAM_BOT_TOKEN__ACCT_A",
            "default_agent": "support"
        });
        let request = serde_json::json!({
            "default_agent": "ops"
        });

        let merged =
            merged_channel_config_fields(meta, Some(&existing), request.as_object().unwrap());

        assert_eq!(
            merged.get("bot_token_env").map(|v| v.0.as_str()),
            Some("TELEGRAM_BOT_TOKEN__ACCT_A")
        );
        assert_eq!(
            merged.get("default_agent").map(|v| v.0.as_str()),
            Some("ops")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wechat_bootstrap_start_persists_owned_session_for_target_instance() {
        let _guard = wechat_test_guard().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let ilink = spawn_wechat_ilink_stub(vec![(
            "/ilink/bot/get_bot_qrcode?bot_type=3".to_string(),
            StatusCode::OK,
            serde_json::json!({
                "qrcode": "provider-qr-handle",
                "qrcode_img_content": "https://stub.example/qr.png"
            }),
        )])
        .await;
        // SAFETY: test-scoped environment mutation.
        unsafe {
            std::env::set_var("LIBREFANG_WECHAT_ILINK_BASE", &ilink);
        }
        let state = build_wechat_test_state(temp.path(), "tenant-admin", "tenant-a").await;
        let app = router().with_state(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/wechat/wechat:tenant-a/bootstrap/start")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;

        assert_eq!(body["instance_key"].as_str(), Some("wechat:tenant-a"));
        assert_eq!(body["account_id"].as_str(), Some("tenant-a"));
        assert_eq!(body["status"].as_str(), Some("pending"));
        assert_eq!(body["qr_url"].as_str(), Some("https://stub.example/qr.png"));
        assert!(body.get("qr_code").is_none());

        let store = crate::channel_bootstrap::ChannelBootstrapStore::new(temp.path());
        store.load().unwrap();
        let session = store
            .get_pending_by_instance("wechat", "wechat:tenant-a")
            .await
            .unwrap();
        assert_eq!(session.account_id, "tenant-a");
        assert_eq!(
            session.provider_handle.as_deref(),
            Some("provider-qr-handle")
        );

        // SAFETY: test cleanup for process-global environment.
        unsafe {
            std::env::remove_var("LIBREFANG_WECHAT_ILINK_BASE");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wechat_bootstrap_status_confirms_owned_session_and_persists_token_to_owner_slot() {
        let _guard = wechat_test_guard().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let ilink = spawn_wechat_ilink_stub(vec![
            (
                "/ilink/bot/get_bot_qrcode?bot_type=3".to_string(),
                StatusCode::OK,
                serde_json::json!({
                    "qrcode": "provider-qr-handle",
                    "qrcode_img_content": "https://stub.example/qr.png"
                }),
            ),
            (
                "/ilink/bot/get_qrcode_status?qrcode=provider-qr-handle".to_string(),
                StatusCode::OK,
                serde_json::json!({
                    "status": "confirmed",
                    "bot_token": "ilink_bot_token_123"
                }),
            ),
        ])
        .await;
        // SAFETY: test-scoped environment mutation.
        unsafe {
            std::env::set_var("LIBREFANG_WECHAT_ILINK_BASE", &ilink);
        }
        let state = build_wechat_test_state(temp.path(), "tenant-admin", "tenant-a").await;
        let app = router().with_state(state.clone());

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/wechat/wechat:tenant-a/bootstrap/start")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/channels/wechat/wechat:tenant-a/bootstrap/status")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;

        assert_eq!(body["connected"].as_bool(), Some(true));
        assert_eq!(body["status"].as_str(), Some("confirmed"));
        assert!(body.get("bot_token").is_none());

        let store = crate::channel_bootstrap::ChannelBootstrapStore::new(temp.path());
        store.load().unwrap();
        let session = store
            .get_by_bootstrap_id(body["bootstrap_id"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            session.status,
            crate::channel_bootstrap::BootstrapStatus::Confirmed
        );

        let token_env = scoped_secret_env_var("WECHAT_BOT_TOKEN", "tenant-a");
        let secrets = std::fs::read_to_string(temp.path().join("secrets.env")).unwrap();
        assert!(secrets.contains(&token_env));
        assert!(secrets.contains("ilink_bot_token_123"));
        assert_eq!(
            std::env::var(&token_env).ok().as_deref(),
            Some("ilink_bot_token_123")
        );

        // SAFETY: test cleanup for process-global environment.
        unsafe {
            std::env::remove_var("LIBREFANG_WECHAT_ILINK_BASE");
            std::env::remove_var(&token_env);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wechat_bootstrap_cancel_marks_owned_session_cancelled() {
        let _guard = wechat_test_guard().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let ilink = spawn_wechat_ilink_stub(vec![(
            "/ilink/bot/get_bot_qrcode?bot_type=3".to_string(),
            StatusCode::OK,
            serde_json::json!({
                "qrcode": "provider-qr-handle",
                "qrcode_img_content": "https://stub.example/qr.png"
            }),
        )])
        .await;
        // SAFETY: test-scoped environment mutation.
        unsafe {
            std::env::set_var("LIBREFANG_WECHAT_ILINK_BASE", &ilink);
        }
        let state = build_wechat_test_state(temp.path(), "tenant-admin", "tenant-a").await;
        let app = router().with_state(state.clone());

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/wechat/wechat:tenant-a/bootstrap/start")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/wechat/wechat:tenant-a/bootstrap/cancel")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;
        assert_eq!(body["status"].as_str(), Some("cancelled"));

        let store = crate::channel_bootstrap::ChannelBootstrapStore::new(temp.path());
        store.load().unwrap();
        let session = store
            .get_by_bootstrap_id(body["bootstrap_id"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            session.status,
            crate::channel_bootstrap::BootstrapStatus::Cancelled
        );

        // SAFETY: test cleanup for process-global environment.
        unsafe {
            std::env::remove_var("LIBREFANG_WECHAT_ILINK_BASE");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whatsapp_bootstrap_start_persists_owned_session_for_target_instance() {
        let _guard = wechat_test_guard().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let gateway = spawn_gateway_stub(vec![(
            "POST".to_string(),
            "/login/start".to_string(),
            StatusCode::OK,
            serde_json::json!({
                "session_id": "wa-session-1",
                "qr_data_url": "data:image/png;base64,abc",
                "message": "Scan with WhatsApp",
                "connected": false
            }),
        )])
        .await;
        unsafe {
            std::env::set_var(
                "WHATSAPP_GATEWAY_URL__ACCOUNT_74_65_6E_61_6E_74_2D_61",
                &gateway,
            );
        }
        let state = build_whatsapp_test_state(temp.path(), "tenant-admin", "tenant-a").await;
        let app = router().with_state(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/whatsapp/whatsapp:tenant-a/bootstrap/start")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;
        assert_eq!(body["instance_key"].as_str(), Some("whatsapp:tenant-a"));
        assert_eq!(body["account_id"].as_str(), Some("tenant-a"));
        assert_eq!(body["status"].as_str(), Some("pending"));
        assert_eq!(body["qr_url"].as_str(), Some("data:image/png;base64,abc"));

        let store = crate::channel_bootstrap::ChannelBootstrapStore::new(temp.path());
        store.load().unwrap();
        let session = store
            .get_pending_by_instance("whatsapp", "whatsapp:tenant-a")
            .await
            .unwrap();
        assert_eq!(session.account_id, "tenant-a");
        assert_eq!(session.provider_handle.as_deref(), Some("wa-session-1"));

        unsafe {
            std::env::remove_var("WHATSAPP_GATEWAY_URL__ACCOUNT_74_65_6E_61_6E_74_2D_61");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whatsapp_bootstrap_status_confirms_owned_session() {
        let _guard = wechat_test_guard().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let gateway = spawn_gateway_stub(vec![
            (
                "POST".to_string(),
                "/login/start".to_string(),
                StatusCode::OK,
                serde_json::json!({
                    "session_id": "wa-session-2",
                    "qr_data_url": "data:image/png;base64,def",
                    "message": "Scan with WhatsApp",
                    "connected": false
                }),
            ),
            (
                "GET".to_string(),
                "/login/status?instance_key=whatsapp%3Atenant-a".to_string(),
                StatusCode::OK,
                serde_json::json!({
                    "instance_key": "whatsapp:tenant-a",
                    "connected": true,
                    "message": "Connected to WhatsApp",
                    "session_id": "wa-session-2",
                    "expired": false
                }),
            ),
        ])
        .await;
        unsafe {
            std::env::set_var(
                "WHATSAPP_GATEWAY_URL__ACCOUNT_74_65_6E_61_6E_74_2D_61",
                &gateway,
            );
        }
        let state = build_whatsapp_test_state(temp.path(), "tenant-admin", "tenant-a").await;
        let app = router().with_state(state.clone());

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/whatsapp/whatsapp:tenant-a/bootstrap/start")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/channels/whatsapp/whatsapp:tenant-a/bootstrap/status")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;
        assert_eq!(body["status"].as_str(), Some("confirmed"));
        assert_eq!(body["connected"].as_bool(), Some(true));

        let store = crate::channel_bootstrap::ChannelBootstrapStore::new(temp.path());
        store.load().unwrap();
        let session = store
            .get_latest_by_instance("whatsapp", "whatsapp:tenant-a")
            .await
            .unwrap();
        assert_eq!(
            session.status,
            crate::channel_bootstrap::BootstrapStatus::Confirmed
        );

        unsafe {
            std::env::remove_var("WHATSAPP_GATEWAY_URL__ACCOUNT_74_65_6E_61_6E_74_2D_61");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whatsapp_bootstrap_cancel_marks_owned_session_cancelled() {
        let _guard = wechat_test_guard().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let gateway = spawn_gateway_stub(vec![(
            "POST".to_string(),
            "/login/start".to_string(),
            StatusCode::OK,
            serde_json::json!({
                "session_id": "wa-session-3",
                "qr_data_url": "data:image/png;base64,ghi",
                "message": "Scan with WhatsApp",
                "connected": false
            }),
        )])
        .await;
        unsafe {
            std::env::set_var(
                "WHATSAPP_GATEWAY_URL__ACCOUNT_74_65_6E_61_6E_74_2D_61",
                &gateway,
            );
        }
        let state = build_whatsapp_test_state(temp.path(), "tenant-admin", "tenant-a").await;
        let app = router().with_state(state.clone());

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/whatsapp/whatsapp:tenant-a/bootstrap/start")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/channels/whatsapp/whatsapp:tenant-a/bootstrap/cancel")
                    .header("X-Account-Id", "tenant-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;
        assert_eq!(body["status"].as_str(), Some("cancelled"));
        assert_eq!(body["connected"].as_bool(), Some(false));

        unsafe {
            std::env::remove_var("WHATSAPP_GATEWAY_URL__ACCOUNT_74_65_6E_61_6E_74_2D_61");
        }
    }

    async fn read_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn wechat_test_guard() -> &'static Mutex<()> {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        GUARD.get_or_init(|| Mutex::new(()))
    }

    async fn build_wechat_test_state(
        home_dir: &std::path::Path,
        admin_account_id: &str,
        wechat_account_id: &str,
    ) -> Arc<AppState> {
        librefang_runtime::registry_sync::sync_registry(
            home_dir,
            librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
            "",
        );

        let mut config = KernelConfig {
            home_dir: home_dir.to_path_buf(),
            data_dir: home_dir.join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
                message_timeout_secs: 300,
            },
            ..KernelConfig::default()
        };
        config.admin_accounts = vec![admin_account_id.to_string()];
        std::fs::write(
            home_dir.join("config.toml"),
            toml::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        let kernel = Arc::new(LibreFangKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();

        let wechat_config = WeChatConfig {
            bot_token_env: scoped_secret_env_var("WECHAT_BOT_TOKEN", wechat_account_id),
            account_id: Some(wechat_account_id.to_string()),
            default_agent: Some("assistant".to_string()),
            ..WeChatConfig::default()
        };
        let mut channels = ChannelsConfig::default();
        channels.wechat.0.push(wechat_config);

        Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(channels),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(
                home_dir.join("test-webhooks.json"),
            ),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            #[cfg(feature = "telemetry")]
            prometheus_handle: None,
            account_sig_secret: None,
        })
    }

    async fn build_whatsapp_test_state(
        home_dir: &std::path::Path,
        admin_account_id: &str,
        whatsapp_account_id: &str,
    ) -> Arc<AppState> {
        librefang_runtime::registry_sync::sync_registry(
            home_dir,
            librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
            "",
        );

        let mut config = KernelConfig {
            home_dir: home_dir.to_path_buf(),
            data_dir: home_dir.join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
                message_timeout_secs: 300,
            },
            ..KernelConfig::default()
        };
        config.admin_accounts = vec![admin_account_id.to_string()];
        std::fs::write(
            home_dir.join("config.toml"),
            toml::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        let kernel = Arc::new(LibreFangKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();

        let whatsapp_config = WhatsAppConfig {
            gateway_url_env: scoped_secret_env_var("WHATSAPP_GATEWAY_URL", whatsapp_account_id),
            account_id: Some(whatsapp_account_id.to_string()),
            default_agent: Some("assistant".to_string()),
            ..WhatsAppConfig::default()
        };
        let mut channels = ChannelsConfig::default();
        channels.whatsapp.0.push(whatsapp_config);

        Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(channels),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(
                home_dir.join("test-webhooks.json"),
            ),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            #[cfg(feature = "telemetry")]
            prometheus_handle: None,
            account_sig_secret: None,
        })
    }

    async fn spawn_wechat_ilink_stub(
        routes: Vec<(String, StatusCode, serde_json::Value)>,
    ) -> String {
        let routes = Arc::new(routes);
        let app = axum::Router::new().fallback({
            let routes = routes.clone();
            axum::routing::any(move |uri: axum::http::Uri| {
                let routes = routes.clone();
                async move {
                    let path = uri
                        .path_and_query()
                        .map(|value| value.as_str())
                        .unwrap_or(uri.path());
                    let (status, body) = routes
                        .iter()
                        .find(|(expected, _, _)| expected == path)
                        .map(|(_, status, body)| (*status, body.clone()))
                        .unwrap_or_else(|| {
                            (
                                StatusCode::NOT_FOUND,
                                serde_json::json!({ "message": format!("unexpected path: {path}") }),
                            )
                        });
                    (status, axum::Json(body))
                }
            })
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    async fn spawn_gateway_stub(
        routes: Vec<(String, String, StatusCode, serde_json::Value)>,
    ) -> String {
        let routes = Arc::new(routes);
        let app = axum::Router::new().fallback({
            let routes = routes.clone();
            axum::routing::any(move |method: axum::http::Method, uri: axum::http::Uri| {
                let routes = routes.clone();
                async move {
                    let path = uri
                        .path_and_query()
                        .map(|value| value.as_str())
                        .unwrap_or(uri.path());
                    let (status, body) = routes
                        .iter()
                        .find(|(expected_method, expected_path, _, _)| {
                            expected_method == method.as_str() && expected_path == path
                        })
                        .map(|(_, _, status, body)| (*status, body.clone()))
                        .unwrap_or_else(|| {
                            (
                                StatusCode::NOT_FOUND,
                                serde_json::json!({
                                    "message": format!("unexpected route: {} {}", method, path)
                                }),
                            )
                        });
                    (status, axum::Json(body))
                }
            })
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }
}
#[utoipa::path(
    post,
    path = "/api/channels/reload",
    tag = "channels",
    responses(
        (status = 200, description = "Channels reloaded successfully", body = serde_json::Value),
        (status = 500, description = "Reload failed", body = serde_json::Value)
    )
)]
/// POST /api/channels/reload — Manually trigger a channel hot-reload from disk config.
pub async fn reload_channels(
    State(state): State<Arc<AppState>>,
    account: AccountId,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "started": started,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "error": e,
            })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// WhatsApp QR login flow (owned bootstrap)
// ---------------------------------------------------------------------------
#[utoipa::path(
    post,
    path = "/api/channels/whatsapp/{instance_key}/bootstrap/start",
    tag = "channels",
    params(
        ("instance_key" = String, Path, description = "Owned WhatsApp instance key, e.g. whatsapp:tenant-a")
    ),
    responses(
        (status = 200, description = "WhatsApp bootstrap session created", body = serde_json::Value),
        (status = 404, description = "Owned WhatsApp instance not found", body = serde_json::Value)
    )
)]
pub async fn whatsapp_bootstrap_start(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(instance_key): Path<String>,
) -> axum::response::Response {
    let created_by =
        match require_admin_account_id(&account, &state.kernel.config_ref().admin_accounts) {
            Ok(account_id) => account_id,
            Err(response) => return response,
        };
    let target = {
        let live_channels = state.channels_config.read().await;
        match resolve_whatsapp_bootstrap_target(&live_channels, &instance_key) {
            Some(target) => target,
            None => {
                return ApiErrorResponse::not_found("Owned WhatsApp instance not found")
                    .into_json_tuple()
                    .into_response()
            }
        }
    };

    let start_url = format!("{}/login/start", target.gateway_url.trim_end_matches('/'));
    let body = match gateway_http_post_json(
        &start_url,
        &serde_json::json!({ "instance_key": target.instance_key }),
    )
    .await
    {
        Ok(body) => body,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Could not reach WhatsApp gateway: {e}"))
                .into_json_tuple()
                .into_response()
        }
    };

    let provider_handle = body
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    if provider_handle.is_empty() {
        return ApiErrorResponse::internal("WhatsApp gateway returned empty session_id")
            .into_json_tuple()
            .into_response();
    }

    let now = chrono::Utc::now();
    let session = crate::channel_bootstrap::ChannelBootstrapSession {
        bootstrap_id: uuid::Uuid::new_v4().to_string(),
        channel_type: "whatsapp".to_string(),
        instance_key: target.instance_key.clone(),
        account_id: target.account_id.clone(),
        bootstrap_kind: crate::channel_bootstrap::BootstrapKind::QrLogin,
        provider_handle: Some(provider_handle.to_string()),
        provider_qr_payload: body
            .get("qr_data_url")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        provider_qr_url: body
            .get("qr_data_url")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        provider_pairing_code: None,
        status: crate::channel_bootstrap::BootstrapStatus::Pending,
        created_at: now,
        updated_at: now,
        expires_at: Some(now + chrono::Duration::minutes(5)),
        created_by,
        last_error: None,
    };
    let store = channel_bootstrap_store(state.kernel.home_dir());
    if let Err(e) = store.load() {
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    if let Err(e) = store.create(session.clone()).await {
        return ApiErrorResponse::bad_request(e)
            .into_json_tuple()
            .into_response();
    }

    (
        StatusCode::OK,
        Json(bootstrap_session_view(
            &session,
            body.get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Scan this QR code with WhatsApp → Linked Devices"),
            body.get("connected")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        )),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/channels/whatsapp/{instance_key}/bootstrap/status",
    tag = "channels",
    params(
        ("instance_key" = String, Path, description = "Owned WhatsApp instance key, e.g. whatsapp:tenant-a")
    ),
    responses(
        (status = 200, description = "WhatsApp bootstrap status", body = serde_json::Value),
        (status = 404, description = "Owned WhatsApp bootstrap session not found", body = serde_json::Value)
    )
)]
pub async fn whatsapp_bootstrap_status(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(instance_key): Path<String>,
) -> axum::response::Response {
    if let Err(response) =
        require_admin_account_id(&account, &state.kernel.config_ref().admin_accounts).map(|_| ())
    {
        return response;
    }
    let target = {
        let live_channels = state.channels_config.read().await;
        match resolve_whatsapp_bootstrap_target(&live_channels, &instance_key) {
            Some(target) => target,
            None => {
                return ApiErrorResponse::not_found("Owned WhatsApp instance not found")
                    .into_json_tuple()
                    .into_response()
            }
        }
    };
    let store = channel_bootstrap_store(state.kernel.home_dir());
    if let Err(e) = store.load() {
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    let session = match store
        .get_latest_by_instance("whatsapp", &target.instance_key)
        .await
    {
        Some(session) => session,
        None => {
            return ApiErrorResponse::not_found("Owned WhatsApp bootstrap session not found")
                .into_json_tuple()
                .into_response()
        }
    };

    if session.status != crate::channel_bootstrap::BootstrapStatus::Pending {
        let connected = session.status == crate::channel_bootstrap::BootstrapStatus::Confirmed;
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &session,
                "Owned bootstrap status loaded",
                connected,
            )),
        )
            .into_response();
    }

    if session
        .expires_at
        .map(|expires_at| chrono::Utc::now() >= expires_at)
        .unwrap_or(false)
    {
        let expired = match store
            .expire(&session.bootstrap_id, chrono::Utc::now())
            .await
        {
            Ok(expired) => expired,
            Err(e) => {
                return ApiErrorResponse::internal(e)
                    .into_json_tuple()
                    .into_response()
            }
        };
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &expired,
                "QR code expired — click Start to get a new one",
                false,
            )),
        )
            .into_response();
    }

    let status_url = format!(
        "{}/login/status?instance_key={}",
        target.gateway_url.trim_end_matches('/'),
        url::form_urlencoded::byte_serialize(target.instance_key.as_bytes()).collect::<String>()
    );
    let body = match gateway_http_get(&status_url).await {
        Ok(body) => body,
        Err(_) => {
            return (
                StatusCode::OK,
                Json(bootstrap_session_view(
                    &session,
                    "Waiting for scan...",
                    false,
                )),
            )
                .into_response()
        }
    };

    let connected = body
        .get("connected")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if connected {
        let confirmed = match store
            .confirm(&session.bootstrap_id, chrono::Utc::now())
            .await
        {
            Ok(confirmed) => confirmed,
            Err(e) => {
                return ApiErrorResponse::internal(e)
                    .into_json_tuple()
                    .into_response()
            }
        };
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &confirmed,
                body.get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("WhatsApp login successful"),
                true,
            )),
        )
            .into_response();
    }

    let expired = body
        .get("expired")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if expired {
        let expired = match store
            .expire(&session.bootstrap_id, chrono::Utc::now())
            .await
        {
            Ok(expired) => expired,
            Err(e) => {
                return ApiErrorResponse::internal(e)
                    .into_json_tuple()
                    .into_response()
            }
        };
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &expired,
                body.get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("QR code expired — click Start to get a new one"),
                false,
            )),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(bootstrap_session_view(
            &session,
            body.get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Waiting for scan..."),
            false,
        )),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/api/channels/whatsapp/{instance_key}/bootstrap/cancel",
    tag = "channels",
    params(
        ("instance_key" = String, Path, description = "Owned WhatsApp instance key, e.g. whatsapp:tenant-a")
    ),
    responses(
        (status = 200, description = "WhatsApp bootstrap session cancelled", body = serde_json::Value),
        (status = 404, description = "Owned WhatsApp bootstrap session not found", body = serde_json::Value)
    )
)]
pub async fn whatsapp_bootstrap_cancel(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(instance_key): Path<String>,
) -> axum::response::Response {
    if let Err(response) =
        require_admin_account_id(&account, &state.kernel.config_ref().admin_accounts).map(|_| ())
    {
        return response;
    }
    let target = {
        let live_channels = state.channels_config.read().await;
        match resolve_whatsapp_bootstrap_target(&live_channels, &instance_key) {
            Some(target) => target,
            None => {
                return ApiErrorResponse::not_found("Owned WhatsApp instance not found")
                    .into_json_tuple()
                    .into_response()
            }
        }
    };
    let store = channel_bootstrap_store(state.kernel.home_dir());
    if let Err(e) = store.load() {
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    let session = match store
        .get_pending_by_instance("whatsapp", &target.instance_key)
        .await
    {
        Some(session) => session,
        None => {
            return ApiErrorResponse::not_found("Owned WhatsApp bootstrap session not found")
                .into_json_tuple()
                .into_response()
        }
    };
    let cancelled = match store
        .cancel(&session.bootstrap_id, chrono::Utc::now())
        .await
    {
        Ok(cancelled) => cancelled,
        Err(e) => {
            return ApiErrorResponse::internal(e)
                .into_json_tuple()
                .into_response()
        }
    };
    (
        StatusCode::OK,
        Json(bootstrap_session_view(
            &cancelled,
            "WhatsApp bootstrap session cancelled",
            false,
        )),
    )
        .into_response()
}

async fn gateway_http_post_json(
    url_with_path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
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

    let body_str = serde_json::to_string(body).map_err(|e| format!("Encode failed: {e}"))?;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body_str.len(),
        body_str
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

struct WhatsAppBootstrapTarget {
    account_id: String,
    instance_key: String,
    gateway_url: String,
}

struct WeChatBootstrapTarget {
    account_id: String,
    instance_key: String,
    bot_token_env: String,
}

fn require_admin_account_id(
    account: &AccountId,
    admin_accounts: &[String],
) -> Result<String, axum::response::Response> {
    let account_id = require_tenant_account_id(account)?.to_string();
    if let Err((code, json)) = require_admin(account, admin_accounts) {
        return Err((code, json).into_response());
    }
    Ok(account_id)
}

fn wechat_ilink_base() -> String {
    std::env::var("LIBREFANG_WECHAT_ILINK_BASE").unwrap_or_else(|_| WECHAT_ILINK_BASE.to_string())
}

fn wechat_instance_key(account_id: &str) -> String {
    format!("wechat:{account_id}")
}

fn channel_bootstrap_store(
    home_dir: &std::path::Path,
) -> crate::channel_bootstrap::ChannelBootstrapStore {
    crate::channel_bootstrap::ChannelBootstrapStore::new(home_dir)
}

fn whatsapp_instance_key(account_id: &str) -> String {
    format!("whatsapp:{account_id}")
}

fn resolve_whatsapp_bootstrap_target(
    channels: &librefang_types::config::ChannelsConfig,
    instance_key: &str,
) -> Option<WhatsAppBootstrapTarget> {
    channels.whatsapp.iter().find_map(|entry| {
        let account_id = entry.account_id.as_deref()?.trim();
        if account_id.is_empty() {
            return None;
        }
        let derived_instance_key = whatsapp_instance_key(account_id);
        if derived_instance_key != instance_key {
            return None;
        }
        let gateway_url = std::env::var(&entry.gateway_url_env)
            .ok()?
            .trim()
            .to_string();
        if gateway_url.is_empty() {
            return None;
        }
        Some(WhatsAppBootstrapTarget {
            account_id: account_id.to_string(),
            instance_key: derived_instance_key,
            gateway_url,
        })
    })
}

fn resolve_wechat_bootstrap_target(
    config: &librefang_types::config::ChannelsConfig,
    instance_key: &str,
) -> Option<WeChatBootstrapTarget> {
    config.wechat.iter().find_map(|entry| {
        let account_id = entry.account_id.as_deref()?.trim();
        if account_id.is_empty() {
            return None;
        }
        let derived_instance_key = wechat_instance_key(account_id);
        if derived_instance_key != instance_key {
            return None;
        }
        let bot_token_env = entry.bot_token_env.trim();
        if bot_token_env.is_empty() {
            return None;
        }
        Some(WeChatBootstrapTarget {
            account_id: account_id.to_string(),
            instance_key: derived_instance_key,
            bot_token_env: bot_token_env.to_string(),
        })
    })
}

async fn persist_wechat_owned_bot_token(
    home_dir: &std::path::Path,
    token_env_name: &str,
    bot_token: &str,
) -> Result<(), String> {
    validate_env_var(token_env_name, bot_token)?;
    write_secret_env(&home_dir.join("secrets.env"), token_env_name, bot_token)
        .map_err(|e| format!("Failed to persist WeChat bot token: {e}"))?;
    // SAFETY: configuration mutation during explicit admin bootstrap flow.
    unsafe {
        std::env::set_var(token_env_name, bot_token);
    }
    Ok(())
}

fn bootstrap_session_view(
    session: &crate::channel_bootstrap::ChannelBootstrapSession,
    message: &str,
    connected: bool,
) -> serde_json::Value {
    serde_json::json!({
        "bootstrap_id": session.bootstrap_id,
        "channel_type": session.channel_type,
        "instance_key": session.instance_key,
        "account_id": session.account_id,
        "status": serde_json::to_value(session.status).unwrap_or(serde_json::Value::Null),
        "qr_url": session.provider_qr_url,
        "qr_payload": session.provider_qr_payload,
        "expires_at": session.expires_at,
        "connected": connected,
        "message": message,
        "last_error": session.last_error,
    })
}

#[utoipa::path(
    post,
    path = "/api/channels/wechat/{instance_key}/bootstrap/start",
    tag = "channels",
    params(
        ("instance_key" = String, Path, description = "Owned WeChat instance key, e.g. wechat:tenant-a")
    ),
    responses(
        (status = 200, description = "WeChat bootstrap session created", body = serde_json::Value),
        (status = 404, description = "Owned WeChat instance not found", body = serde_json::Value)
    )
)]
pub async fn wechat_bootstrap_start(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(instance_key): Path<String>,
) -> axum::response::Response {
    let created_by =
        match require_admin_account_id(&account, &state.kernel.config_ref().admin_accounts) {
            Ok(account_id) => account_id,
            Err(response) => return response,
        };
    let target = {
        let live_channels = state.channels_config.read().await;
        match resolve_wechat_bootstrap_target(&live_channels, &instance_key) {
            Some(target) => target,
            None => {
                return ApiErrorResponse::not_found("Owned WeChat instance not found")
                    .into_json_tuple()
                    .into_response()
            }
        }
    };

    let client = match librefang_runtime::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return ApiErrorResponse::internal(format!("HTTP client error: {e}"))
                .into_json_tuple()
                .into_response()
        }
    };

    let url = format!(
        "{}/ilink/bot/get_bot_qrcode?bot_type=3",
        wechat_ilink_base()
    );
    let response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Could not reach iLink API: {e}"))
                .into_json_tuple()
                .into_response()
        }
    };
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return ApiErrorResponse::internal(format!("iLink QR request failed ({status}): {body}"))
            .into_json_tuple()
            .into_response();
    }
    let body = match response.json::<serde_json::Value>().await {
        Ok(body) => body,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Failed to parse iLink response: {e}"))
                .into_json_tuple()
                .into_response()
        }
    };
    let provider_handle = body
        .get("qrcode")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    if provider_handle.is_empty() {
        return ApiErrorResponse::internal("iLink returned empty qrcode")
            .into_json_tuple()
            .into_response();
    }

    let now = chrono::Utc::now();
    let session = crate::channel_bootstrap::ChannelBootstrapSession {
        bootstrap_id: uuid::Uuid::new_v4().to_string(),
        channel_type: "wechat".to_string(),
        instance_key: target.instance_key.clone(),
        account_id: target.account_id.clone(),
        bootstrap_kind: crate::channel_bootstrap::BootstrapKind::QrLogin,
        provider_handle: Some(provider_handle.to_string()),
        provider_qr_payload: body
            .get("qrcode_img_content")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        provider_qr_url: body
            .get("qrcode_img_content")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        provider_pairing_code: None,
        status: crate::channel_bootstrap::BootstrapStatus::Pending,
        created_at: now,
        updated_at: now,
        expires_at: Some(now + chrono::Duration::minutes(5)),
        created_by,
        last_error: None,
    };
    let store = channel_bootstrap_store(state.kernel.home_dir());
    if let Err(e) = store.load() {
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    if let Err(e) = store.create(session.clone()).await {
        return ApiErrorResponse::bad_request(e)
            .into_json_tuple()
            .into_response();
    }

    (
        StatusCode::OK,
        Json(bootstrap_session_view(
            &session,
            "Scan this QR code with your WeChat app to log in",
            false,
        )),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/channels/wechat/{instance_key}/bootstrap/status",
    tag = "channels",
    params(
        ("instance_key" = String, Path, description = "Owned WeChat instance key, e.g. wechat:tenant-a")
    ),
    responses(
        (status = 200, description = "WeChat bootstrap status", body = serde_json::Value),
        (status = 404, description = "Owned WeChat bootstrap session not found", body = serde_json::Value)
    )
)]
pub async fn wechat_bootstrap_status(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(instance_key): Path<String>,
) -> axum::response::Response {
    if let Err(response) =
        require_admin_account_id(&account, &state.kernel.config_ref().admin_accounts).map(|_| ())
    {
        return response;
    }

    let target = {
        let live_channels = state.channels_config.read().await;
        match resolve_wechat_bootstrap_target(&live_channels, &instance_key) {
            Some(target) => target,
            None => {
                return ApiErrorResponse::not_found("Owned WeChat instance not found")
                    .into_json_tuple()
                    .into_response()
            }
        }
    };

    let store = channel_bootstrap_store(state.kernel.home_dir());
    if let Err(e) = store.load() {
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    let session = match store
        .get_latest_by_instance("wechat", &target.instance_key)
        .await
    {
        Some(session) => session,
        None => {
            return ApiErrorResponse::not_found("Owned WeChat bootstrap session not found")
                .into_json_tuple()
                .into_response()
        }
    };

    if session.status != crate::channel_bootstrap::BootstrapStatus::Pending {
        let connected = session.status == crate::channel_bootstrap::BootstrapStatus::Confirmed;
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &session,
                "Owned bootstrap status loaded",
                connected,
            )),
        )
            .into_response();
    }

    if session
        .expires_at
        .map(|expires_at| chrono::Utc::now() >= expires_at)
        .unwrap_or(false)
    {
        let expired = match store
            .expire(&session.bootstrap_id, chrono::Utc::now())
            .await
        {
            Ok(expired) => expired,
            Err(e) => {
                return ApiErrorResponse::internal(e)
                    .into_json_tuple()
                    .into_response()
            }
        };
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &expired,
                "QR code expired — click Start to get a new one",
                false,
            )),
        )
            .into_response();
    }

    let provider_handle = match session.provider_handle.as_deref() {
        Some(handle) if !handle.is_empty() => handle,
        _ => {
            return ApiErrorResponse::internal(
                "Owned WeChat bootstrap session is missing provider handle",
            )
            .into_json_tuple()
            .into_response()
        }
    };

    let client = match librefang_runtime::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return ApiErrorResponse::internal(format!("HTTP client error: {e}"))
                .into_json_tuple()
                .into_response()
        }
    };
    let encoded: String =
        url::form_urlencoded::byte_serialize(provider_handle.as_bytes()).collect();
    let url = format!(
        "{}/ilink/bot/get_qrcode_status?qrcode={encoded}",
        wechat_ilink_base()
    );
    let response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(_) => {
            return (
                StatusCode::OK,
                Json(bootstrap_session_view(
                    &session,
                    "Waiting for scan...",
                    false,
                )),
            )
                .into_response()
        }
    };
    if !response.status().is_success() {
        return (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &session,
                "Waiting for scan...",
                false,
            )),
        )
            .into_response();
    }
    let body = match response.json::<serde_json::Value>().await {
        Ok(body) => body,
        Err(_) => {
            return (
                StatusCode::OK,
                Json(bootstrap_session_view(
                    &session,
                    "Failed to parse status response",
                    false,
                )),
            )
                .into_response()
        }
    };

    match body
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("pending")
    {
        "confirmed" => {
            let bot_token = body
                .get("bot_token")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if bot_token.is_empty() {
                let failed = match store
                    .fail(
                        &session.bootstrap_id,
                        chrono::Utc::now(),
                        "WeChat confirmation response did not include bot_token".to_string(),
                    )
                    .await
                {
                    Ok(failed) => failed,
                    Err(e) => {
                        return ApiErrorResponse::internal(e)
                            .into_json_tuple()
                            .into_response()
                    }
                };
                return (
                    StatusCode::OK,
                    Json(bootstrap_session_view(
                        &failed,
                        "WeChat confirmation failed",
                        false,
                    )),
                )
                    .into_response();
            }
            if let Err(e) = persist_wechat_owned_bot_token(
                state.kernel.home_dir(),
                &target.bot_token_env,
                bot_token,
            )
            .await
            {
                return ApiErrorResponse::internal(e)
                    .into_json_tuple()
                    .into_response();
            }
            let confirmed = match store
                .confirm(&session.bootstrap_id, chrono::Utc::now())
                .await
            {
                Ok(confirmed) => confirmed,
                Err(e) => {
                    return ApiErrorResponse::internal(e)
                        .into_json_tuple()
                        .into_response()
                }
            };
            (
                StatusCode::OK,
                Json(bootstrap_session_view(
                    &confirmed,
                    "WeChat login successful",
                    true,
                )),
            )
                .into_response()
        }
        "expired" => {
            let expired = match store
                .expire(&session.bootstrap_id, chrono::Utc::now())
                .await
            {
                Ok(expired) => expired,
                Err(e) => {
                    return ApiErrorResponse::internal(e)
                        .into_json_tuple()
                        .into_response()
                }
            };
            (
                StatusCode::OK,
                Json(bootstrap_session_view(
                    &expired,
                    "QR code expired — click Start to get a new one",
                    false,
                )),
            )
                .into_response()
        }
        _ => (
            StatusCode::OK,
            Json(bootstrap_session_view(
                &session,
                "Waiting for scan...",
                false,
            )),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/channels/wechat/{instance_key}/bootstrap/cancel",
    tag = "channels",
    params(
        ("instance_key" = String, Path, description = "Owned WeChat instance key, e.g. wechat:tenant-a")
    ),
    responses(
        (status = 200, description = "WeChat bootstrap session cancelled", body = serde_json::Value),
        (status = 404, description = "Owned WeChat bootstrap session not found", body = serde_json::Value)
    )
)]
pub async fn wechat_bootstrap_cancel(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(instance_key): Path<String>,
) -> axum::response::Response {
    if let Err(response) =
        require_admin_account_id(&account, &state.kernel.config_ref().admin_accounts).map(|_| ())
    {
        return response;
    }
    let target = {
        let live_channels = state.channels_config.read().await;
        match resolve_wechat_bootstrap_target(&live_channels, &instance_key) {
            Some(target) => target,
            None => {
                return ApiErrorResponse::not_found("Owned WeChat instance not found")
                    .into_json_tuple()
                    .into_response()
            }
        }
    };
    let store = channel_bootstrap_store(state.kernel.home_dir());
    if let Err(e) = store.load() {
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    let session = match store
        .get_pending_by_instance("wechat", &target.instance_key)
        .await
    {
        Some(session) => session,
        None => {
            return ApiErrorResponse::not_found("Owned WeChat bootstrap session not found")
                .into_json_tuple()
                .into_response()
        }
    };
    let cancelled = match store
        .cancel(&session.bootstrap_id, chrono::Utc::now())
        .await
    {
        Ok(cancelled) => cancelled,
        Err(e) => {
            return ApiErrorResponse::internal(e)
                .into_json_tuple()
                .into_response()
        }
    };
    (
        StatusCode::OK,
        Json(bootstrap_session_view(
            &cancelled,
            "WeChat bootstrap session cancelled",
            false,
        )),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/api/channels/wechat/qr/start",
    tag = "channels",
    responses(
        (status = 200, description = "WeChat QR login initiated", body = serde_json::Value)
    )
)]
/// POST /api/channels/wechat/qr/start — Request a QR code from iLink for WeChat login.
pub async fn wechat_qr_start(
    account: AccountId,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let client = match librefang_runtime::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Json(serde_json::json!({
                "available": false,
                "message": format!("HTTP client error: {e}")
            }))
            .into_response();
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
                    }))
                    .into_response();
                }
                Json(serde_json::json!({
                    "available": true,
                    "qr_code": qrcode,
                    "qr_url": qrcode_url,
                    "message": "Scan this QR code with your WeChat app to log in",
                }))
                .into_response()
            }
            Err(e) => Json(serde_json::json!({
                "available": false,
                "message": format!("Failed to parse iLink response: {e}")
            }))
            .into_response(),
        },
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Json(serde_json::json!({
                "available": false,
                "message": format!("iLink QR request failed ({status}): {body}")
            }))
            .into_response()
        }
        Err(e) => Json(serde_json::json!({
            "available": false,
            "message": format!("Could not reach iLink API: {e}")
        }))
        .into_response(),
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
        (status = 200, description = "WeChat QR scan status", body = serde_json::Value)
    )
)]
/// GET /api/channels/wechat/qr/status — Poll iLink for QR scan confirmation.
pub async fn wechat_qr_status(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let qr_code = params.get("qr_code").cloned().unwrap_or_default();
    if qr_code.is_empty() {
        return Json(serde_json::json!({
            "connected": false,
            "expired": false,
            "message": "Missing qr_code parameter"
        }))
        .into_response();
    }

    // iLink uses long-polling: the request hangs until the user scans or it
    // times out server-side (~30s). Use a generous timeout so we don't mistake
    // a normal long-poll wait for a network error.
    let client = match librefang_runtime::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return Json(serde_json::json!({
                "connected": false,
                "expired": false,
                "message": "HTTP client error"
            }))
            .into_response();
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
                        .into_response()
                    }
                    "expired" => Json(serde_json::json!({
                        "connected": false,
                        "expired": true,
                        "message": "QR code expired — click Start to get a new one"
                    }))
                    .into_response(),
                    _ => Json(serde_json::json!({
                        "connected": false,
                        "expired": false,
                        "message": "Waiting for scan..."
                    }))
                    .into_response(),
                }
            }
            Err(_) => Json(serde_json::json!({
                "connected": false,
                "expired": false,
                "message": "Failed to parse status response"
            }))
            .into_response(),
        },
        // Timeout is normal for long-poll — treat as "still waiting"
        _ => Json(serde_json::json!({
            "connected": false,
            "expired": false,
            "message": "Waiting for scan..."
        }))
        .into_response(),
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
pub async fn list_channel_registry(
    State(state): State<Arc<AppState>>,
    account: AccountId,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let channels_dir = state.kernel.home_dir().join("channels");
    let metadata = librefang_runtime::channel_registry::load_channel_metadata(&channels_dir);
    Json(serde_json::to_value(&metadata).unwrap_or_default()).into_response()
}
