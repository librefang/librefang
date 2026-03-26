//! Configuration validation logic: unknown field detection, structural validation, and safety boundary constraints.

use super::types::*;

impl KernelConfig {
    pub fn known_top_level_fields() -> &'static [&'static str] {
        &[
            "home_dir",
            "data_dir",
            "log_level",
            "api_listen",
            "listen_addr", // alias for api_listen
            "cors_origin",
            "network_enabled",
            "default_model",
            "memory",
            "network",
            "channels",
            "api_key",
            "mode",
            "language",
            "users",
            "mcp_servers",
            "a2a",
            "usage_footer",
            "stable_prefix_mode",
            "web",
            "fallback_providers",
            "browser",
            "extensions",
            "vault",
            "workspaces_dir",
            "media",
            "links",
            "reload",
            "webhook_triggers",
            "approval",
            "approval_policy", // alias for approval
            "max_cron_jobs",
            "include",
            "exec_policy",
            "bindings",
            "broadcast",
            "auto_reply",
            "canvas",
            "tts",
            "docker",
            "pairing",
            "auth_profiles",
            "provider_cooldown",
            "thinking",
            "budget",
            "provider_urls",
            "provider_regions",
            "provider_api_keys",
            "vertex_ai",
            "oauth",
            "sidecar_channels",
            "proxy",
            "prompt_caching",
            "session",
            "queue",
            "external_auth",
            "tool_policy",
            "proactive_memory",
            "context_engine",
            "audit",
            "health_check",
            "plugins",
            "strict_config",
            "dashboard_user",
            "dashboard_pass",
            "dashboard_pass_hash",
            "log_dir",
            "inbox",
            "azure_openai",
            "heartbeat",
            "privacy",
            "prompt_intelligence",
            "qwen_code_path",
            "sanitize",
            "telemetry",
            "update_channel",
        ]
    }

    /// Detect unknown top-level keys in a raw TOML value.
    ///
    /// Returns a list of field names that appear at the top level of the
    /// config file but are not recognised by `KernelConfig`.
    pub fn detect_unknown_fields(raw: &toml::Value) -> Vec<String> {
        let known: std::collections::HashSet<&str> =
            Self::known_top_level_fields().iter().copied().collect();
        let mut unknown = Vec::new();
        if let toml::Value::Table(tbl) = raw {
            for key in tbl.keys() {
                if !known.contains(key.as_str()) {
                    unknown.push(key.clone());
                }
            }
        }
        unknown.sort();
        unknown
    }

    /// Validate the configuration, returning a list of warnings.
    ///
    /// Checks for common misconfigurations such as missing API keys for
    /// configured channels, invalid port numbers, unreachable paths,
    /// and unrecognised log levels.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        for tg in self.channels.telegram.iter() {
            if std::env::var(&tg.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Telegram configured but {} is not set",
                    tg.bot_token_env
                ));
            }
        }
        for dc in self.channels.discord.iter() {
            if std::env::var(&dc.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Discord configured but {} is not set",
                    dc.bot_token_env
                ));
            }
        }
        for sl in self.channels.slack.iter() {
            if std::env::var(&sl.app_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Slack configured but {} is not set",
                    sl.app_token_env
                ));
            }
            if std::env::var(&sl.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Slack configured but {} is not set",
                    sl.bot_token_env
                ));
            }
        }
        for wa in self.channels.whatsapp.iter() {
            if std::env::var(&wa.access_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "WhatsApp configured but {} is not set",
                    wa.access_token_env
                ));
            }
        }
        for mx in self.channels.matrix.iter() {
            if std::env::var(&mx.access_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Matrix configured but {} is not set",
                    mx.access_token_env
                ));
            }
        }
        for em in self.channels.email.iter() {
            if std::env::var(&em.password_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Email configured but {} is not set",
                    em.password_env
                ));
            }
        }
        for t in self.channels.teams.iter() {
            if std::env::var(&t.app_password_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Teams configured but {} is not set",
                    t.app_password_env
                ));
            }
        }
        for m in self.channels.mattermost.iter() {
            if std::env::var(&m.token_env).unwrap_or_default().is_empty() {
                warnings.push(format!(
                    "Mattermost configured but {} is not set",
                    m.token_env
                ));
            }
        }
        for z in self.channels.zulip.iter() {
            if std::env::var(&z.api_key_env).unwrap_or_default().is_empty() {
                warnings.push(format!("Zulip configured but {} is not set", z.api_key_env));
            }
        }
        for tw in self.channels.twitch.iter() {
            if std::env::var(&tw.oauth_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Twitch configured but {} is not set",
                    tw.oauth_token_env
                ));
            }
        }
        for rc in self.channels.rocketchat.iter() {
            if std::env::var(&rc.token_env).unwrap_or_default().is_empty() {
                warnings.push(format!(
                    "Rocket.Chat configured but {} is not set",
                    rc.token_env
                ));
            }
        }
        for gc in self.channels.google_chat.iter() {
            let has_env = !std::env::var(&gc.service_account_env)
                .unwrap_or_default()
                .is_empty();
            let has_key_path = gc
                .service_account_key_path
                .as_ref()
                .is_some_and(|p| !p.is_empty());
            if !has_env && !has_key_path {
                warnings.push(format!(
                    "Google Chat configured but neither {} nor service_account_key_path is set",
                    gc.service_account_env
                ));
            }
        }
        for x in self.channels.xmpp.iter() {
            if std::env::var(&x.password_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!("XMPP configured but {} is not set", x.password_env));
            }
        }
        // Wave 3 channels
        for ln in self.channels.line.iter() {
            if std::env::var(&ln.access_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "LINE configured but {} is not set",
                    ln.access_token_env
                ));
            }
        }
        for vb in self.channels.viber.iter() {
            if std::env::var(&vb.auth_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Viber configured but {} is not set",
                    vb.auth_token_env
                ));
            }
        }
        for ms in self.channels.messenger.iter() {
            if std::env::var(&ms.page_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Messenger configured but {} is not set",
                    ms.page_token_env
                ));
            }
        }
        for rd in self.channels.reddit.iter() {
            if std::env::var(&rd.client_secret_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Reddit configured but {} is not set",
                    rd.client_secret_env
                ));
            }
        }
        for md in self.channels.mastodon.iter() {
            if std::env::var(&md.access_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Mastodon configured but {} is not set",
                    md.access_token_env
                ));
            }
        }
        for bs in self.channels.bluesky.iter() {
            if std::env::var(&bs.app_password_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Bluesky configured but {} is not set",
                    bs.app_password_env
                ));
            }
        }
        for fs in self.channels.feishu.iter() {
            if std::env::var(&fs.app_secret_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Feishu configured but {} is not set",
                    fs.app_secret_env
                ));
            }
        }
        for rv in self.channels.revolt.iter() {
            if std::env::var(&rv.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Revolt configured but {} is not set",
                    rv.bot_token_env
                ));
            }
        }
        // Wave 4 channels
        for nc in self.channels.nextcloud.iter() {
            if std::env::var(&nc.token_env).unwrap_or_default().is_empty() {
                warnings.push(format!(
                    "Nextcloud configured but {} is not set",
                    nc.token_env
                ));
            }
        }
        for gd in self.channels.guilded.iter() {
            if std::env::var(&gd.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Guilded configured but {} is not set",
                    gd.bot_token_env
                ));
            }
        }
        for kb in self.channels.keybase.iter() {
            if std::env::var(&kb.paperkey_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Keybase configured but {} is not set",
                    kb.paperkey_env
                ));
            }
        }
        for tm in self.channels.threema.iter() {
            if std::env::var(&tm.secret_env).unwrap_or_default().is_empty() {
                warnings.push(format!(
                    "Threema configured but {} is not set",
                    tm.secret_env
                ));
            }
        }
        for ns in self.channels.nostr.iter() {
            if std::env::var(&ns.private_key_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Nostr configured but {} is not set",
                    ns.private_key_env
                ));
            }
        }
        for wx in self.channels.webex.iter() {
            if std::env::var(&wx.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Webex configured but {} is not set",
                    wx.bot_token_env
                ));
            }
        }
        for pb in self.channels.pumble.iter() {
            if std::env::var(&pb.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Pumble configured but {} is not set",
                    pb.bot_token_env
                ));
            }
        }
        for fl in self.channels.flock.iter() {
            if std::env::var(&fl.bot_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Flock configured but {} is not set",
                    fl.bot_token_env
                ));
            }
        }
        for tw in self.channels.twist.iter() {
            if std::env::var(&tw.token_env).unwrap_or_default().is_empty() {
                warnings.push(format!("Twist configured but {} is not set", tw.token_env));
            }
        }
        // Wave 5 channels
        for mb in self.channels.mumble.iter() {
            if std::env::var(&mb.password_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Mumble configured but {} is not set",
                    mb.password_env
                ));
            }
        }
        for dt in self.channels.dingtalk.iter() {
            if std::env::var(&dt.access_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "DingTalk configured but {} is not set",
                    dt.access_token_env
                ));
            }
        }
        for dc in self.channels.discourse.iter() {
            if std::env::var(&dc.api_key_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Discourse configured but {} is not set",
                    dc.api_key_env
                ));
            }
        }
        for gt in self.channels.gitter.iter() {
            if std::env::var(&gt.token_env).unwrap_or_default().is_empty() {
                warnings.push(format!("Gitter configured but {} is not set", gt.token_env));
            }
        }
        for nf in self.channels.ntfy.iter() {
            if !nf.token_env.is_empty()
                && std::env::var(&nf.token_env).unwrap_or_default().is_empty()
            {
                warnings.push(format!("ntfy configured but {} is not set", nf.token_env));
            }
        }
        for gf in self.channels.gotify.iter() {
            if std::env::var(&gf.app_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "Gotify configured but {} is not set",
                    gf.app_token_env
                ));
            }
        }
        for wh in self.channels.webhook.iter() {
            if std::env::var(&wh.secret_env).unwrap_or_default().is_empty() {
                warnings.push(format!(
                    "Webhook configured but {} is not set",
                    wh.secret_env
                ));
            }
        }
        for li in self.channels.linkedin.iter() {
            if std::env::var(&li.access_token_env)
                .unwrap_or_default()
                .is_empty()
            {
                warnings.push(format!(
                    "LinkedIn configured but {} is not set",
                    li.access_token_env
                ));
            }
        }

        // Web search provider validation
        match self.web.search_provider {
            SearchProvider::Brave => {
                if std::env::var(&self.web.brave.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Brave search selected but {} is not set",
                        self.web.brave.api_key_env
                    ));
                }
            }
            SearchProvider::Tavily => {
                if std::env::var(&self.web.tavily.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Tavily search selected but {} is not set",
                        self.web.tavily.api_key_env
                    ));
                }
            }
            SearchProvider::Perplexity => {
                if std::env::var(&self.web.perplexity.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Perplexity search selected but {} is not set",
                        self.web.perplexity.api_key_env
                    ));
                }
            }
            SearchProvider::DuckDuckGo | SearchProvider::Auto => {}
        }

        // --- Structural validation ---

        // Validate api_listen has a parseable port
        if let Some(colon_pos) = self.api_listen.rfind(':') {
            let port_str = &self.api_listen[colon_pos + 1..];
            match port_str.parse::<u16>() {
                Ok(0) => {
                    warnings
                        .push("api_listen port is 0 (OS will assign a random port)".to_string());
                }
                Err(_) => {
                    warnings.push(format!("api_listen port '{}' is not a valid u16", port_str));
                }
                Ok(_) => {}
            }
        } else {
            warnings.push(format!(
                "api_listen '{}' does not contain a port (expected host:port)",
                self.api_listen
            ));
        }

        // Validate log_level is a recognised value
        match self.log_level.to_lowercase().as_str() {
            "trace" | "debug" | "info" | "warn" | "error" | "off" => {}
            other => {
                warnings.push(format!(
                    "log_level '{}' is not a recognised level (expected trace/debug/info/warn/error/off)",
                    other
                ));
            }
        }

        // Validate home_dir exists (or can be created)
        if !self.home_dir.as_os_str().is_empty() && !self.home_dir.exists() {
            warnings.push(format!(
                "home_dir '{}' does not exist (will be created on first use)",
                self.home_dir.display()
            ));
        }

        // Validate data_dir parent is writable (basic path sanity)
        if !self.data_dir.as_os_str().is_empty() && !self.data_dir.exists() {
            if let Some(parent) = self.data_dir.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    warnings.push(format!(
                        "data_dir parent '{}' does not exist",
                        parent.display()
                    ));
                }
            }
        }

        // Validate max_cron_jobs is within a reasonable range
        if self.max_cron_jobs > 10_000 {
            warnings.push(format!(
                "max_cron_jobs {} exceeds reasonable limit (10000)",
                self.max_cron_jobs
            ));
        }

        // Validate network config: shared_secret must be set if network is enabled
        if self.network_enabled && self.network.shared_secret.is_empty() {
            warnings.push("network_enabled is true but network.shared_secret is empty".to_string());
        }

        warnings
    }

    /// Clamp configuration values to safe production bounds.
    ///
    /// Called after loading config to prevent zero timeouts, unbounded buffers,
    /// or other misconfigurations that cause silent failures at runtime.
    pub fn clamp_bounds(&mut self) {
        // Browser timeout: min 5s, max 300s
        if self.browser.timeout_secs == 0 {
            self.browser.timeout_secs = 30;
        } else if self.browser.timeout_secs > 300 {
            self.browser.timeout_secs = 300;
        }

        // Browser max sessions: min 1, max 100
        if self.browser.max_sessions == 0 {
            self.browser.max_sessions = 3;
        } else if self.browser.max_sessions > 100 {
            self.browser.max_sessions = 100;
        }

        // Web fetch max_response_bytes: min 1KB, max 50MB
        if self.web.fetch.max_response_bytes == 0 {
            self.web.fetch.max_response_bytes = 5_000_000;
        } else if self.web.fetch.max_response_bytes > 50_000_000 {
            self.web.fetch.max_response_bytes = 50_000_000;
        }

        // Web fetch timeout: min 5s, max 120s
        if self.web.fetch.timeout_secs == 0 {
            self.web.fetch.timeout_secs = 30;
        } else if self.web.fetch.timeout_secs > 120 {
            self.web.fetch.timeout_secs = 120;
        }

        // Queue concurrency: min 1 per lane (0 would deadlock)
        if self.queue.concurrency.main_lane == 0 {
            self.queue.concurrency.main_lane = 1;
        }
        if self.queue.concurrency.cron_lane == 0 {
            self.queue.concurrency.cron_lane = 1;
        }
        if self.queue.concurrency.subagent_lane == 0 {
            self.queue.concurrency.subagent_lane = 1;
        }
    }
}
