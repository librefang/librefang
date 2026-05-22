//! Wire protocol message types.
//!
//! All communication between LibreFang peers uses JSON-framed messages
//! over TCP. Each message is prefixed with a 4-byte big-endian length header.

use serde::{Deserialize, Serialize};

/// A wire protocol message (envelope).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    /// Unique message ID.
    pub id: String,
    /// Message variant.
    #[serde(flatten)]
    pub kind: WireMessageKind,
}

/// The different kinds of wire messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WireMessageKind {
    /// Request from one peer to another.
    #[serde(rename = "request")]
    Request(WireRequest),
    /// Response to a request.
    #[serde(rename = "response")]
    Response(WireResponse),
    /// One-way notification (no response expected).
    #[serde(rename = "notification")]
    Notification(WireNotification),
    /// Forward-compat fallback: any unknown message `type` from a peer
    /// running a newer protocol version. Decodes successfully so the TCP
    /// link stays alive (#3544); callers should ignore the message.
    #[serde(other)]
    Unknown,
}

/// Request messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum WireRequest {
    /// Handshake: exchange peer identity.
    #[serde(rename = "handshake")]
    Handshake {
        /// The peer's unique node ID.
        node_id: String,
        /// Human-readable node name.
        node_name: String,
        /// Protocol version.
        protocol_version: u32,
        /// List of agents available on this peer.
        agents: Vec<RemoteAgentInfo>,
        /// Random nonce for HMAC authentication.
        #[serde(default)]
        nonce: String,
        /// HMAC-SHA256(shared_secret, nonce + node_id).
        #[serde(default)]
        auth_hmac: String,
        /// SECURITY (#3873): Sender's Ed25519 public key (base64). Optional
        /// for backward compatibility with peers that do not yet provision a
        /// keypair — those fall back to HMAC-only authentication and no
        /// TOFU pin is established.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        public_key: Option<String>,
        /// SECURITY (#3873): Ed25519 signature (base64) over the same
        /// `nonce|node_id|recipient_node_id` byte string the HMAC covers,
        /// signed with the sender's private key. Verified against
        /// `public_key`; on first contact the pubkey is pinned to `node_id`
        /// (TOFU) and subsequent handshakes from the same `node_id` MUST
        /// present the same pubkey or are rejected. Optional only when
        /// `public_key` is also absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity_signature: Option<String>,
        /// SECURITY (#4269): Per-handshake X25519 ephemeral public key
        /// (base64, 32 bytes). When both peers send one, the per-message
        /// HMAC `session_key` is derived via X25519 ECDH + HKDF instead
        /// of from `shared_secret + nonces`, decoupling session integrity
        /// from `shared_secret` and gaining forward secrecy. Optional
        /// for backward compatibility with peers that omit it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ephemeral_pubkey: Option<String>,
    },
    /// Discover agents matching a query on the remote peer.
    #[serde(rename = "discover")]
    Discover {
        /// Search query (matches name, tags, description).
        query: String,
    },
    /// Send a message to a specific agent on the remote peer.
    #[serde(rename = "agent_message")]
    AgentMessage {
        /// Target agent ID or name on the remote peer.
        agent: String,
        /// The message text.
        message: String,
        /// Optional sender identity.
        sender: Option<String>,
    },
    /// Ping to check if the peer is alive.
    #[serde(rename = "ping")]
    Ping,
    /// Forward-compat fallback for unknown `method` values. See `WireMessageKind::Unknown`.
    #[serde(other)]
    Unknown,
}

/// Response messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum WireResponse {
    /// Handshake acknowledgement.
    #[serde(rename = "handshake_ack")]
    HandshakeAck {
        node_id: String,
        node_name: String,
        protocol_version: u32,
        agents: Vec<RemoteAgentInfo>,
        /// Random nonce for HMAC authentication.
        #[serde(default)]
        nonce: String,
        /// HMAC-SHA256(shared_secret, nonce + node_id).
        #[serde(default)]
        auth_hmac: String,
        /// SECURITY (#3873): See `WireRequest::Handshake::public_key`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        public_key: Option<String>,
        /// SECURITY (#3873): See `WireRequest::Handshake::identity_signature`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity_signature: Option<String>,
        /// SECURITY (#4269): See `WireRequest::Handshake::ephemeral_pubkey`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ephemeral_pubkey: Option<String>,
    },
    /// Discovery results.
    #[serde(rename = "discover_result")]
    DiscoverResult { agents: Vec<RemoteAgentInfo> },
    /// Agent message response.
    #[serde(rename = "agent_response")]
    AgentResponse {
        /// The agent's response text.
        text: String,
    },
    /// Pong response.
    #[serde(rename = "pong")]
    Pong {
        /// Uptime in seconds.
        uptime_secs: u64,
    },
    /// Error response.
    #[serde(rename = "error")]
    Error {
        /// Error code.
        code: i32,
        /// Error message.
        message: String,
    },
    /// Forward-compat fallback for unknown `method` values. See `WireMessageKind::Unknown`.
    #[serde(other)]
    Unknown,
}

/// Notification messages (one-way, no response).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum WireNotification {
    /// An agent was spawned on the peer.
    #[serde(rename = "agent_spawned")]
    AgentSpawned { agent: RemoteAgentInfo },
    /// An agent was terminated on the peer.
    #[serde(rename = "agent_terminated")]
    AgentTerminated { agent_id: String },
    /// Peer is shutting down.
    #[serde(rename = "shutting_down")]
    ShuttingDown,
    /// Forward-compat fallback for unknown `event` values. See `WireMessageKind::Unknown`.
    #[serde(other)]
    Unknown,
}

/// Information about a remote agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAgentInfo {
    /// Agent ID (UUID string).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of what the agent does.
    pub description: String,
    /// Tags for categorization/discovery.
    pub tags: Vec<String>,
    /// Available tools.
    pub tools: Vec<String>,
    /// Current state.
    pub state: String,
}

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Encode a wire message to bytes (4-byte big-endian length + JSON).
pub fn encode_message(msg: &WireMessage) -> Result<Vec<u8>, serde_json::Error> {
    let json = serde_json::to_vec(msg)?;
    let len = json.len() as u32;
    let mut bytes = Vec::with_capacity(4 + json.len());
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes.extend_from_slice(&json);
    Ok(bytes)
}

/// Decode the length prefix from a 4-byte header.
pub fn decode_length(header: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*header)
}

/// Parse a JSON body into a WireMessage.
pub fn decode_message(body: &[u8]) -> Result<WireMessage, serde_json::Error> {
    serde_json::from_slice(body)
}

/// Classify which `Unknown` arm (if any) a decoded `WireMessage`
/// matched, and the raw tag string the peer actually sent. Returns
/// `None` when the message decoded into a known variant — the
/// common case. Used by the receive loops to attach a `warn!` log
/// with the offending tag so peer-misbehaviour or
/// protocol-version-skew is visible to operators instead of being
/// silently dropped (audit: wire-message-other-variant-silent).
pub fn classify_unknown(body: &[u8], msg: &WireMessage) -> Option<UnknownTag> {
    let envelope = match &msg.kind {
        WireMessageKind::Unknown => UnknownLevel::Envelope,
        WireMessageKind::Request(WireRequest::Unknown) => UnknownLevel::RequestMethod,
        WireMessageKind::Response(WireResponse::Unknown) => UnknownLevel::ResponseMethod,
        WireMessageKind::Notification(WireNotification::Unknown) => {
            UnknownLevel::NotificationEvent
        }
        _ => return None,
    };
    // The `serde(other)` arm doesn't capture the raw tag the peer
    // sent, so re-peek the JSON body for the relevant field. Best-
    // effort: a body that no longer parses (somehow decoded once and
    // now doesn't — shouldn't happen in practice) falls back to
    // `"<unparseable>"` so the log line is still produced.
    let raw_tag = peek_tag_for(body, envelope).unwrap_or_else(|| "<unparseable>".into());
    Some(UnknownTag {
        level: envelope,
        raw_tag,
    })
}

/// Which `serde(other)` arm a peer triggered. Determines which top-
/// level JSON field name to inspect for the offending tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownLevel {
    /// `WireMessageKind::Unknown` — top-level `type` field unrecognised.
    Envelope,
    /// `WireRequest::Unknown` — `type:"request"` but `method` field unrecognised.
    RequestMethod,
    /// `WireResponse::Unknown` — `type:"response"` but `method` field unrecognised.
    ResponseMethod,
    /// `WireNotification::Unknown` — `type:"notification"` but `event` field unrecognised.
    NotificationEvent,
}

impl UnknownLevel {
    /// Stable string name suitable for tracing field values.
    pub fn name(self) -> &'static str {
        match self {
            UnknownLevel::Envelope => "envelope.type",
            UnknownLevel::RequestMethod => "request.method",
            UnknownLevel::ResponseMethod => "response.method",
            UnknownLevel::NotificationEvent => "notification.event",
        }
    }
}

/// Result of [`classify_unknown`] for messages that landed in a
/// `serde(other)` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTag {
    /// Which arm matched (envelope `type`, request/response `method`,
    /// notification `event`).
    pub level: UnknownLevel,
    /// The raw tag string the peer actually sent — `"future_message"`,
    /// `"future_method"`, etc. May be `"<unparseable>"` or
    /// `"<missing>"` for pathological inputs.
    pub raw_tag: String,
}

fn peek_tag_for(body: &[u8], level: UnknownLevel) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = v.as_object()?;
    let field = match level {
        UnknownLevel::Envelope => "type",
        UnknownLevel::RequestMethod | UnknownLevel::ResponseMethod => "method",
        UnknownLevel::NotificationEvent => "event",
    };
    Some(
        obj.get(field)
            .and_then(|x| x.as_str())
            .unwrap_or("<missing>")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let msg = WireMessage {
            id: "msg-1".to_string(),
            kind: WireMessageKind::Request(WireRequest::Ping),
        };
        let bytes = encode_message(&msg).unwrap();
        // First 4 bytes are length
        let len = decode_length(&[bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(len as usize, bytes.len() - 4);
        let decoded = decode_message(&bytes[4..]).unwrap();
        assert_eq!(decoded.id, "msg-1");
    }

    #[test]
    fn test_handshake_serialization() {
        let msg = WireMessage {
            id: "hs-1".to_string(),
            kind: WireMessageKind::Request(WireRequest::Handshake {
                node_id: "node-abc".to_string(),
                node_name: "my-kernel".to_string(),
                protocol_version: PROTOCOL_VERSION,
                agents: vec![RemoteAgentInfo {
                    id: "agent-1".to_string(),
                    name: "coder".to_string(),
                    description: "A coding agent".to_string(),
                    tags: vec!["code".to_string()],
                    tools: vec!["file_read".to_string()],
                    state: "running".to_string(),
                }],
                nonce: "test-nonce".to_string(),
                auth_hmac: "test-hmac".to_string(),
                public_key: None,
                identity_signature: None,
                ephemeral_pubkey: None,
            }),
        };
        let json = serde_json::to_string_pretty(&msg).unwrap();
        assert!(json.contains("handshake"));
        assert!(json.contains("coder"));
        let decoded: WireMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "hs-1");
    }

    #[test]
    fn test_agent_message_serialization() {
        let msg = WireMessage {
            id: "am-1".to_string(),
            kind: WireMessageKind::Request(WireRequest::AgentMessage {
                agent: "coder".to_string(),
                message: "Write a hello world".to_string(),
                sender: Some("orchestrator".to_string()),
            }),
        };
        let bytes = encode_message(&msg).unwrap();
        let decoded = decode_message(&bytes[4..]).unwrap();
        match decoded.kind {
            WireMessageKind::Request(WireRequest::AgentMessage { agent, message, .. }) => {
                assert_eq!(agent, "coder");
                assert_eq!(message, "Write a hello world");
            }
            other => panic!("Expected AgentMessage, got {other:?}"),
        }
    }

    #[test]
    fn test_error_response() {
        let msg = WireMessage {
            id: "err-1".to_string(),
            kind: WireMessageKind::Response(WireResponse::Error {
                code: 404,
                message: "Agent not found".to_string(),
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: WireMessage = serde_json::from_str(&json).unwrap();
        match decoded.kind {
            WireMessageKind::Response(WireResponse::Error { code, message }) => {
                assert_eq!(code, 404);
                assert_eq!(message, "Agent not found");
            }
            other => panic!("Expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_notification_serialization() {
        let msg = WireMessage {
            id: "n-1".to_string(),
            kind: WireMessageKind::Notification(WireNotification::AgentSpawned {
                agent: RemoteAgentInfo {
                    id: "a-1".to_string(),
                    name: "researcher".to_string(),
                    description: "Research agent".to_string(),
                    tags: vec![],
                    tools: vec![],
                    state: "running".to_string(),
                },
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("agent_spawned"));
        let _: WireMessage = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_unknown_message_type_decodes() {
        // A peer running a newer protocol version may emit message types we
        // don't understand. The TCP link must stay alive (#3544).
        let json = r#"{"id":"x","type":"future_message","payload":{"foo":1}}"#;
        let decoded: WireMessage = serde_json::from_str(json).unwrap();
        match decoded.kind {
            WireMessageKind::Unknown => {}
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn test_unknown_request_method_decodes() {
        let json = r#"{"id":"x","type":"request","method":"future_method","x":1}"#;
        let decoded: WireMessage = serde_json::from_str(json).unwrap();
        match decoded.kind {
            WireMessageKind::Request(WireRequest::Unknown) => {}
            other => panic!("expected Request(Unknown), got {other:?}"),
        }
    }

    #[test]
    fn test_unknown_response_method_decodes() {
        let json = r#"{"id":"x","type":"response","method":"future_method"}"#;
        let decoded: WireMessage = serde_json::from_str(json).unwrap();
        match decoded.kind {
            WireMessageKind::Response(WireResponse::Unknown) => {}
            other => panic!("expected Response(Unknown), got {other:?}"),
        }
    }

    #[test]
    fn test_unknown_notification_event_decodes() {
        let json = r#"{"id":"x","type":"notification","event":"future_event"}"#;
        let decoded: WireMessage = serde_json::from_str(json).unwrap();
        match decoded.kind {
            WireMessageKind::Notification(WireNotification::Unknown) => {}
            other => panic!("expected Notification(Unknown), got {other:?}"),
        }
    }

    #[test]
    fn classify_unknown_envelope_type_returns_raw_tag() {
        // Audit: wire-message-other-variant-silent. The decoded
        // message lands in `WireMessageKind::Unknown` but
        // `classify_unknown` peeks the body and surfaces the raw
        // tag the peer sent so receivers can log it.
        let json = r#"{"id":"x","type":"future_message","payload":{"foo":1}}"#;
        let msg: WireMessage = serde_json::from_str(json).unwrap();
        let cls = classify_unknown(json.as_bytes(), &msg);
        assert_eq!(
            cls,
            Some(UnknownTag {
                level: UnknownLevel::Envelope,
                raw_tag: "future_message".to_string(),
            })
        );
    }

    #[test]
    fn classify_unknown_request_method_returns_raw_tag() {
        let json = r#"{"id":"x","type":"request","method":"future_method","x":1}"#;
        let msg: WireMessage = serde_json::from_str(json).unwrap();
        let cls = classify_unknown(json.as_bytes(), &msg);
        assert_eq!(
            cls,
            Some(UnknownTag {
                level: UnknownLevel::RequestMethod,
                raw_tag: "future_method".to_string(),
            })
        );
    }

    #[test]
    fn classify_unknown_response_method_returns_raw_tag() {
        let json = r#"{"id":"x","type":"response","method":"future_resp"}"#;
        let msg: WireMessage = serde_json::from_str(json).unwrap();
        let cls = classify_unknown(json.as_bytes(), &msg);
        assert_eq!(
            cls,
            Some(UnknownTag {
                level: UnknownLevel::ResponseMethod,
                raw_tag: "future_resp".to_string(),
            })
        );
    }

    #[test]
    fn classify_unknown_notification_event_returns_raw_tag() {
        let json = r#"{"id":"x","type":"notification","event":"future_event"}"#;
        let msg: WireMessage = serde_json::from_str(json).unwrap();
        let cls = classify_unknown(json.as_bytes(), &msg);
        assert_eq!(
            cls,
            Some(UnknownTag {
                level: UnknownLevel::NotificationEvent,
                raw_tag: "future_event".to_string(),
            })
        );
    }

    #[test]
    fn classify_unknown_returns_none_for_known_variant() {
        // Sanity: the happy-path messages don't trigger the
        // observability hook — receivers should only log when a
        // serde(other) arm fired.
        let json = r#"{"id":"x","type":"request","method":"ping"}"#;
        let msg: WireMessage = serde_json::from_str(json).unwrap();
        assert!(classify_unknown(json.as_bytes(), &msg).is_none());
    }

    #[test]
    fn classify_unknown_handles_missing_tag_gracefully() {
        // A malformed peer that sent `{"id":"x"}` (no `type` field
        // at all) won't actually decode into our enum — it'll fail
        // at serde. But if a peer somehow lands in Unknown with a
        // body that has no usable tag (e.g. `type` field was a
        // number, not a string), the helper falls back to
        // `<missing>` rather than panicking, so the warn line is
        // still produced.
        // Construct manually to simulate.
        let body = b"{\"id\":\"x\",\"type\":42}".to_vec();
        let msg = WireMessage {
            id: "x".to_string(),
            kind: WireMessageKind::Unknown,
        };
        let cls = classify_unknown(&body, &msg).unwrap();
        assert_eq!(cls.level, UnknownLevel::Envelope);
        assert_eq!(cls.raw_tag, "<missing>");
    }

    #[test]
    fn test_discover_request() {
        let msg = WireMessage {
            id: "d-1".to_string(),
            kind: WireMessageKind::Request(WireRequest::Discover {
                query: "security".to_string(),
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("discover"));
        assert!(json.contains("security"));
    }
}
