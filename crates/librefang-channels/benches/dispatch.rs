//! Criterion benchmarks for channel message dispatch hot paths.
//!
//! Covers: message serialization/deserialization, channel routing,
//! and message formatting (markdown conversion).

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use chrono::Utc;
use librefang_channels::formatter::format_for_channel;
use librefang_channels::router::{AgentRouter, BindingContext};
use librefang_channels::types::{
    default_phase_emoji, split_message, AgentPhase, ChannelContent, ChannelMessage, ChannelType,
    ChannelUser,
};
use librefang_types::agent::AgentId;
use librefang_types::config::OutputFormat;
use std::borrow::Cow;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Message serialization / deserialization
// ---------------------------------------------------------------------------

fn make_sample_message() -> ChannelMessage {
    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: "msg-12345".to_string(),
        sender: ChannelUser {
            platform_id: "user-42".to_string(),
            display_name: "Alice".to_string(),
            librefang_user: None,
        },
        content: ChannelContent::Text("Hello, how can you help me today?".to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    }
}

fn bench_message_serialize(c: &mut Criterion) {
    let msg = make_sample_message();
    c.bench_function("message_serialize", |b| {
        b.iter(|| serde_json::to_string(black_box(&msg)).unwrap())
    });
}

fn bench_message_deserialize(c: &mut Criterion) {
    let msg = make_sample_message();
    let json = serde_json::to_string(&msg).unwrap();
    c.bench_function("message_deserialize", |b| {
        b.iter(|| serde_json::from_str::<ChannelMessage>(black_box(&json)).unwrap())
    });
}

fn bench_message_roundtrip(c: &mut Criterion) {
    let msg = make_sample_message();
    c.bench_function("message_roundtrip", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&msg)).unwrap();
            let _: ChannelMessage = serde_json::from_str(&json).unwrap();
        })
    });
}

// ---------------------------------------------------------------------------
// Channel routing / dispatch
// ---------------------------------------------------------------------------

fn bench_router_resolve_direct(c: &mut Criterion) {
    let mut router = AgentRouter::new();
    let agent = AgentId::new();
    router.set_default(agent);
    router.set_direct_route("Telegram".to_string(), "user-42".to_string(), agent);

    c.bench_function("router_resolve_direct", |b| {
        b.iter(|| {
            router.resolve(
                black_box(&ChannelType::Telegram),
                black_box("user-42"),
                black_box(None),
            )
        })
    });
}

fn bench_router_resolve_default(c: &mut Criterion) {
    let mut router = AgentRouter::new();
    let agent = AgentId::new();
    router.set_default(agent);

    c.bench_function("router_resolve_default_fallback", |b| {
        b.iter(|| {
            router.resolve(
                black_box(&ChannelType::Discord),
                black_box("unknown-user"),
                black_box(None),
            )
        })
    });
}

fn bench_router_resolve_with_bindings(c: &mut Criterion) {
    let router = AgentRouter::new();
    let agent = AgentId::new();
    router.register_agent("support".to_string(), agent);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "support".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            peer_id: Some("vip-user".to_string()),
            ..Default::default()
        },
    }]);

    c.bench_function("router_resolve_binding_match", |b| {
        b.iter(|| {
            router.resolve(
                black_box(&ChannelType::Telegram),
                black_box("vip-user"),
                black_box(None),
            )
        })
    });
}

fn bench_router_resolve_with_context(c: &mut Criterion) {
    let router = AgentRouter::new();
    let agent = AgentId::new();
    router.register_agent("admin-bot".to_string(), agent);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "admin-bot".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("discord".to_string()),
            guild_id: Some("guild-1".to_string()),
            roles: vec!["admin".to_string()],
            ..Default::default()
        },
    }]);

    let ctx = BindingContext {
        channel: Cow::Borrowed("discord"),
        peer_id: Cow::Borrowed("user-1"),
        guild_id: Some(Cow::Borrowed("guild-1")),
        roles: smallvec::smallvec![Cow::Borrowed("admin"), Cow::Borrowed("moderator")],
        ..Default::default()
    };

    c.bench_function("router_resolve_with_context", |b| {
        b.iter(|| {
            router.resolve_with_context(
                black_box(&ChannelType::Discord),
                black_box("user-1"),
                black_box(None),
                black_box(&ctx),
            )
        })
    });
}

// ---------------------------------------------------------------------------
// Message formatting (markdown conversion)
// ---------------------------------------------------------------------------

const SAMPLE_MARKDOWN: &str = "\
**LibreFang** is an *open-source* Agent OS.\n\
\n\
Features:\n\
- `multi-provider` LLM support\n\
- [A2A protocol](https://a2a.example.com) integration\n\
- **budget tracking** with per-agent cost limits\n\
\n\
Visit the [docs](https://docs.librefang.ai) for more info.";

const SHORT_TEXT: &str = "Hello world!";

fn bench_format_markdown_passthrough(c: &mut Criterion) {
    c.bench_function("format_markdown_passthrough", |b| {
        b.iter(|| format_for_channel(black_box(SAMPLE_MARKDOWN), OutputFormat::Markdown))
    });
}

fn bench_format_telegram_html(c: &mut Criterion) {
    c.bench_function("format_telegram_html", |b| {
        b.iter(|| format_for_channel(black_box(SAMPLE_MARKDOWN), OutputFormat::TelegramHtml))
    });
}

fn bench_format_slack_mrkdwn(c: &mut Criterion) {
    c.bench_function("format_slack_mrkdwn", |b| {
        b.iter(|| format_for_channel(black_box(SAMPLE_MARKDOWN), OutputFormat::SlackMrkdwn))
    });
}

fn bench_format_plain_text(c: &mut Criterion) {
    c.bench_function("format_plain_text", |b| {
        b.iter(|| format_for_channel(black_box(SAMPLE_MARKDOWN), OutputFormat::PlainText))
    });
}

fn bench_format_short_text(c: &mut Criterion) {
    c.bench_function("format_telegram_html_short", |b| {
        b.iter(|| format_for_channel(black_box(SHORT_TEXT), OutputFormat::TelegramHtml))
    });
}

fn bench_split_message_short(c: &mut Criterion) {
    c.bench_function("split_message_short", |b| {
        b.iter(|| split_message(black_box("Hello!"), 4096))
    });
}

fn bench_split_message_long(c: &mut Criterion) {
    let long_text = "Line of text here.\n".repeat(500);
    c.bench_function("split_message_long", |b| {
        b.iter(|| split_message(black_box(&long_text), 4096))
    });
}

fn bench_default_phase_emoji(c: &mut Criterion) {
    let phases = [
        AgentPhase::Queued,
        AgentPhase::Thinking,
        AgentPhase::tool_use("web_fetch"),
        AgentPhase::Streaming,
        AgentPhase::Done,
        AgentPhase::Error,
    ];
    c.bench_function("default_phase_emoji_all", |b| {
        b.iter(|| {
            for phase in &phases {
                black_box(default_phase_emoji(phase));
            }
        })
    });
}

// ---------------------------------------------------------------------------
// Criterion groups
// ---------------------------------------------------------------------------

criterion_group!(
    serialization,
    bench_message_serialize,
    bench_message_deserialize,
    bench_message_roundtrip,
);

criterion_group!(
    routing,
    bench_router_resolve_direct,
    bench_router_resolve_default,
    bench_router_resolve_with_bindings,
    bench_router_resolve_with_context,
);

criterion_group!(
    formatting,
    bench_format_markdown_passthrough,
    bench_format_telegram_html,
    bench_format_slack_mrkdwn,
    bench_format_plain_text,
    bench_format_short_text,
    bench_split_message_short,
    bench_split_message_long,
    bench_default_phase_emoji,
);

criterion_main!(serialization, routing, formatting);
