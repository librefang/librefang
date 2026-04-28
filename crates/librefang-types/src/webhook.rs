//! Webhook trigger types for system event injection and isolated agent turns.

use serde::{Deserialize, Serialize};

/// Wake mode for system event injection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeMode {
    /// Trigger immediate processing.
    #[default]
    Now,
    /// Defer until the next heartbeat cycle.
    NextHeartbeat,
}

/// Payload for POST /hooks/wake — inject a system event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakePayload {
    /// Event text to inject (max 4096 chars).
    pub text: String,
    /// When to process the event.
    #[serde(default)]
    pub mode: WakeMode,
}

/// Payload for POST /hooks/agent — run an isolated agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHookPayload {
    /// Message to send to the agent (max 16384 chars).
    pub message: String,
    /// Target agent (by name or ID). None = default agent.
    #[serde(default)]
    pub agent: Option<String>,
    /// Whether to deliver response to a channel.
    #[serde(default)]
    pub deliver: bool,
    /// Target channel for delivery.
    #[serde(default)]
    pub channel: Option<String>,
    /// Model override.
    #[serde(default)]
    pub model: Option<String>,
    /// Timeout in seconds (default 120, max 600).
    #[serde(default = "default_hook_timeout")]
    pub timeout_secs: u64,
}

fn default_hook_timeout() -> u64 {
    120
}

/// Maximum length for wake event text.
const MAX_WAKE_TEXT: usize = 4096;
/// Maximum length for agent hook message.
const MAX_AGENT_MESSAGE: usize = 16384;
/// Minimum timeout in seconds.
const MIN_TIMEOUT_SECS: u64 = 10;
/// Maximum timeout in seconds.
const MAX_TIMEOUT_SECS: u64 = 600;
/// Maximum channel name length.
const MAX_CHANNEL_NAME: usize = 64;

/// Returns true if the character is a control character other than newline.
fn is_forbidden_control(c: char) -> bool {
    c.is_control() && c != '\n'
}

impl WakePayload {
    /// Validate the wake payload.
    ///
    /// - `text` must be non-empty.
    /// - `text` must not exceed 4096 characters.
    /// - `text` must not contain control characters other than newline.
    pub fn validate(&self) -> Result<(), String> {
        if self.text.is_empty() {
            return Err("text must not be empty".to_string());
        }
        if self.text.len() > MAX_WAKE_TEXT {
            return Err(format!(
                "text exceeds maximum length of {} chars (got {})",
                MAX_WAKE_TEXT,
                self.text.len()
            ));
        }
        if let Some(pos) = self.text.find(is_forbidden_control) {
            let c = self.text[pos..].chars().next().unwrap();
            return Err(format!(
                "text contains forbidden control character U+{:04X} at byte offset {}",
                c as u32, pos
            ));
        }
        Ok(())
    }
}

impl AgentHookPayload {
    /// Validate the agent hook payload.
    ///
    /// - `message` must be non-empty.
    /// - `message` must not exceed 16384 characters.
    /// - `timeout_secs` must be between 10 and 600 inclusive.
    /// - `channel`, if present, must not exceed 64 characters.
    pub fn validate(&self) -> Result<(), String> {
        if self.message.is_empty() {
            return Err("message must not be empty".to_string());
        }
        if self.message.len() > MAX_AGENT_MESSAGE {
            return Err(format!(
                "message exceeds maximum length of {} chars (got {})",
                MAX_AGENT_MESSAGE,
                self.message.len()
            ));
        }
        if self.timeout_secs < MIN_TIMEOUT_SECS || self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs must be between {} and {} (got {})",
                MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS, self.timeout_secs
            ));
        }
        if let Some(ref ch) = self.channel {
            if ch.len() > MAX_CHANNEL_NAME {
                return Err(format!(
                    "channel name exceeds maximum length of {} chars (got {})",
                    MAX_CHANNEL_NAME,
                    ch.len()
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── WakePayload validation ──────────────────────────────────────

    #[test]
    fn wake_valid_simple() {
        let p = WakePayload {
            text: "deploy complete".to_string(),
            mode: WakeMode::Now,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn wake_valid_with_newlines() {
        let p = WakePayload {
            text: "line one\nline two\nline three".to_string(),
            mode: WakeMode::NextHeartbeat,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn wake_empty_text() {
        let p = WakePayload {
            text: String::new(),
            mode: WakeMode::Now,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn wake_text_too_long() {
        let p = WakePayload {
            text: "x".repeat(4097),
            mode: WakeMode::Now,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("exceeds maximum length"), "got: {err}");
    }

    #[test]
    fn wake_text_exactly_max() {
        let p = WakePayload {
            text: "a".repeat(4096),
            mode: WakeMode::Now,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn wake_control_char_rejected() {
        let p = WakePayload {
            text: "hello\x00world".to_string(),
            mode: WakeMode::Now,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("control character"), "got: {err}");
    }

    #[test]
    fn wake_tab_rejected() {
        let p = WakePayload {
            text: "col1\tcol2".to_string(),
            mode: WakeMode::Now,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("control character"), "got: {err}");
    }

    // ── AgentHookPayload validation ─────────────────────────────────

    #[test]
    fn agent_hook_valid_minimal() {
        let p = AgentHookPayload {
            message: "summarize today's logs".to_string(),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 120,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn agent_hook_valid_full() {
        let p = AgentHookPayload {
            message: "deploy staging".to_string(),
            agent: Some("devops-lead".to_string()),
            deliver: true,
            channel: Some("slack-ops".to_string()),
            model: Some("sonnet".to_string()),
            timeout_secs: 300,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn agent_hook_empty_message() {
        let p = AgentHookPayload {
            message: String::new(),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 120,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn agent_hook_message_too_long() {
        let p = AgentHookPayload {
            message: "m".repeat(16385),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 120,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("exceeds maximum length"), "got: {err}");
    }

    #[test]
    fn agent_hook_message_exactly_max() {
        let p = AgentHookPayload {
            message: "m".repeat(16384),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 120,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn agent_hook_timeout_too_low() {
        let p = AgentHookPayload {
            message: "hello".to_string(),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 5,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("timeout_secs must be between"), "got: {err}");
    }

    #[test]
    fn agent_hook_timeout_too_high() {
        let p = AgentHookPayload {
            message: "hello".to_string(),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 601,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("timeout_secs must be between"), "got: {err}");
    }

    #[test]
    fn agent_hook_timeout_boundary_min() {
        let p = AgentHookPayload {
            message: "hello".to_string(),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 10,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn agent_hook_timeout_boundary_max() {
        let p = AgentHookPayload {
            message: "hello".to_string(),
            agent: None,
            deliver: false,
            channel: None,
            model: None,
            timeout_secs: 600,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn agent_hook_channel_too_long() {
        let p = AgentHookPayload {
            message: "hello".to_string(),
            agent: None,
            deliver: true,
            channel: Some("c".repeat(65)),
            model: None,
            timeout_secs: 120,
        };
        let err = p.validate().unwrap_err();
        assert!(err.contains("channel name exceeds"), "got: {err}");
    }

    #[test]
    fn agent_hook_channel_exactly_max() {
        let p = AgentHookPayload {
            message: "hello".to_string(),
            agent: None,
            deliver: true,
            channel: Some("c".repeat(64)),
            model: None,
            timeout_secs: 120,
        };
        assert!(p.validate().is_ok());
    }

    // ── Serde roundtrips ────────────────────────────────────────────

    #[test]
    fn wake_serde_roundtrip_now() {
        let orig = WakePayload {
            text: "something happened".to_string(),
            mode: WakeMode::Now,
        };
        let json = serde_json::to_string(&orig).unwrap();
        let back: WakePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, orig.text);
        assert_eq!(back.mode, WakeMode::Now);
    }

    #[test]
    fn wake_serde_roundtrip_next_heartbeat() {
        let orig = WakePayload {
            text: "deferred event".to_string(),
            mode: WakeMode::NextHeartbeat,
        };
        let json = serde_json::to_string(&orig).unwrap();
        assert!(json.contains("\"next_heartbeat\""));
        let back: WakePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mode, WakeMode::NextHeartbeat);
    }

    #[test]
    fn wake_serde_default_mode() {
        let json = r#"{"text":"hello"}"#;
        let p: WakePayload = serde_json::from_str(json).unwrap();
        assert_eq!(p.mode, WakeMode::Now);
    }

    #[test]
    fn agent_hook_serde_roundtrip() {
        let orig = AgentHookPayload {
            message: "run diagnostics".to_string(),
            agent: Some("ops".to_string()),
            deliver: true,
            channel: Some("slack-alerts".to_string()),
            model: Some("gemini-2.5-flash".to_string()),
            timeout_secs: 300,
        };
        let json = serde_json::to_string(&orig).unwrap();
        let back: AgentHookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message, orig.message);
        assert_eq!(back.agent.as_deref(), Some("ops"));
        assert!(back.deliver);
        assert_eq!(back.channel.as_deref(), Some("slack-alerts"));
        assert_eq!(back.model.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(back.timeout_secs, 300);
    }

    #[test]
    fn agent_hook_serde_defaults() {
        let json = r#"{"message":"hi"}"#;
        let p: AgentHookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(p.message, "hi");
        assert!(p.agent.is_none());
        assert!(!p.deliver);
        assert!(p.channel.is_none());
        assert!(p.model.is_none());
        assert_eq!(p.timeout_secs, 120);
    }

    #[test]
    fn wake_mode_serde_variants() {
        let now: WakeMode = serde_json::from_str(r#""now""#).unwrap();
        assert_eq!(now, WakeMode::Now);
        let next: WakeMode = serde_json::from_str(r#""next_heartbeat""#).unwrap();
        assert_eq!(next, WakeMode::NextHeartbeat);
    }
}
