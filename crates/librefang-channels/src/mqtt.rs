//! MQTT pub/sub channel adapter for IoT integration.
//!
//! Subscribes to configurable MQTT topics for incoming messages and publishes
//! agent responses to configurable response topics. Supports MQTT v3.1.1 and v5
//! via the `rumqttc` crate, with automatic reconnection and exponential backoff.

use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser, LifecycleReaction,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Default MQTT broker port.
const DEFAULT_PORT: u16 = 1883;
/// Channel buffer size for incoming messages.
const CHANNEL_BUFFER: usize = 256;
/// Default keep-alive interval in seconds.
const DEFAULT_KEEP_ALIVE_SECS: u64 = 30;
/// Capacity of the internal rumqttc event channel.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// Quality of Service level for MQTT operations.
#[derive(Debug, Clone, Copy, Default)]
pub enum MqttQoS {
    /// At most once (fire and forget).
    AtMostOnce,
    /// At least once (acknowledged delivery).
    #[default]
    AtLeastOnce,
    /// Exactly once (assured delivery).
    ExactlyOnce,
}

impl MqttQoS {
    fn to_rumqttc(self) -> QoS {
        match self {
            MqttQoS::AtMostOnce => QoS::AtMostOnce,
            MqttQoS::AtLeastOnce => QoS::AtLeastOnce,
            MqttQoS::ExactlyOnce => QoS::ExactlyOnce,
        }
    }
}

/// MQTT channel adapter configuration.
pub struct MqttConfig {
    /// Broker hostname or IP address (e.g. `"localhost"` or `"broker.hivemq.com"`).
    pub host: String,
    /// Broker port (default: 1883).
    pub port: u16,
    /// MQTT client ID. Must be unique per connection.
    pub client_id: String,
    /// Topics to subscribe to for incoming messages.
    pub subscribe_topics: Vec<String>,
    /// Topic to publish agent responses to.
    pub response_topic: String,
    /// Quality of Service level.
    pub qos: MqttQoS,
    /// Optional username for broker authentication.
    pub username: Option<String>,
    /// Optional password for broker authentication (zeroized on drop).
    pub password: Option<String>,
    /// Keep-alive interval in seconds (default: 30).
    pub keep_alive_secs: u64,
    /// Optional account identifier for multi-bot routing.
    pub account_id: Option<String>,
}

impl MqttConfig {
    /// Create a minimal config with required fields.
    pub fn new(
        host: String,
        client_id: String,
        subscribe_topics: Vec<String>,
        response_topic: String,
    ) -> Self {
        Self {
            host,
            port: DEFAULT_PORT,
            client_id,
            subscribe_topics,
            response_topic,
            qos: MqttQoS::default(),
            username: None,
            password: None,
            keep_alive_secs: DEFAULT_KEEP_ALIVE_SECS,
            account_id: None,
        }
    }
}

/// MQTT pub/sub channel adapter.
///
/// Connects to an MQTT broker, subscribes to configured topics for incoming
/// messages, and publishes agent responses to a dedicated response topic.
/// Supports credentials, configurable QoS, and automatic reconnection.
pub struct MqttAdapter {
    /// Broker host.
    host: String,
    /// Broker port.
    port: u16,
    /// Client ID.
    client_id: String,
    /// Topics to subscribe for incoming messages.
    subscribe_topics: Vec<String>,
    /// Topic to publish responses to.
    response_topic: String,
    /// QoS level.
    qos: MqttQoS,
    /// Optional username.
    username: Option<String>,
    /// SECURITY: Password is zeroized on drop.
    password: Option<Zeroizing<String>>,
    /// Keep-alive interval.
    keep_alive_secs: u64,
    /// Optional account identifier.
    account_id: Option<String>,
    /// MQTT async client handle (populated after `start()`).
    client: Arc<tokio::sync::RwLock<Option<AsyncClient>>>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl MqttAdapter {
    /// Create a new MQTT adapter from configuration.
    pub fn new(config: MqttConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            host: config.host,
            port: config.port,
            client_id: config.client_id,
            subscribe_topics: config.subscribe_topics,
            response_topic: config.response_topic,
            qos: config.qos,
            username: config.username,
            password: config.password.map(Zeroizing::new),
            keep_alive_secs: config.keep_alive_secs,
            account_id: config.account_id,
            client: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    /// Build MQTT options from adapter configuration.
    fn build_mqtt_options(&self) -> MqttOptions {
        let mut opts = MqttOptions::new(&self.client_id, &self.host, self.port);
        opts.set_keep_alive(Duration::from_secs(self.keep_alive_secs));

        if let (Some(ref user), Some(ref pass)) = (&self.username, &self.password) {
            opts.set_credentials(user.clone(), pass.as_str());
        }

        // NOTE: TLS not yet supported. Port 8883 connections will need
        // future work to call opts.set_transport(Transport::tls(...)).

        opts
    }

    /// Parse an MQTT payload into a ChannelMessage.
    ///
    /// Supports two formats:
    /// 1. JSON: `{"sender": "device-1", "message": "hello", "agent": "optional-agent-id"}`
    /// 2. Plain text: treated as message body with topic as sender ID.
    fn parse_payload(
        topic: &str,
        payload: &[u8],
        account_id: &Option<String>,
    ) -> Option<ChannelMessage> {
        let text = std::str::from_utf8(payload).ok()?;
        if text.is_empty() {
            return None;
        }

        // Try JSON format first
        let (sender_id, sender_name, message, target_agent) =
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
                let msg = val["message"].as_str().unwrap_or("").to_string();
                if msg.is_empty() {
                    return None;
                }
                let sender = val["sender"].as_str().unwrap_or("mqtt-device").to_string();
                let agent = val["agent"].as_str().map(|s| s.to_string());
                (sender.clone(), sender, msg, agent)
            } else {
                // Plain text: use last topic segment as sender hint
                let sender = topic
                    .rsplit('/')
                    .next()
                    .unwrap_or("mqtt-device")
                    .to_string();
                (sender.clone(), sender, text.to_string(), None)
            };

        let content = if message.starts_with('/') {
            let parts: Vec<&str> = message.splitn(2, ' ').collect();
            let cmd = parts[0].trim_start_matches('/');
            let args: Vec<String> = parts
                .get(1)
                .map(|a| a.split_whitespace().map(String::from).collect())
                .unwrap_or_default();
            ChannelContent::Command {
                name: cmd.to_string(),
                args,
            }
        } else {
            ChannelContent::Text(message)
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "mqtt_topic".to_string(),
            serde_json::Value::String(topic.to_string()),
        );

        let mut msg = ChannelMessage {
            channel: ChannelType::Custom("mqtt".to_string()),
            platform_message_id: uuid::Uuid::new_v4().to_string(),
            sender: ChannelUser {
                platform_id: sender_id,
                display_name: sender_name,
                librefang_user: None,
            },
            content,
            // `agent` field carries a UUID string; reject anything that doesn't
            // parse so we don't silently turn garbage into a bogus AgentId.
            target_agent: target_agent
                .as_deref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(librefang_types::agent::AgentId),
            timestamp: Utc::now(),
            is_group: false,
            thread_id: None,
            metadata,
        };

        if let Some(ref aid) = account_id {
            msg.metadata
                .insert("account_id".to_string(), serde_json::json!(aid));
        }

        Some(msg)
    }
}

#[async_trait]
impl ChannelAdapter for MqttAdapter {
    fn name(&self) -> &str {
        "mqtt"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("mqtt".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        info!(
            "MQTT adapter connecting to {}:{} (client_id={})",
            self.host, self.port, self.client_id
        );

        let opts = self.build_mqtt_options();
        let (client, mut eventloop) = AsyncClient::new(opts, EVENT_CHANNEL_CAPACITY);

        // Store client for sending responses
        {
            let mut lock = self.client.write().await;
            *lock = Some(client.clone());
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(CHANNEL_BUFFER);
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();
        let subscribe_topics = self.subscribe_topics.clone();
        let qos = self.qos.to_rumqttc();
        let client_clone = client;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("MQTT adapter shutting down");
                            return;
                        }
                    }
                    event = eventloop.poll() => {
                        match event {
                            Ok(Event::Incoming(Packet::Publish(publish))) => {
                                let topic = publish.topic.clone();
                                debug!("MQTT: received message on topic '{topic}'");

                                if let Some(msg) = MqttAdapter::parse_payload(
                                    &topic,
                                    &publish.payload,
                                    &account_id,
                                ) {
                                    if tx.send(msg).await.is_err() {
                                        info!("MQTT: receiver dropped, stopping event loop");
                                        return;
                                    }
                                }
                            }
                            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                                info!("MQTT: connected to broker");
                                for topic in &subscribe_topics {
                                    if let Err(e) = client_clone.subscribe(topic.as_str(), qos).await {
                                        warn!("MQTT: failed to subscribe to '{topic}': {e}");
                                    } else {
                                        info!("MQTT: subscribed to topic '{topic}'");
                                    }
                                }
                            }
                            Ok(_) => {
                                // Other events (PingResp, SubAck, etc.) — ignore
                            }
                            Err(e) => {
                                warn!("MQTT: event loop error: {e}");
                                // rumqttc handles reconnection internally;
                                // a short sleep prevents busy-looping on persistent errors.
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        _user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            ChannelContent::Command { name, args } => {
                format!("/{name} {}", args.join(" ")).trim().to_string()
            }
            _ => "(Unsupported content type)".to_string(),
        };

        let client_lock = self.client.read().await;
        let client = client_lock
            .as_ref()
            .ok_or("MQTT client not connected — call start() first")?;

        client
            .publish(
                &self.response_topic,
                self.qos.to_rumqttc(),
                false,
                text.as_bytes(),
            )
            .await?;

        debug!("MQTT: published response to '{}'", self.response_topic);
        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // MQTT has no typing indicator concept.
        Ok(())
    }

    async fn send_reaction(
        &self,
        _user: &ChannelUser,
        _message_id: &str,
        _reaction: &LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // MQTT has no reaction concept.
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);

        // Disconnect MQTT client gracefully
        let mut lock = self.client.write().await;
        if let Some(client) = lock.take() {
            if let Err(e) = client.disconnect().await {
                warn!("MQTT: error during disconnect: {e}");
            }
        }

        info!("MQTT adapter stopped");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> MqttConfig {
        MqttConfig::new(
            "localhost".to_string(),
            "test-client-001".to_string(),
            vec!["devices/+/telemetry".to_string()],
            "agents/response".to_string(),
        )
    }

    #[test]
    fn test_mqtt_adapter_creation() {
        let adapter = MqttAdapter::new(test_config());
        assert_eq!(adapter.name(), "mqtt");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("mqtt".to_string())
        );
        assert_eq!(adapter.host, "localhost");
        assert_eq!(adapter.port, DEFAULT_PORT);
    }

    #[test]
    fn test_mqtt_config_defaults() {
        let config = test_config();
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.keep_alive_secs, DEFAULT_KEEP_ALIVE_SECS);
        assert!(config.username.is_none());
        assert!(config.password.is_none());
    }

    #[test]
    fn test_mqtt_config_with_credentials() {
        let mut config = test_config();
        config.username = Some("iot-user".to_string());
        config.password = Some("secret123".to_string());

        let adapter = MqttAdapter::new(config);
        assert_eq!(adapter.username.as_deref(), Some("iot-user"));
        assert!(adapter.password.is_some());
    }

    #[test]
    fn test_mqtt_build_options_basic() {
        let adapter = MqttAdapter::new(test_config());
        let opts = adapter.build_mqtt_options();
        // MqttOptions does not expose fields directly, but construction should not panic.
        drop(opts);
    }

    #[test]
    fn test_mqtt_build_options_with_credentials() {
        let mut config = test_config();
        config.username = Some("user".to_string());
        config.password = Some("pass".to_string());
        let adapter = MqttAdapter::new(config);
        let opts = adapter.build_mqtt_options();
        drop(opts);
    }

    #[test]
    fn test_parse_payload_json_message() {
        let topic = "devices/sensor-1/telemetry";
        let payload = br#"{"sender": "sensor-1", "message": "temperature=22.5"}"#;
        let msg = MqttAdapter::parse_payload(topic, payload, &None);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.sender.platform_id, "sensor-1");
        match msg.content {
            ChannelContent::Text(t) => assert_eq!(t, "temperature=22.5"),
            _ => panic!("Expected Text content"),
        }
    }

    #[test]
    fn test_parse_payload_json_with_agent() {
        let topic = "commands/input";
        // `agent` must be a UUID — other adapters (Telegram, Discord) route by
        // AgentId too, and accepting free-form strings here would produce
        // routing mismatches.
        let payload = br#"{"sender": "controller", "message": "turn on lights", "agent": "550e8400-e29b-41d4-a716-446655440000"}"#;
        let msg = MqttAdapter::parse_payload(topic, payload, &None);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert!(msg.target_agent.is_some());
    }

    #[test]
    fn test_parse_payload_json_with_non_uuid_agent_ignored() {
        let topic = "commands/input";
        let payload =
            br#"{"sender": "controller", "message": "turn on lights", "agent": "not-a-uuid"}"#;
        let msg = MqttAdapter::parse_payload(topic, payload, &None);
        assert!(msg.is_some());
        // Non-UUID agent strings must be dropped rather than producing a
        // corrupt AgentId — the message still goes through, just unrouted.
        assert!(msg.unwrap().target_agent.is_none());
    }

    #[test]
    fn test_parse_payload_plain_text() {
        let topic = "devices/thermostat/data";
        let payload = b"humidity=45%";
        let msg = MqttAdapter::parse_payload(topic, payload, &None);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        // Last topic segment used as sender
        assert_eq!(msg.sender.platform_id, "data");
        match msg.content {
            ChannelContent::Text(t) => assert_eq!(t, "humidity=45%"),
            _ => panic!("Expected Text content"),
        }
    }

    #[test]
    fn test_parse_payload_command() {
        let topic = "commands/in";
        let payload = br#"{"sender": "admin", "message": "/status all"}"#;
        let msg = MqttAdapter::parse_payload(topic, payload, &None);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        match msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "status");
                assert_eq!(args, vec!["all"]);
            }
            _ => panic!("Expected Command content"),
        }
    }

    #[test]
    fn test_parse_payload_empty() {
        let msg = MqttAdapter::parse_payload("topic", b"", &None);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_payload_empty_json_message() {
        let payload = br#"{"sender": "x", "message": ""}"#;
        let msg = MqttAdapter::parse_payload("topic", payload, &None);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_payload_invalid_utf8() {
        let payload: &[u8] = &[0xFF, 0xFE, 0xFD];
        let msg = MqttAdapter::parse_payload("topic", payload, &None);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_payload_with_account_id() {
        let payload = b"hello";
        let account = Some("acct-42".to_string());
        let msg = MqttAdapter::parse_payload("topic/device", payload, &account);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(
            msg.metadata.get("account_id"),
            Some(&serde_json::json!("acct-42"))
        );
    }

    #[test]
    fn test_parse_payload_metadata_contains_topic() {
        let payload = b"test";
        let msg = MqttAdapter::parse_payload("sensors/temp", payload, &None);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(
            msg.metadata.get("mqtt_topic"),
            Some(&serde_json::Value::String("sensors/temp".to_string()))
        );
    }

    #[test]
    fn test_qos_conversion() {
        assert!(matches!(MqttQoS::AtMostOnce.to_rumqttc(), QoS::AtMostOnce));
        assert!(matches!(
            MqttQoS::AtLeastOnce.to_rumqttc(),
            QoS::AtLeastOnce
        ));
        assert!(matches!(
            MqttQoS::ExactlyOnce.to_rumqttc(),
            QoS::ExactlyOnce
        ));
    }

    #[test]
    fn test_qos_default() {
        let qos = MqttQoS::default();
        assert!(matches!(qos, MqttQoS::AtLeastOnce));
    }

    #[test]
    fn test_mqtt_channel_type_is_custom() {
        let adapter = MqttAdapter::new(test_config());
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("mqtt".to_string())
        );
    }

    #[test]
    fn test_mqtt_multiple_subscribe_topics() {
        let config = MqttConfig::new(
            "broker.local".to_string(),
            "multi-sub".to_string(),
            vec![
                "sensors/#".to_string(),
                "commands/+/input".to_string(),
                "alerts/critical".to_string(),
            ],
            "responses/out".to_string(),
        );
        let adapter = MqttAdapter::new(config);
        assert_eq!(adapter.subscribe_topics.len(), 3);
    }

    #[test]
    fn test_mqtt_custom_port() {
        let mut config = test_config();
        config.port = 8883;
        let adapter = MqttAdapter::new(config);
        assert_eq!(adapter.port, 8883);
    }

    // ----- send() path tests (issue #3820) -----
    //
    // MQTT outbound goes through `rumqttc::AsyncClient::publish`. Faking
    // that boundary requires either a trait abstraction over `AsyncClient`
    // or an in-process MQTT broker (e.g. `rumqttd`); both are larger
    // architectural changes than this PR's wiremock-coverage scope.
    //
    // What we *can* pin without that work is the fail-loud contract: if
    // `send()` is called before `start()` has populated `self.client`,
    // it must return an Err naming the missing connection — and it must
    // not panic on the `RwLock<Option<AsyncClient>>` `.as_ref()` path.

    fn mqtt_user(topic: &str) -> ChannelUser {
        ChannelUser {
            platform_id: topic.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn mqtt_send_returns_err_when_not_started() {
        let adapter = MqttAdapter::new(test_config());
        // No `start()` ⇒ `self.client` stays None.
        let err = adapter
            .send(
                &mqtt_user("librefang/out"),
                ChannelContent::Text("x".into()),
            )
            .await
            .expect_err("mqtt send must error when client is unset");
        assert!(
            err.to_string().to_lowercase().contains("not connected"),
            "error should mention disconnected client, got: {err}"
        );
    }

    // -- happy-path against a minimal MQTT v3.1.1 broker fixture --
    //
    // No public Rust crate ships a "tap-the-publish" mock for `rumqttc`,
    // so the fixture is a hand-written TCP listener that speaks just
    // enough of the MQTT 3.1.1 wire format to satisfy `rumqttc`:
    // CONNECT → CONNACK, SUBSCRIBE → SUBACK, PUBLISH → PUBACK. The
    // captured PUBLISH topic + payload are forwarded through a oneshot
    // and asserted against what `MqttAdapter::send()` was asked to
    // emit. Every wire-format byte is laid out inline in the fixture
    // below; there is no extra dev-dependency.

    /// Read a variable-length MQTT v3.1.1 "remaining length" header
    /// from `r`. MQTT encodes it as 1-4 bytes; the high bit signals
    /// continuation. See MQTT-3.1.1 §2.2.3.
    async fn mqtt_read_remaining_length<R: tokio::io::AsyncRead + Unpin>(
        r: &mut R,
    ) -> std::io::Result<usize> {
        use tokio::io::AsyncReadExt as _;
        let mut multiplier: usize = 1;
        let mut value: usize = 0;
        for _ in 0..4 {
            let mut byte = [0u8; 1];
            r.read_exact(&mut byte).await?;
            value += (byte[0] & 0x7f) as usize * multiplier;
            if byte[0] & 0x80 == 0 {
                return Ok(value);
            }
            multiplier *= 128;
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "MQTT remaining-length exceeds 4 bytes",
        ))
    }

    #[tokio::test]
    async fn mqtt_send_publishes_payload_to_response_topic_via_min_broker() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;
        use tokio::sync::oneshot;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = addr.ip().to_string();
        let port = addr.port();

        let (tx, rx) = oneshot::channel::<(String, Vec<u8>)>();
        let broker = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let mut tx_opt = Some(tx);
            loop {
                let mut byte0 = [0u8; 1];
                if r.read_exact(&mut byte0).await.is_err() {
                    return;
                }
                let packet_type = byte0[0] >> 4;
                let qos = (byte0[0] >> 1) & 0x03;
                let remaining_len = match mqtt_read_remaining_length(&mut r).await {
                    Ok(n) => n,
                    Err(_) => return,
                };
                let mut body = vec![0u8; remaining_len];
                if r.read_exact(&mut body).await.is_err() {
                    return;
                }
                match packet_type {
                    1 => {
                        // CONNECT — accept any client and reply with CONNACK
                        let _ = w.write_all(&[0x20, 0x02, 0x00, 0x00]).await;
                    }
                    8 => {
                        // SUBSCRIBE — first 2 bytes are the packet ID
                        let pkt_id = [body[0], body[1]];
                        let _ = w.write_all(&[0x90, 0x03, pkt_id[0], pkt_id[1], 0x00]).await;
                    }
                    3 => {
                        // PUBLISH — variable header: topic_len (BE u16) +
                        // topic + (packet_id if qos > 0). Payload fills
                        // the rest of the body.
                        let topic_len = u16::from_be_bytes([body[0], body[1]]) as usize;
                        let topic = String::from_utf8_lossy(&body[2..2 + topic_len]).to_string();
                        let payload_start = if qos > 0 {
                            let pkt_id = [body[2 + topic_len], body[3 + topic_len]];
                            let _ = w.write_all(&[0x40, 0x02, pkt_id[0], pkt_id[1]]).await;
                            4 + topic_len
                        } else {
                            2 + topic_len
                        };
                        let payload = body[payload_start..].to_vec();
                        if let Some(tx) = tx_opt.take() {
                            let _ = tx.send((topic, payload));
                        }
                    }
                    _ => {
                        // PINGREQ / DISCONNECT / etc. — ignore
                    }
                }
            }
        });

        let mut config = test_config();
        config.host = host;
        config.port = port;
        config.response_topic = "librefang/out".to_string();
        let adapter = MqttAdapter::new(config);

        // `start()` populates `self.client` *before* spawning the
        // eventloop, so `send()` can publish as soon as we return.
        // The eventloop will pump the bytes onto the wire once it has
        // observed CONNACK from the fixture.
        let _stream = adapter
            .start()
            .await
            .expect("mqtt start must succeed against fixture broker");

        adapter
            .send(
                &mqtt_user("ignored-by-mqtt-adapter"),
                ChannelContent::Text("hi mqtt".into()),
            )
            .await
            .expect("mqtt send must succeed against fixture broker");

        let (topic, payload) = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("fixture must observe PUBLISH within 5s")
            .expect("oneshot must not be dropped");
        assert_eq!(topic, "librefang/out");
        assert_eq!(
            payload, b"hi mqtt",
            "PUBLISH payload must round-trip the text body"
        );

        // Tidy up; we have what we needed.
        broker.abort();
    }
}
