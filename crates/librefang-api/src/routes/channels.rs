//! Channel configuration, status, and WhatsApp QR flow handlers.

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
            "/channels/whatsapp/qr/start",
            axum::routing::post(whatsapp_qr_start),
        )
        .route(
            "/channels/whatsapp/qr/status",
            axum::routing::get(whatsapp_qr_status),
        )
}

use super::skills::{
    remove_channel_config, remove_secret_env, upsert_channel_config, validate_env_var,
    write_secret_env,
};
use super::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

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
}

impl FieldType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Text => "text",
            Self::Number => "number",
            Self::List => "list",
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
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("TELEGRAM_BOT_TOKEN"), required: true, placeholder: "123456:ABC-DEF...", advanced: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "12345, 67890", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
            ChannelField { key: "poll_interval_secs", label: "Poll Interval (sec)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "1", advanced: true },
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
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("DISCORD_BOT_TOKEN"), required: true, placeholder: "MTIz...", advanced: false },
            ChannelField { key: "allowed_guilds", label: "Allowed Guild IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123456789, 987654321", advanced: true },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123456789, 987654321", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
            ChannelField { key: "intents", label: "Intents Bitmask", field_type: FieldType::Number, env_var: None, required: false, placeholder: "37376", advanced: true },
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
            ChannelField { key: "app_token_env", label: "App Token (xapp-)", field_type: FieldType::Secret, env_var: Some("SLACK_APP_TOKEN"), required: true, placeholder: "xapp-1-...", advanced: false },
            ChannelField { key: "bot_token_env", label: "Bot Token (xoxb-)", field_type: FieldType::Secret, env_var: Some("SLACK_BOT_TOKEN"), required: true, placeholder: "xoxb-...", advanced: false },
            ChannelField { key: "allowed_channels", label: "Allowed Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "C01234, C56789", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("WHATSAPP_ACCESS_TOKEN"), required: false, placeholder: "EAAx...", advanced: true },
            ChannelField { key: "phone_number_id", label: "Phone Number ID", field_type: FieldType::Text, env_var: None, required: false, placeholder: "1234567890", advanced: true },
            ChannelField { key: "verify_token_env", label: "Verify Token", field_type: FieldType::Secret, env_var: Some("WHATSAPP_VERIFY_TOKEN"), required: false, placeholder: "my-verify-token", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8443", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "api_url", label: "signal-cli API URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "http://localhost:8080", advanced: false },
            ChannelField { key: "phone_number", label: "Phone Number", field_type: FieldType::Text, env_var: None, required: true, placeholder: "+1234567890", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("MATRIX_ACCESS_TOKEN"), required: true, placeholder: "syt_...", advanced: false },
            ChannelField { key: "homeserver_url", label: "Homeserver URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://matrix.org", advanced: false },
            ChannelField { key: "user_id", label: "Bot User ID", field_type: FieldType::Text, env_var: None, required: false, placeholder: "@librefang:matrix.org", advanced: true },
            ChannelField { key: "allowed_rooms", label: "Allowed Room IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "!abc:matrix.org", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "username", label: "Email Address", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@example.com", advanced: false },
            ChannelField { key: "password_env", label: "Password / App Password", field_type: FieldType::Secret, env_var: Some("EMAIL_PASSWORD"), required: true, placeholder: "app-password", advanced: false },
            ChannelField { key: "imap_host", label: "IMAP Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "imap.gmail.com", advanced: false },
            ChannelField { key: "smtp_host", label: "SMTP Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "smtp.gmail.com", advanced: false },
            ChannelField { key: "imap_port", label: "IMAP Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "993", advanced: true },
            ChannelField { key: "smtp_port", label: "SMTP Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "587", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "channel_secret_env", label: "Channel Secret", field_type: FieldType::Secret, env_var: Some("LINE_CHANNEL_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "access_token_env", label: "Channel Access Token", field_type: FieldType::Secret, env_var: Some("LINE_CHANNEL_ACCESS_TOKEN"), required: true, placeholder: "xyz789...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8450", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "auth_token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("VIBER_AUTH_TOKEN"), required: true, placeholder: "4dc...", advanced: false },
            ChannelField { key: "webhook_url", label: "Webhook URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://your-domain.com/viber", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8451", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "page_token_env", label: "Page Access Token", field_type: FieldType::Secret, env_var: Some("MESSENGER_PAGE_TOKEN"), required: true, placeholder: "EAAx...", advanced: false },
            ChannelField { key: "verify_token_env", label: "Verify Token", field_type: FieldType::Secret, env_var: Some("MESSENGER_VERIFY_TOKEN"), required: false, placeholder: "my-verify-token", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8452", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "secret_env", label: "API Secret", field_type: FieldType::Secret, env_var: Some("THREEMA_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "threema_id", label: "Gateway ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "*MYID01", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8454", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "username", label: "Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang_bot", advanced: false },
            ChannelField { key: "paperkey_env", label: "Paper Key", field_type: FieldType::Secret, env_var: Some("KEYBASE_PAPERKEY"), required: true, placeholder: "word1 word2 word3...", advanced: false },
            ChannelField { key: "allowed_teams", label: "Allowed Teams", field_type: FieldType::List, env_var: None, required: false, placeholder: "team1, team2", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "client_id", label: "Client ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123def", advanced: false },
            ChannelField { key: "client_secret_env", label: "Client Secret", field_type: FieldType::Secret, env_var: Some("REDDIT_CLIENT_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "username", label: "Bot Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang_bot", advanced: false },
            ChannelField { key: "password_env", label: "Bot Password", field_type: FieldType::Secret, env_var: Some("REDDIT_PASSWORD"), required: true, placeholder: "password", advanced: false },
            ChannelField { key: "subreddits", label: "Subreddits", field_type: FieldType::List, env_var: None, required: false, placeholder: "librefang, rust", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("MASTODON_ACCESS_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "instance_url", label: "Instance URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://mastodon.social", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "identifier", label: "Handle", field_type: FieldType::Text, env_var: None, required: true, placeholder: "user.bsky.social", advanced: false },
            ChannelField { key: "app_password_env", label: "App Password", field_type: FieldType::Secret, env_var: Some("BLUESKY_APP_PASSWORD"), required: true, placeholder: "xxxx-xxxx-xxxx-xxxx", advanced: false },
            ChannelField { key: "service_url", label: "PDS URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://bsky.social", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("LINKEDIN_ACCESS_TOKEN"), required: true, placeholder: "AQV...", advanced: false },
            ChannelField { key: "organization_id", label: "Organization ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "12345678", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "private_key_env", label: "Private Key", field_type: FieldType::Secret, env_var: Some("NOSTR_PRIVATE_KEY"), required: true, placeholder: "nsec1...", advanced: false },
            ChannelField { key: "relays", label: "Relay URLs", field_type: FieldType::List, env_var: None, required: false, placeholder: "wss://relay.damus.io", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "00000000-0000-...", advanced: false },
            ChannelField { key: "app_password_env", label: "App Password", field_type: FieldType::Secret, env_var: Some("TEAMS_APP_PASSWORD"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "3978", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://mattermost.example.com", advanced: false },
            ChannelField { key: "token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("MATTERMOST_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "allowed_channels", label: "Allowed Channels", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123, def456", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "service_account_env", label: "Service Account JSON", field_type: FieldType::Secret, env_var: Some("GOOGLE_CHAT_SERVICE_ACCOUNT"), required: true, placeholder: "/path/to/key.json", advanced: false },
            ChannelField { key: "space_ids", label: "Space IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "spaces/AAAA", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8444", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("WEBEX_BOT_TOKEN"), required: true, placeholder: "NjI...", advanced: false },
            ChannelField { key: "allowed_rooms", label: "Allowed Rooms", field_type: FieldType::List, env_var: None, required: false, placeholder: "Y2lz...", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "cli_abc123", advanced: false },
            ChannelField { key: "app_secret_env", label: "App Secret", field_type: FieldType::Secret, env_var: Some("FEISHU_APP_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8453", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create an app at open.feishu.cn", "Copy App ID and Secret", "Paste them below"],
        config_template: "[channels.feishu]\napp_id = \"\"\napp_secret_env = \"FEISHU_APP_SECRET\"",
    },
    ChannelMeta {
        name: "dingtalk", display_name: "DingTalk", icon: "DT",
        description: "DingTalk Robot API adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your webhook token and signing secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("DINGTALK_ACCESS_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "secret_env", label: "Signing Secret", field_type: FieldType::Secret, env_var: Some("DINGTALK_SECRET"), required: true, placeholder: "SEC...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8457", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a robot in your DingTalk group", "Copy the token and signing secret", "Paste them below"],
        config_template: "[channels.dingtalk]\naccess_token_env = \"DINGTALK_ACCESS_TOKEN\"\nsecret_env = \"DINGTALK_SECRET\"",
    },
    ChannelMeta {
        name: "pumble", display_name: "Pumble", icon: "PB",
        description: "Pumble bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("PUMBLE_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8455", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("FLOCK_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8456", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "token_env", label: "API Token", field_type: FieldType::Secret, env_var: Some("TWIST_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "workspace_id", label: "Workspace ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "12345", advanced: false },
            ChannelField { key: "allowed_channels", label: "Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123, 456", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://chat.zulip.org", advanced: false },
            ChannelField { key: "bot_email", label: "Bot Email", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@zulip.example.com", advanced: false },
            ChannelField { key: "api_key_env", label: "API Key", field_type: FieldType::Secret, env_var: Some("ZULIP_API_KEY"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "streams", label: "Streams", field_type: FieldType::List, env_var: None, required: false, placeholder: "general, dev", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "server", label: "Server", field_type: FieldType::Text, env_var: None, required: true, placeholder: "irc.libera.chat", advanced: false },
            ChannelField { key: "nick", label: "Nickname", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang", advanced: false },
            ChannelField { key: "channels", label: "Channels", field_type: FieldType::List, env_var: None, required: false, placeholder: "#librefang, #general", advanced: false },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "6667", advanced: true },
            ChannelField { key: "use_tls", label: "Use TLS", field_type: FieldType::Text, env_var: None, required: false, placeholder: "false", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "jid", label: "JID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@jabber.org", advanced: false },
            ChannelField { key: "password_env", label: "Password", field_type: FieldType::Secret, env_var: Some("XMPP_PASSWORD"), required: true, placeholder: "password", advanced: false },
            ChannelField { key: "server", label: "Server", field_type: FieldType::Text, env_var: None, required: false, placeholder: "jabber.org", advanced: true },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "5222", advanced: true },
            ChannelField { key: "rooms", label: "MUC Rooms", field_type: FieldType::List, env_var: None, required: false, placeholder: "room@conference.jabber.org", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("GITTER_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "room_id", label: "Room ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123def456", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "base_url", label: "Forum URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://forum.example.com", advanced: false },
            ChannelField { key: "api_key_env", label: "API Key", field_type: FieldType::Secret, env_var: Some("DISCOURSE_API_KEY"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "api_username", label: "API Username", field_type: FieldType::Text, env_var: None, required: false, placeholder: "system", advanced: true },
            ChannelField { key: "categories", label: "Categories", field_type: FieldType::List, env_var: None, required: false, placeholder: "general, support", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("REVOLT_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "api_url", label: "API URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://api.revolt.chat", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("GUILDED_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "server_ids", label: "Server IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://cloud.example.com", advanced: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("NEXTCLOUD_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "allowed_rooms", label: "Room Tokens", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://rocket.example.com", advanced: false },
            ChannelField { key: "user_id", label: "Bot User ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123", advanced: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("ROCKETCHAT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "allowed_channels", label: "Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "GENERAL", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "oauth_token_env", label: "OAuth Token", field_type: FieldType::Secret, env_var: Some("TWITCH_OAUTH_TOKEN"), required: true, placeholder: "oauth:abc123...", advanced: false },
            ChannelField { key: "nick", label: "Bot Nickname", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang", advanced: false },
            ChannelField { key: "channels", label: "Channels (no #)", field_type: FieldType::List, env_var: None, required: true, placeholder: "mychannel", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "topic", label: "Topic", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang-alerts", advanced: false },
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://ntfy.sh", advanced: true },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("NTFY_TOKEN"), required: false, placeholder: "tk_abc123...", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://gotify.example.com", advanced: false },
            ChannelField { key: "app_token_env", label: "App Token (send)", field_type: FieldType::Secret, env_var: Some("GOTIFY_APP_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "client_token_env", label: "Client Token (receive)", field_type: FieldType::Secret, env_var: Some("GOTIFY_CLIENT_TOKEN"), required: true, placeholder: "def456...", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "secret_env", label: "HMAC Secret", field_type: FieldType::Secret, env_var: Some("WEBHOOK_SECRET"), required: false, placeholder: "my-secret", advanced: false },
            ChannelField { key: "listen_port", label: "Listen Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8460", advanced: true },
            ChannelField { key: "callback_url", label: "Callback URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://example.com/webhook", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Enter an HMAC secret (or leave blank)", "Click Save — that's it!"],
        config_template: "[channels.webhook]\nsecret_env = \"WEBHOOK_SECRET\"",
    },
    ChannelMeta {
        name: "mumble", display_name: "Mumble", icon: "MB",
        description: "Mumble text chat adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Enter server host and username",
        setup_type: "form",
        fields: &[
            ChannelField { key: "host", label: "Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "mumble.example.com", advanced: false },
            ChannelField { key: "username", label: "Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "librefang", advanced: false },
            ChannelField { key: "password_env", label: "Server Password", field_type: FieldType::Secret, env_var: Some("MUMBLE_PASSWORD"), required: false, placeholder: "password", advanced: true },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "64738", advanced: true },
            ChannelField { key: "channel", label: "Channel", field_type: FieldType::Text, env_var: None, required: false, placeholder: "Root", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
            ChannelField { key: "bot_token_env", label: "Bot Token (from previous session)", field_type: FieldType::Secret, env_var: Some("WECHAT_BOT_TOKEN"), required: false, placeholder: "ilink_bot_...", advanced: true },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123@im.wechat", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Open WeChat on your phone", "The QR code will appear in the dashboard", "Scan it with WeChat to connect"],
        config_template: "[channels.wechat]\nbot_token_env = \"WECHAT_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "wecom", display_name: "WeCom", icon: "WC",
        description: "WeCom (WeChat Work) adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your Corp ID, Agent ID, and Secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "corp_id", label: "Corp ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "wwxxxxx", advanced: false },
            ChannelField { key: "agent_id", label: "Agent ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "wwxxxxx", advanced: false },
            ChannelField { key: "secret_env", label: "Secret", field_type: FieldType::Secret, env_var: Some("WECOM_SECRET"), required: true, placeholder: "secret", advanced: false },
            ChannelField { key: "token_env", label: "Callback Token", field_type: FieldType::Secret, env_var: Some("WECOM_TOKEN"), required: false, placeholder: "WECOM_TOKEN", advanced: true },
            ChannelField { key: "encoding_aes_key_env", label: "Encoding AES Key", field_type: FieldType::Secret, env_var: Some("WECOM_ENCODING_AES_KEY"), required: false, placeholder: "WECOM_ENCODING_AES_KEY", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8454", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a WeCom application at work.weixin.qq.com", "Get Corp ID, Agent ID, and Secret", "Configure callback URL to your webhook endpoint"],
        config_template: "[channels.wecom]\ncorp_id = \"\"\nagent_id = \"\"\nsecret_env = \"WECOM_SECRET\"",
    },
    ChannelMeta {
        name: "qq", display_name: "QQ Bot", icon: "QQ",
        description: "QQ Bot API v2 — guild, group, and DM adapter",
        category: "messaging", difficulty: "Medium", setup_time: "~5 min",
        quick_setup: "Enter your App ID and set QQ_BOT_APP_SECRET env var",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "102xxxxx", advanced: false },
            ChannelField { key: "app_secret_env", label: "App Secret", field_type: FieldType::Secret, env_var: Some("QQ_BOT_APP_SECRET"), required: true, placeholder: "secret", advanced: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "12345, 67890", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
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
        "mumble" => config.mumble.is_some(),
        "wechat" => config.wechat.is_some(),
        "wecom" => config.wecom.is_some(),
        "qq" => config.qq.is_some(),
        _ => false,
    }
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

/// GET /api/channels — List all 40 channel adapters with status and field metadata.
#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "channels",
    responses(
        (status = 200, description = "List configured channels", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read the live channels config (updated on every hot-reload) instead of the
    // stale boot-time kernel.config, so newly configured channels show correctly.
    let live_channels = state.channels_config.read().await;
    let mut channels = Vec::new();
    let mut configured_count = 0u32;

    for meta in CHANNEL_REGISTRY {
        let configured = is_channel_configured(&live_channels, meta.name);
        if configured {
            configured_count += 1;
        }

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
        let fields: Vec<serde_json::Value> = meta
            .fields
            .iter()
            .map(|f| build_field_json(f, config_vals.as_ref()))
            .collect();

        channels.push(serde_json::json!({
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
        }));
    }

    Json(serde_json::json!({
        "channels": channels,
        "total": channels.len(),
        "configured_count": configured_count,
    }))
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
    Path(name): Path<String>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Unknown channel: {name}")})),
            )
        }
    };

    let live_channels = state.channels_config.read().await;
    let configured = is_channel_configured(&live_channels, meta.name);

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
    let fields: Vec<serde_json::Value> = meta
        .fields
        .iter()
        .map(|f| build_field_json(f, config_vals.as_ref()))
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
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
        })),
    )
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
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Unknown channel"})),
            )
        }
    };

    let fields = match body.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'fields' object"})),
            )
        }
    };

    let home = librefang_kernel::config::librefang_home();
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
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": msg})),
                );
            }
            // Secret field — write to secrets.env and set in process
            if let Err(e) = write_secret_env(&secrets_path, env_var, value) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to write secret: {e}")})),
                );
            }
            // SAFETY: We are the only writer; this is a single-threaded config operation
            unsafe {
                std::env::set_var(env_var, value);
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

    // Write config.toml section
    if let Err(e) = upsert_channel_config(&config_path, &name, &config_fields) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write config: {e}")})),
        );
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
        (status = 200, description = "Channel removed successfully", body = serde_json::Value),
        (status = 404, description = "Unknown channel", body = serde_json::Value),
        (status = 500, description = "Internal server error", body = serde_json::Value)
    )
)]
/// DELETE /api/channels/{name}/configure — Remove channel secrets + config section.
pub async fn remove_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Unknown channel"})),
            )
        }
    };

    let home = librefang_kernel::config::librefang_home();
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

    // Remove config section
    if let Err(e) = remove_channel_config(&config_path, &name) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to remove config: {e}")})),
        );
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
        ),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after remove");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "removed",
                    "channel": name,
                    "note": format!("Removed, but hot-reload failed: {e}. Restart daemon to fully deactivate.")
                })),
            )
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
    Path(name): Path<String>,
    raw_body: axum::body::Bytes,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"status": "error", "message": "Unknown channel"})),
            )
        }
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
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Missing required env vars: {}", missing.join(", "))
            })),
        );
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
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": format!("Credentials valid but failed to send test message: {e}")
                    })),
                );
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
    let client = librefang_runtime::http_client::proxied_client();
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
        (status = 200, description = "Channels reloaded successfully", body = serde_json::Value),
        (status = 500, description = "Reload failed", body = serde_json::Value)
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "error": e,
            })),
        ),
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
        (status = 200, description = "WhatsApp QR session started", body = serde_json::Value)
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
        (status = 200, description = "WhatsApp QR scan status", body = serde_json::Value)
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
