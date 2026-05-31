//! Sidecar channel adapter — runs an external process that communicates via JSON-RPC over stdin/stdout.
//!
//! This allows external processes written in any language (Python, Go, JS, etc.)
//! to act as channel adapters without touching Rust code. Communication uses
//! newline-delimited JSON (one JSON object per line) over stdin/stdout.

use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
    GroupMember, InteractiveMessage, LifecycleReaction, ParticipantRef, TypingEvent,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tracing::{debug, error, info, warn};

/// Cap on a single inbound event line from a sidecar adapter. The inbound
/// stream was previously read with the unbounded `lines()`, so a runaway
/// adapter emitting bytes without a newline could grow the reader's buffer
/// without bound and OOM the daemon. 64 MiB is far above any real platform
/// event (media is referenced, not inlined); an over-cap line drops the
/// connection and the supervisor restarts the adapter.
const MAX_EVENT_LINE_BYTES: usize = 64 * 1024 * 1024;

/// Read one event line via the shared bounded reader, mapped back to the
/// `AsyncBufReadExt::next_line()` shape the reader's `select!` already matches
/// on (`Ok(Some)` / `Ok(None)` / `Err`). An over-cap line surfaces as an error
/// so the existing error arm tears the connection down and respawns.
async fn read_event_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    match librefang_subprocess::read_capped_line(reader, buf, MAX_EVENT_LINE_BYTES).await? {
        librefang_subprocess::Line::Data(s) => Ok(Some(s)),
        librefang_subprocess::Line::Eof => Ok(None),
        librefang_subprocess::Line::TooLong => Err(std::io::Error::other(
            "sidecar event line exceeded the 64 MiB cap",
        )),
    }
}

/// Deserialize `T`, mapping an explicit JSON `null` to `T::default()`.
///
/// `#[serde(default)]` alone only covers an *omitted* field; a present
/// `"params": null` (emitted by many JSON-RPC libraries for no-arg
/// notifications) would otherwise fail to deserialize into a struct and
/// the whole event would be dropped.
fn de_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

// ── JSON-RPC Protocol Types ────────────────────────────────────────

/// Messages from the sidecar process TO LibreFang (one JSON per line on stdout).
#[derive(Debug, Deserialize)]
#[serde(tag = "method")]
pub enum SidecarEvent {
    /// A new message received from the platform.
    ///
    /// Boxed: `SidecarMessageParams` carries full `ChannelContent` +
    /// group rosters, so it dwarfs the other variants
    /// (clippy::large_enum_variant). Box keeps `SidecarEvent` small;
    /// serde and field access (incl. partial moves) are transparent.
    #[serde(rename = "message")]
    Message { params: Box<SidecarMessageParams> },
    /// Adapter is ready to receive commands. Carries the declared
    /// capability set + identity metadata. Both the bare legacy form
    /// `{"method":"ready"}` (field omitted) and the JSON-RPC
    /// `{"method":"ready","params":null}` form parse to defaults.
    #[serde(rename = "ready")]
    Ready {
        #[serde(default, deserialize_with = "de_null_default")]
        params: SidecarReadyParams,
    },
    /// Adapter encountered an error.
    #[serde(rename = "error")]
    Error { params: SidecarErrorParams },
    /// A typing indicator from the platform.
    ///
    /// P0 skeleton: not yet wired through to `ChannelAdapter::typing_events`
    /// — that happens in P2. Present now so external adapters can be
    /// developed against the final wire shape.
    #[serde(rename = "typing")]
    Typing { params: SidecarTypingParams },
    /// Sidecar has fetched a QR-login code and is waiting for the
    /// user to scan it (WeChat iLink, WhatsApp Web pairing, …). The
    /// daemon caches this on `ChannelStatus.qr` so
    /// `GET /api/channels/{name}/qr` can return it to the dashboard
    /// without requiring the operator to read sidecar logs.
    #[serde(rename = "qr_ready")]
    QrReady { params: SidecarQrReadyParams },
    /// Sidecar reports a state transition on the active QR session
    /// (scanning → confirmed / expired / failed). Replaces the
    /// retired `/api/channels/{wechat,whatsapp}/qr/status` long-poll
    /// the dashboard used pre-sidecar.
    #[serde(rename = "qr_status")]
    QrStatus { params: SidecarQrStatusParams },
}

#[derive(Debug, Deserialize)]
pub struct SidecarMessageParams {
    pub user_id: String,
    pub user_name: String,
    pub text: Option<String>,
    pub channel_id: Option<String>,
    pub platform: Option<String>,
    /// The platform's *native* message id. Stored as
    /// `ChannelMessage.platform_message_id` so lifecycle features
    /// (`send_reaction`, edits) target the real message. Absent ⇒ a
    /// UUID is generated (legacy behaviour; reactions won't resolve).
    #[serde(default)]
    pub message_id: Option<String>,
    /// Full structured content. When present, supersedes `text`.
    /// Legacy text-only adapters omit this and keep working.
    #[serde(default)]
    pub content: Option<ChannelContent>,
    /// Sender `@handle` if the platform exposes one. Folded into
    /// message metadata — `ChannelUser` has no handle slot, and
    /// routing/identity is the bridge's concern, not the adapter's.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional mapping to a LibreFang user identity.
    #[serde(default)]
    pub librefang_user: Option<String>,
    /// Whether this message came from a group chat (vs DM).
    #[serde(default)]
    pub is_group: bool,
    /// Thread / reply-to identifier, if any.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Group roster, folded into metadata. The bridge owns
    /// `SenderContext`; the adapter only transports the data.
    #[serde(default)]
    pub group_members: Vec<GroupMember>,
    /// Group participant refs, folded into metadata.
    #[serde(default)]
    pub group_participants: Vec<ParticipantRef>,
    /// Free-form metadata merged into the `ChannelMessage` metadata map.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SidecarErrorParams {
    pub message: String,
}

/// `qr_ready` event params — see [`QrState`](crate::types::QrState)
/// for the daemon-side projection and lifecycle.
#[derive(Debug, Deserialize)]
pub struct SidecarQrReadyParams {
    /// Raw QR payload the user's scanner reads.
    pub qr_code: String,
    /// Optional pre-formed scan URL (deep-link). When `None` the
    /// dashboard renders `qr_code` directly.
    #[serde(default)]
    pub qr_url: Option<String>,
    /// Optional operator-visible note (e.g. "5-minute window").
    #[serde(default)]
    pub message: Option<String>,
    /// Optional expiry timestamp from the platform's response.
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

/// `qr_status` event params — drives transitions out of `Pending`.
///
/// A previous iteration also carried a `bot_token` field for the
/// dashboard to auto-persist on `confirmed`. That path was unsafe
/// against the current configure endpoint (see `types.rs::QrState`
/// for the full note) so the field was dropped from the protocol
/// entirely. The sidecar logs the captured token at DEBUG instead
/// and the operator copies it into `secrets.env`.
#[derive(Debug, Deserialize)]
pub struct SidecarQrStatusParams {
    /// One of `scanning` / `confirmed` / `expired` / `failed`.
    /// Anything else is treated as `failed` so a sidecar bug can't
    /// strand the dashboard in `pending` forever.
    pub status: String,
    /// Operator-visible reason / next-step hint.
    #[serde(default)]
    pub message: Option<String>,
}

/// Inbound typing indicator params — fed to `typing_events()`.
#[derive(Debug, Deserialize)]
pub struct SidecarTypingParams {
    pub user_id: String,
    pub user_name: String,
    pub is_typing: bool,
}

/// Capability + identity payload an adapter declares in its `ready`
/// event. Every field is optional so the bare legacy
/// `{"method":"ready"}` still deserializes (all defaults).
///
/// `capabilities` strings gate the optional `ChannelAdapter` methods:
/// `typing`, `reaction`, `interactive`, `thread`, `streaming`,
/// `typing_events`. An adapter that declares nothing degrades to the
/// pre-P2 behaviour (plain text only).
#[derive(Debug, Default, Deserialize)]
pub struct SidecarReadyParams {
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub suppress_error_responses: bool,
    #[serde(default)]
    pub notification_recipients: Vec<ChannelUser>,
    /// Per-host header rules for `fetch_headers_for`. `(host, headers)`;
    /// auth is only emitted for URLs whose host matches exactly.
    #[serde(default)]
    pub header_rules: Vec<(String, Vec<(String, String)>)>,
    /// Reserved for skew diagnostics (logged, never enforced).
    #[serde(default)]
    pub protocol_version: Option<u32>,
}

/// Commands from LibreFang TO the sidecar process (one JSON per line on stdin).
#[derive(Debug, Serialize)]
#[serde(tag = "method")]
pub enum SidecarCommand {
    /// Send a message to the platform.
    #[serde(rename = "send")]
    Send { params: SidecarSendParams },
    /// Acknowledge a `ready` event so the adapter stops re-announcing.
    /// P0 skeleton — the ready/ack handshake is wired in P2.
    #[serde(rename = "ready_ack")]
    ReadyAck,
    /// Send a typing indicator to the platform.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "typing")]
    Typing { params: SidecarTypingCmdParams },
    /// Add a reaction to a platform message.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "reaction")]
    Reaction { params: SidecarReactionParams },
    /// Send an interactive (buttons) message.
    /// P0 skeleton — full button shape lands in P2.
    #[serde(rename = "interactive")]
    Interactive { params: SidecarInteractiveParams },
    /// Begin a streamed response.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "stream_start")]
    StreamStart { params: SidecarStreamStartParams },
    /// A chunk of a streamed response.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "stream_delta")]
    StreamDelta { params: SidecarStreamDeltaParams },
    /// End a streamed response.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "stream_end")]
    StreamEnd { params: SidecarStreamEndParams },
    /// Liveness ping.
    /// P0 skeleton — optional keepalive wired in P2.
    #[serde(rename = "heartbeat")]
    Heartbeat,
    /// Graceful shutdown request.
    #[serde(rename = "shutdown")]
    Shutdown,
}

#[derive(Debug, Serialize)]
pub struct SidecarSendParams {
    pub channel_id: String,
    /// Best-effort flattened text. Legacy adapters read only this;
    /// new adapters read the full `content`.
    pub text: String,
    /// Full structured content (every `ChannelContent` variant).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChannelContent>,
    /// Thread to reply into, if any. Populated by `send_in_thread`
    /// (wired in P2); plain `send` leaves it `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Full sender identity (`channel_id` is `user.platform_id`).
    pub user: ChannelUser,
}

/// `typing` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarTypingCmdParams {
    pub channel_id: String,
}

/// `reaction` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarReactionParams {
    pub channel_id: String,
    pub message_id: String,
    pub reaction: String,
}

/// `interactive` command params — full button shape.
#[derive(Debug, Serialize)]
pub struct SidecarInteractiveParams {
    pub channel_id: String,
    pub message: InteractiveMessage,
}

/// `stream_start` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarStreamStartParams {
    pub channel_id: String,
    pub stream_id: String,
    /// Thread to stream the reply into, if the inbound message was
    /// threaded. `None` for a top-level reply. Skipped when absent so
    /// adapters that ignore threads see the pre-thread wire shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// `stream_delta` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarStreamDeltaParams {
    pub stream_id: String,
    pub text: String,
}

/// `stream_end` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarStreamEndParams {
    pub stream_id: String,
}

// ── Sidecar Adapter Implementation ─────────────────────────────────

type StdinHandle = Arc<Mutex<Option<tokio::process::ChildStdin>>>;

/// Capability set + identity an adapter declared via its `ready` event.
/// Populated by the stdout reader; read by the cap-gated trait methods.
#[derive(Debug, Default)]
struct Caps {
    set: HashSet<String>,
    suppress_errors: bool,
    notification_recipients: Vec<ChannelUser>,
    header_rules: Vec<(String, Vec<(String, String)>)>,
}

/// Detect the canonical Python-side failure mode that fires when
/// `librefang-sdk` is not importable from the interpreter the daemon
/// spawned for the sidecar. We narrow on BOTH the `ModuleNotFoundError`
/// (or module-spec lookup) phrase AND the `librefang` token so a
/// random adapter that prints "librefang" in stderr for unrelated
/// reasons doesn't trip the install-hint translation and mask the
/// real bug. This is intentionally Python-shaped — non-Python
/// sidecars fall through to the raw passthrough.
///
/// Shared with `librefang-api::routes::sidecar_describe` (which
/// re-uses this single detector + `format_librefang_sdk_missing_hint`
/// so the discovery-time and runtime-time install hints stay in
/// lockstep). Keep both call sites in sync — if Python's traceback
/// format changes, update HERE only.
pub fn looks_like_librefang_sdk_missing(line: &str) -> bool {
    let module_not_found =
        line.contains("ModuleNotFoundError") && line.contains("No module named 'librefang'");
    let spec_lookup_failed =
        line.contains("Error while finding module specification for 'librefang.sidecar");
    module_not_found || spec_lookup_failed
}

/// Render the single canonical "install librefang-sdk" hint for a
/// given interpreter command. Used by both the boot-time discovery
/// translator (in `librefang-api`) and the runtime stderr loop (in
/// this crate) so operators see exactly the same message regardless
/// of which path tripped — and editing one updates both.
///
/// Single-quoted (not backticked) — the daemon WARN log channel
/// renders plain text, so markdown backticks appear literally to
/// operators and read worse than single quotes.
pub fn format_librefang_sdk_missing_hint(command: &str) -> String {
    format!(
        "librefang-sdk is not installed in the Python interpreter \
         resolved by '{command}'. Install with 'pip install \
         librefang-sdk' (or 'pip install -e sdk/python/' from a \
         source checkout). The daemon and your shell can resolve \
         different python3 binaries under mise / pyenv / conda — \
         verify with '{command} -c \"import librefang.sidecar; \
         print(librefang.__file__)\"'."
    )
}

/// What `StderrTranslator::handle_line` decided to do with a given
/// stderr line. Extracted from the inline `warn!`/`debug!` macro
/// calls so the WARN-then-DEBUG dedupe behavior is unit-testable
/// without spinning up a tracing subscriber.
#[derive(Debug, PartialEq, Eq)]
pub enum StderrAction {
    /// Emit at WARN level — the line is either the first
    /// install-hint-worthy crash signal or an unrelated stderr
    /// line worth surfacing to operators.
    Warn(String),
    /// Emit at DEBUG level — subsequent install-hint-worthy lines
    /// from the same crash (Python's traceback prints across 2-3
    /// lines and we don't want to triple-WARN per restart).
    Debug(String),
}

/// Per-spawn stderr line classifier. Owns the dedupe state for the
/// install-hint translation so the stderr-reader task in
/// `spawn_once` stays a thin loop and the WARN-vs-DEBUG decision
/// is testable in isolation.
pub struct StderrTranslator {
    /// Sidecar interpreter command (e.g. `python3`). Referenced in
    /// the install hint so operators see exactly which binary the
    /// daemon resolved.
    command: String,
    /// Flips to true after we emit the install hint once for this
    /// spawn. Subsequent matching lines from the same crash drop
    /// to DEBUG so a 3-line Python traceback doesn't fan out to 3
    /// identical WARNs per restart attempt.
    install_hint_emitted: bool,
}

impl StderrTranslator {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            install_hint_emitted: false,
        }
    }

    /// Classify one stderr line. Side-effecting (toggles internal
    /// dedupe state on the first install-hint match); idempotent
    /// thereafter on matching lines (always returns `Debug`).
    pub fn handle_line(&mut self, line: &str) -> StderrAction {
        if looks_like_librefang_sdk_missing(line) {
            if !self.install_hint_emitted {
                self.install_hint_emitted = true;
                return StderrAction::Warn(format!(
                    "[sidecar stderr] {}",
                    format_librefang_sdk_missing_hint(&self.command),
                ));
            }
            return StderrAction::Debug(format!("[sidecar stderr] {line}"));
        }
        StderrAction::Warn(format!("[sidecar stderr] {line}"))
    }
}

/// Write one newline-delimited JSON command to the child's stdin.
/// Shared by `SidecarAdapter::send_command` and the stdout reader
/// (which needs to emit `ReadyAck` without a `&self`).
async fn write_command(
    stdin_tx: &StdinHandle,
    cmd: &SidecarCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut guard = stdin_tx.lock().await;
    let stdin = guard
        .as_mut()
        .ok_or("Sidecar process stdin not available")?;
    let mut line = serde_json::to_string(cmd)?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

/// Extract the lowercased host from a URL string, stripping scheme,
/// userinfo and port. Returns `None` when there is no `://`.
///
/// IPv6 literal hosts (`https://[::1]:8443/`) are not parsed correctly
/// — the naive `:` split truncates at the first colon. The only
/// consumer is `fetch_headers_for`, which exact-matches against
/// adapter-declared `header_rules`; a mangled host simply fails to
/// match, so the failure mode is fail-closed (no auth header emitted),
/// never a credential leak. IPv6 hosts in `header_rules` are
/// unsupported, not unsafe.
fn url_host(url: &str) -> Option<String> {
    let after = url.split("://").nth(1)?;
    let authority = after.split('/').next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

// ── Supervision config ─────────────────────────────────────────────

use librefang_types::config::SidecarOverflowPolicy;

/// Per-adapter supervision tunables, snapshotted from
/// `SidecarChannelConfig` at construction. All scalar/Copy so the
/// supervisor can carry it cheaply across (re)spawns.
#[derive(Debug, Clone, Copy)]
struct SupCfg {
    restart: bool,
    initial_backoff_ms: u64,
    max_backoff_ms: u64,
    max_retries: u32,
    reset_after_secs: u64,
    ready_timeout_secs: u64,
    shutdown_grace_secs: u64,
    message_buffer: usize,
    overflow: SidecarOverflowPolicy,
}

impl SupCfg {
    fn from_config(c: &librefang_types::config::SidecarChannelConfig) -> Self {
        Self {
            restart: c.restart,
            initial_backoff_ms: c.restart_initial_backoff_ms,
            max_backoff_ms: c.restart_max_backoff_ms,
            max_retries: c.restart_max_retries,
            reset_after_secs: c.restart_reset_after_secs,
            ready_timeout_secs: c.ready_timeout_secs,
            shutdown_grace_secs: c.shutdown_grace_secs,
            message_buffer: c.message_buffer.max(1),
            overflow: c.overflow,
        }
    }
}

/// Why the stdout reader task ended — drives the supervisor's decision
/// to restart vs. stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReaderExit {
    /// stdout closed or a read error — the child is gone; restart it.
    ChildClosed,
    /// `stop()` signalled shutdown — do not restart.
    Shutdown,
    /// The bridge dropped the message stream — nothing to feed; stop.
    ReceiverGone,
}

/// Owned, cloneable context the supervisor re-uses for every (re)spawn.
/// `tokio::spawn` requires `'static`, so the supervisor can't borrow
/// `&self`; it owns clones of the adapter's shared (Arc/channel) state.
#[derive(Clone)]
struct SpawnCtx {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    /// Kernel home directory — used to locate `secrets.env` so that
    /// secrets saved by the dashboard after boot are visible to the
    /// spawned child (the boot-time `load_dotenv` Once-loader does not
    /// re-fire after a hot save).
    home_dir: PathBuf,
    channel_type: ChannelType,
    name: String,
    stdin_tx: StdinHandle,
    child: Arc<Mutex<Option<tokio::process::Child>>>,
    status: Arc<std::sync::Mutex<ChannelStatus>>,
    caps: Arc<RwLock<Caps>>,
    account_id_cell: Arc<OnceLock<Option<String>>>,
    typing_tx: mpsc::Sender<TypingEvent>,
    tx: mpsc::Sender<ChannelMessage>,
    shutdown_rx: watch::Receiver<bool>,
    sup: SupCfg,
}

/// Parse `secrets.env` at `path` into key/value pairs (best-effort).
///
/// Returns an empty `Vec` if the file is absent / unreadable — secrets.env
/// is an optional convenience file, and a missing file is not an error.
///
/// # Contract
///
/// Tolerates the lightweight dotenv conventions an operator hand-editing
/// `secrets.env` is likely to reach for:
/// - Surrounding whitespace on the line, key, and value is trimmed.
/// - One matched pair of outer single (`'`) or double (`"`) quotes
///   around the value is stripped. Quotes that aren't both leading
///   AND trailing (e.g. `KEY=a"b` or `KEY="abc`) are left as part of
///   the value verbatim.
/// - Blank lines and `#`-prefixed comments are skipped.
///
/// Does NOT process escape sequences (`\n`, `\t`, …) — that's a larger
/// surface than this best-effort reader is meant to cover. If an
/// operator needs escaped values they should set the env var via
/// `crate::secrets_env::upsert_secret` (which writes a known shape) or
/// export it from their shell.
fn parse_secrets_env(path: &Path) -> Vec<(String, String)> {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let k = trimmed[..eq].trim();
            let v_raw = trimmed[eq + 1..].trim();
            let v = strip_matching_outer_quotes(v_raw);
            if !k.is_empty() {
                out.push((k.to_string(), v.to_string()));
            }
        }
    }
    out
}

/// If `s` begins and ends with the SAME ASCII single (`'`) or double
/// (`"`) quote and has length ≥ 2, return the inner slice; otherwise
/// return `s` unchanged. A lone quote at one end, or mismatched quote
/// types (`"abc'`), are kept verbatim so we don't silently corrupt
/// values that legitimately contain a single quote character.
fn strip_matching_outer_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Build the final environment for the child by layering, in order:
///   1. `secrets.env` from `home_dir` — lowest priority. Each entry is
///      applied only when the parent process env does NOT already have
///      that key, matching the dotenv loader's "system env wins"
///      precedence (`librefang_extensions::dotenv`). The child inherits
///      the parent env by default, so we must avoid overwriting it.
///   2. `ctx_env` — explicit `[sidecar_channels.env]` from config.toml.
///      Wins over `secrets.env` (operator-explicit overrides), matching
///      the dotenv loader's precedence where explicit values dominate
///      the file-loaded fallback.
///
/// Returned list is the set of `(key, value)` pairs to apply via
/// `Command::env`, with both layers merged. The parent env is NOT
/// returned here — the spawned child already inherits it via
/// `Command::env_clear` not being called.
fn build_spawn_env(home_dir: &Path, ctx_env: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut merged: HashMap<String, String> = HashMap::new();
    let secrets_path = home_dir.join("secrets.env");
    for (k, v) in parse_secrets_env(&secrets_path) {
        // Parent process env wins (dotenv precedence).
        if std::env::var(&k).is_err() {
            merged.insert(k, v);
        }
    }
    // Explicit config.toml [sidecar_channels.env] wins over secrets.env.
    for (k, v) in ctx_env {
        merged.insert(k.clone(), v.clone());
    }
    merged.into_iter().collect()
}

/// Cheap, dependency-free jitter: 0..=20% of `base`, seeded off the
/// wall clock. Backoff jitter does not need a CSPRNG.
fn backoff_with_jitter(attempt: u32, initial_ms: u64, max_ms: u64) -> std::time::Duration {
    let exp = initial_ms.saturating_mul(1u64 << attempt.min(20));
    let base = exp.min(max_ms);
    let span = base / 5 + 1;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    std::time::Duration::from_millis(base + nanos % span)
}

/// Spawn the child once and wire stdin/stdout/stderr. Returns the
/// stdout-reader join handle (its `ReaderExit` tells the supervisor
/// why the child ended) and a oneshot that fires on the first `ready`.
async fn spawn_once(
    ctx: &SpawnCtx,
) -> Result<
    (tokio::task::JoinHandle<ReaderExit>, oneshot::Receiver<()>),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let mut cmd = Command::new(&ctx.command);
    cmd.args(&ctx.args);
    // Merge `secrets.env` (low precedence) and `ctx.env`
    // ([sidecar_channels.env] from config.toml, high precedence) so
    // secrets saved after boot — past the one-shot `load_dotenv`
    // loader — still reach the child without a daemon restart. The
    // parent process env is the highest precedence and the child
    // already inherits it via the default `Command` setup.
    let mut env_map: HashMap<String, String> = build_spawn_env(&ctx.home_dir, &ctx.env)
        .into_iter()
        .collect();
    // Embedded-SDK fallback: when the spawn command is a Python
    // interpreter that cannot already `import librefang.sidecar`, put
    // the daemon-bundled copy on PYTHONPATH so a fresh user with just
    // `python3` on PATH can enable a sidecar channel without first
    // running `pip install librefang-sdk`. No-op for developers whose
    // editable install already wins, and for non-Python commands
    // (`uv`, `bash`, …). See `embedded_sdk.rs` for precedence and
    // extraction details.
    let existing_pythonpath = env_map
        .get("PYTHONPATH")
        .cloned()
        .or_else(|| std::env::var("PYTHONPATH").ok());
    if let Some(composed) = crate::embedded_sdk::pythonpath_with_embedded(
        &ctx.command,
        &ctx.home_dir,
        existing_pythonpath.as_deref(),
    ) {
        env_map.insert("PYTHONPATH".to_string(), composed);
    }
    for (k, v) in &env_map {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "Failed to spawn sidecar '{}' ({}): {e}",
            ctx.name, ctx.command
        )
    })?;

    let child_stdin = child
        .stdin
        .take()
        .ok_or("Failed to capture sidecar stdin")?;
    {
        let mut guard = ctx.stdin_tx.lock().await;
        *guard = Some(child_stdin);
    }
    let child_stdout = child
        .stdout
        .take()
        .ok_or("Failed to capture sidecar stdout")?;
    let child_stderr = child
        .stderr
        .take()
        .ok_or("Failed to capture sidecar stderr")?;
    {
        let mut guard = ctx.child.lock().await;
        *guard = Some(child);
    }
    {
        let mut s = ctx.status.lock().unwrap_or_else(|e| e.into_inner());
        s.connected = true;
        s.started_at = Some(Utc::now());
    }

    let stderr_name = ctx.name.clone();
    let mut stderr_translator = StderrTranslator::new(&ctx.command);
    tokio::spawn(async move {
        let reader = BufReader::new(child_stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match stderr_translator.handle_line(&line) {
                StderrAction::Warn(msg) => {
                    warn!(adapter = %stderr_name, "{msg}");
                }
                StderrAction::Debug(msg) => {
                    debug!(adapter = %stderr_name, "{msg}");
                }
            }
        }
    });

    let (ready_tx, ready_rx) = oneshot::channel::<()>();
    let channel_type = ctx.channel_type.clone();
    let adapter_name = ctx.name.clone();
    let status_clone = ctx.status.clone();
    let caps = ctx.caps.clone();
    let account_id_cell = ctx.account_id_cell.clone();
    let reader_stdin = ctx.stdin_tx.clone();
    let typing_tx = ctx.typing_tx.clone();
    let tx = ctx.tx.clone();
    let overflow = ctx.sup.overflow;
    let mut shutdown_rx = ctx.shutdown_rx.clone();

    let handle = tokio::spawn(async move {
        let mut ready_tx = Some(ready_tx);
        let mut dropped: u64 = 0;
        let mut reader = BufReader::new(child_stdout);
        let mut event_buf: Vec<u8> = Vec::new();
        let exit;
        loop {
            tokio::select! {
                result = read_event_line(&mut reader, &mut event_buf) => {
                    match result {
                        Ok(Some(line)) => {
                            let line = line.trim().to_string();
                            if line.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<SidecarEvent>(&line) {
                                Ok(SidecarEvent::Ready { params }) => {
                                    let cap_count = params.capabilities.len();
                                    match caps.write() {
                                        Ok(mut g) => {
                                            g.set = params
                                                .capabilities
                                                .iter()
                                                .cloned()
                                                .collect();
                                            g.suppress_errors =
                                                params.suppress_error_responses;
                                            g.notification_recipients =
                                                params.notification_recipients.clone();
                                            g.header_rules =
                                                params.header_rules.clone();
                                        }
                                        Err(p) => {
                                            let mut g = p.into_inner();
                                            g.set = params
                                                .capabilities
                                                .iter()
                                                .cloned()
                                                .collect();
                                            g.suppress_errors =
                                                params.suppress_error_responses;
                                            g.notification_recipients =
                                                params.notification_recipients.clone();
                                            g.header_rules =
                                                params.header_rules.clone();
                                        }
                                    }
                                    let _ = account_id_cell
                                        .set(params.account_id.clone());
                                    info!(
                                        adapter = %adapter_name,
                                        capabilities = cap_count,
                                        protocol_version = params.protocol_version,
                                        "Sidecar adapter ready"
                                    );
                                    if let Some(t) = ready_tx.take() {
                                        let _ = t.send(());
                                    }
                                    if let Err(e) = write_command(
                                        &reader_stdin,
                                        &SidecarCommand::ReadyAck,
                                    )
                                    .await
                                    {
                                        debug!(
                                            adapter = %adapter_name,
                                            "Failed to send ReadyAck: {e}"
                                        );
                                    }
                                }
                                Ok(SidecarEvent::Typing { params }) => {
                                    let _ = typing_tx.try_send(TypingEvent {
                                        channel: channel_type.clone(),
                                        sender: ChannelUser {
                                            platform_id: params.user_id,
                                            display_name: params.user_name,
                                            librefang_user: None,
                                        },
                                        is_typing: params.is_typing,
                                    });
                                }
                                Ok(SidecarEvent::Message { params }) => {
                                    let params = *params;
                                    debug!(
                                        adapter = %adapter_name,
                                        user = %params.user_name,
                                        "Received message from sidecar"
                                    );
                                    let mut metadata = params.metadata;
                                    // #5227 follow-up — sidecar protocol
                                    // splits `user_id` (the human sender)
                                    // and `channel_id` (the chat the
                                    // message belongs to). The bridge's
                                    // `build_sender_context` derives
                                    // `chat_id` from `sender.platform_id`
                                    // (see `bridge.rs` near
                                    // `build_sender_context`) and reads
                                    // the human sender from
                                    // `metadata[SENDER_USER_ID_KEY]`
                                    // (falling back to `platform_id`). For
                                    // the in-process Discord adapter
                                    // `platform_id` is already the chat
                                    // id; sidecar adapters must mirror
                                    // that shape so the cross-chat scope
                                    // composition (`compose_sender_scope`)
                                    // sees a DM and a group of the same
                                    // user as DISTINCT chats. Without
                                    // this swap a Telegram-sidecar group
                                    // and DM for the same user collapse
                                    // to one scope (#5227 P3).
                                    let raw_chat_id = params.channel_id.clone();
                                    if let Some(ch) = params.channel_id {
                                        metadata.insert(
                                            "channel_id".to_string(),
                                            serde_json::Value::String(ch),
                                        );
                                    }
                                    if let Some(p) = params.platform {
                                        metadata.insert(
                                            "platform".to_string(),
                                            serde_json::Value::String(p),
                                        );
                                    }
                                    if let Some(h) = params.username {
                                        // `sender_username` is the key the
                                        // bridge reads when building
                                        // `SenderContext` and upserting the
                                        // group roster; `"username"` was a
                                        // dead key the bridge never consumed.
                                        metadata.insert(
                                            "sender_username".to_string(),
                                            serde_json::Value::String(h),
                                        );
                                    }
                                    if !params.group_members.is_empty() {
                                        if let Ok(v) = serde_json::to_value(
                                            &params.group_members,
                                        ) {
                                            metadata.insert(
                                                "group_members".to_string(),
                                                v,
                                            );
                                        }
                                    }
                                    if !params.group_participants.is_empty() {
                                        if let Ok(v) = serde_json::to_value(
                                            &params.group_participants,
                                        ) {
                                            metadata.insert(
                                                "group_participants".to_string(),
                                                v,
                                            );
                                        }
                                    }
                                    let content = params
                                        .content
                                        .unwrap_or_else(|| {
                                            ChannelContent::Text(
                                                params.text.unwrap_or_default(),
                                            )
                                        });
                                    let (platform_id, sender_user_id_meta) =
                                        derive_sidecar_sender_identity(
                                            &params.user_id,
                                            raw_chat_id.as_deref(),
                                        );
                                    if let Some(uid) = sender_user_id_meta {
                                        // Only stamp if the upstream sidecar
                                        // hasn't already populated this key
                                        // (the Telegram poll-answer path
                                        // does — see `_poll_answer_to_event`
                                        // in `telegram.py`).
                                        metadata
                                            .entry(
                                                crate::bridge::SENDER_USER_ID_KEY.to_string(),
                                            )
                                            .or_insert(serde_json::Value::String(uid));
                                    }
                                    let msg = ChannelMessage {
                                        channel: channel_type.clone(),
                                        platform_message_id: params
                                            .message_id
                                            .unwrap_or_else(|| {
                                                uuid::Uuid::new_v4()
                                                    .to_string()
                                            }),
                                        sender: ChannelUser {
                                            platform_id,
                                            display_name: params.user_name,
                                            librefang_user: params.librefang_user,
                                        },
                                        content,
                                        target_agent: None,
                                        timestamp: Utc::now(),
                                        is_group: params.is_group,
                                        thread_id: params.thread_id,
                                        metadata,
                                    };
                                    {
                                        let mut s = status_clone
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner());
                                        s.messages_received += 1;
                                        s.last_message_at = Some(Utc::now());
                                    }
                                    match overflow {
                                        SidecarOverflowPolicy::Block => {
                                            if tx.send(msg).await.is_err() {
                                                debug!(
                                                    adapter = %adapter_name,
                                                    "Message receiver dropped"
                                                );
                                                exit = ReaderExit::ReceiverGone;
                                                break;
                                            }
                                        }
                                        SidecarOverflowPolicy::DropNewest => {
                                            use tokio::sync::mpsc::error::TrySendError;
                                            match tx.try_send(msg) {
                                                Ok(()) => {}
                                                Err(TrySendError::Closed(_)) => {
                                                    debug!(
                                                        adapter = %adapter_name,
                                                        "Message receiver dropped"
                                                    );
                                                    exit =
                                                        ReaderExit::ReceiverGone;
                                                    break;
                                                }
                                                Err(TrySendError::Full(_)) => {
                                                    dropped += 1;
                                                    // Rate-limited: first, then
                                                    // every 100th, so a flooded
                                                    // notification sidecar can't
                                                    // spam the log.
                                                    if dropped == 1
                                                        || dropped
                                                            .is_multiple_of(100)
                                                    {
                                                        warn!(
                                                            adapter = %adapter_name,
                                                            dropped,
                                                            "Inbound buffer full; dropping message (overflow=drop_newest)"
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(SidecarEvent::Error { params }) => {
                                    warn!(
                                        adapter = %adapter_name,
                                        error = %params.message,
                                        "Sidecar adapter reported error"
                                    );
                                    let mut s = status_clone
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    s.last_error = Some(params.message);
                                }
                                Ok(SidecarEvent::QrReady { params }) => {
                                    info!(
                                        adapter = %adapter_name,
                                        has_url = params.qr_url.is_some(),
                                        "Sidecar published QR for login"
                                    );
                                    let mut s = status_clone
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    s.qr = Some(crate::types::QrState {
                                        status: crate::types::QrStatusKind::Pending,
                                        qr_code: params.qr_code,
                                        qr_url: params.qr_url,
                                        message: params.message,
                                        expires_at: params.expires_at,
                                        updated_at: Utc::now(),
                                    });
                                }
                                Ok(SidecarEvent::QrStatus { params }) => {
                                    let kind = match params.status.as_str() {
                                        "pending" => {
                                            crate::types::QrStatusKind::Pending
                                        }
                                        "scanning" => {
                                            crate::types::QrStatusKind::Scanning
                                        }
                                        "confirmed" => {
                                            crate::types::QrStatusKind::Confirmed
                                        }
                                        "expired" => {
                                            crate::types::QrStatusKind::Expired
                                        }
                                        // Anything off-protocol is treated as
                                        // terminal failure so the dashboard
                                        // doesn't spin forever on a sidecar
                                        // bug that emits a misspelled status.
                                        _ => crate::types::QrStatusKind::Failed,
                                    };
                                    info!(
                                        adapter = %adapter_name,
                                        status = ?kind,
                                        "QR session transitioned"
                                    );
                                    let mut s = status_clone
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    // Update in place when we already have a
                                    // session — keeps qr_code / qr_url stable
                                    // across transitions. Drop the cached
                                    // session entirely on a terminal-failure
                                    // path so a stale, scanned-already QR
                                    // doesn't leak into a future fresh login.
                                    match s.qr.as_mut() {
                                        Some(q) => {
                                            q.status = kind;
                                            if params.message.is_some() {
                                                q.message = params.message;
                                            }
                                            q.updated_at = Utc::now();
                                        }
                                        None => {
                                            // qr_status before qr_ready is
                                            // technically protocol misuse, but
                                            // we accept it so a sidecar that
                                            // restarts mid-flow can still
                                            // surface its state. qr_code is
                                            // empty because we never saw the
                                            // payload; the dashboard treats
                                            // empty as "no scannable image".
                                            s.qr = Some(crate::types::QrState {
                                                status: kind,
                                                qr_code: String::new(),
                                                qr_url: None,
                                                message: params.message,
                                                expires_at: None,
                                                updated_at: Utc::now(),
                                            });
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        adapter = %adapter_name,
                                        line = %line,
                                        "Failed to parse sidecar event: {e}"
                                    );
                                }
                            }
                        }
                        Ok(None) => {
                            info!(
                                adapter = %adapter_name,
                                "Sidecar process stdout closed"
                            );
                            let mut s = status_clone
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            s.connected = false;
                            exit = ReaderExit::ChildClosed;
                            break;
                        }
                        Err(e) => {
                            error!(
                                adapter = %adapter_name,
                                "Error reading sidecar stdout: {e}"
                            );
                            let mut s = status_clone
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            s.connected = false;
                            s.last_error =
                                Some(format!("stdout read error: {e}"));
                            exit = ReaderExit::ChildClosed;
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!(
                        adapter = %adapter_name,
                        "Sidecar reader received shutdown signal"
                    );
                    exit = ReaderExit::Shutdown;
                    break;
                }
            }
        }
        exit
    });

    Ok((handle, ready_rx))
}

/// Circuit-breaker: restarts exhausted. Logged exactly once (the
/// supervisor breaks right after), so no log-rate gate is needed.
///
/// Preserves the most recent per-attempt `last_error` (spawn error,
/// sidecar-emitted `error` event, or stdout read failure) by composing
/// it into the final `last_error` and surfacing it on the structured
/// log as `last_cause`. The earlier behaviour overwrote the specific
/// cause with a generic "circuit-breaker tripped" message, leaving
/// `GET /api/channels` / dashboard operators with no actionable signal
/// when a sidecar had been retrying-and-failing for the same reason
/// (missing required env var, bad token, etc.) all along.
fn trip_circuit(ctx: &SpawnCtx, attempt: u32) {
    let prior = {
        let mut s = ctx.status.lock().unwrap_or_else(|e| e.into_inner());
        s.connected = false;
        let prior = s.last_error.clone();
        s.last_error = Some(match prior.as_deref() {
            Some(cause) if !cause.is_empty() => format!(
                "sidecar restart circuit-breaker tripped after {attempt} attempts; \
                 last cause: {cause}"
            ),
            _ => format!("sidecar restart circuit-breaker tripped after {attempt} attempts"),
        });
        prior
    };
    error!(
        adapter = %ctx.name,
        attempt,
        max_retries = ctx.sup.max_retries,
        last_cause = prior.as_deref().unwrap_or("(none recorded)"),
        "Sidecar exceeded restart attempts; giving up (circuit-break)"
    );
}

/// A channel adapter that delegates to an external subprocess via JSON-RPC
/// over stdin/stdout.
pub struct SidecarAdapter {
    name: String,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    /// Kernel home directory — propagated into `SpawnCtx` so each
    /// (re)spawn can read `secrets.env` from the kernel's configured
    /// path. Must come from `KernelApi::home_dir` rather than a
    /// recomputed `LIBREFANG_HOME`/`~/.librefang` to honour custom
    /// `KernelConfig.home_dir` (see #5249's sidecar configure fix).
    home_dir: PathBuf,
    channel_type: ChannelType,
    /// Shared handle to the child's stdin for sending commands.
    stdin_tx: StdinHandle,
    /// Handle to the child process (kept alive to prevent kill_on_drop).
    child: Arc<Mutex<Option<tokio::process::Child>>>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Current status.
    status: Arc<std::sync::Mutex<ChannelStatus>>,
    /// Capabilities declared by the adapter's `ready` event.
    caps: Arc<RwLock<Caps>>,
    /// `account_id` from `ready` — set once, returned as `&str` by
    /// `account_id()` (a sync `&str` return can't borrow a lock guard).
    /// `OnceLock`: captured from the first `ready` only. A `ready` after
    /// a supervised restart cannot change it (the `set` is a no-op once
    /// initialized). This is intentional — `account_id` is stable
    /// adapter identity; a restarted child reporting a different id
    /// would indicate a misconfigured adapter, not a value to adopt.
    account_id_cell: Arc<OnceLock<Option<String>>>,
    /// Sender half feeding `typing_events()`. The reader pushes inbound
    /// `Typing` events here best-effort.
    typing_tx: mpsc::Sender<TypingEvent>,
    /// Receiver half, handed out once by `typing_events()` (sync — uses
    /// a std Mutex, never held across `.await`).
    typing_rx: Arc<std::sync::Mutex<Option<mpsc::Receiver<TypingEvent>>>>,
    /// Supervision tunables snapshotted from config at construction.
    sup: SupCfg,
    /// Per-instance channel behaviour overrides built from the
    /// `[[sidecar_channels]]` block — command policy + message
    /// coalescing (#5841). `None` when the block carries no such
    /// settings, so the bridge falls back to kernel-resolved overrides.
    overrides: Option<librefang_types::config::ChannelOverrides>,
}

/// Translate the command-policy + coalescing fields of a
/// `[[sidecar_channels]]` block into the `ChannelOverrides` the bridge
/// already understands (#5841). Returns `None` when every field is at
/// its default, so a plain sidecar config produces no override and the
/// bridge keeps falling back to the kernel-level lookup.
fn overrides_from_sidecar_config(
    config: &librefang_types::config::SidecarChannelConfig,
) -> Option<librefang_types::config::ChannelOverrides> {
    use librefang_types::config::SidecarCommandPolicy;

    let policy = config.command_policy;
    let coalesce_ms = config.message_coalesce_window_ms;

    let has_command_policy = policy != SidecarCommandPolicy::Allow;
    if !has_command_policy && coalesce_ms == 0 {
        return None;
    }

    let mut ov = librefang_types::config::ChannelOverrides::default();
    // Map the sidecar policy enum onto the existing boolean / list
    // fields `is_command_allowed` consults. Precedence inside the bridge
    // is `disable_commands` > `allowed_commands` > `blocked_commands`, so
    // exactly one of the three is populated per policy variant.
    match policy {
        SidecarCommandPolicy::Allow => {}
        SidecarCommandPolicy::Disable => ov.disable_commands = true,
        SidecarCommandPolicy::Allowlist => {
            // Fail closed: `allowlist` is an explicit default-deny intent. An
            // empty list means "honour no commands", so it must deny all, not
            // fall through to the allow-everything path `is_command_allowed`
            // takes when `allowed_commands` is empty. Map empty-allowlist onto
            // `disable_commands` (the highest-precedence deny) instead.
            if config.allowed_commands.is_empty() {
                warn!(
                    channel = %config.name,
                    "command_policy = \"allowlist\" with an empty allowed_commands list — denying all commands (fail-closed)"
                );
                ov.disable_commands = true;
            } else {
                ov.allowed_commands = config.allowed_commands.clone();
            }
        }
        SidecarCommandPolicy::Blocklist => {
            // An empty blocklist legitimately means "allow all, block nothing"
            // — intuitive and harmless — but it usually signals a forgotten
            // list, so surface it rather than silently allowing everything.
            if config.blocked_commands.is_empty() {
                warn!(
                    channel = %config.name,
                    "command_policy = \"blocklist\" with an empty blocked_commands list — all commands remain allowed"
                );
            }
            ov.blocked_commands = config.blocked_commands.clone();
        }
    }
    // The bridge's debouncer keys off `message_debounce_ms`; the
    // sidecar-facing name is `message_coalesce_window_ms`, matching the
    // pre-migration `[[channels.telegram]]` field operators knew (#4441).
    ov.message_debounce_ms = coalesce_ms;
    Some(ov)
}

impl SidecarAdapter {
    /// Create a new sidecar adapter from a config entry.
    ///
    /// `home_dir` MUST be the kernel's configured home directory
    /// (`KernelApi::home_dir`) so `secrets.env` resolution at spawn time
    /// matches the path the API layer writes to. Recomputing from
    /// `LIBREFANG_HOME`/`~/.librefang` here would silently diverge when
    /// the operator overrides `KernelConfig.home_dir`.
    pub fn new(config: &librefang_types::config::SidecarChannelConfig, home_dir: PathBuf) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let channel_type = config
            .channel_type
            .as_ref()
            .map(|s| ChannelType::Custom(s.clone()))
            .unwrap_or_else(|| ChannelType::Custom(config.name.clone()));
        let (typing_tx, typing_rx) = mpsc::channel::<TypingEvent>(64);

        Self {
            name: config.name.clone(),
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            home_dir,
            channel_type,
            stdin_tx: Arc::new(Mutex::new(None)),
            child: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(std::sync::Mutex::new(ChannelStatus::default())),
            caps: Arc::new(RwLock::new(Caps::default())),
            account_id_cell: Arc::new(OnceLock::new()),
            typing_tx,
            typing_rx: Arc::new(std::sync::Mutex::new(Some(typing_rx))),
            sup: SupCfg::from_config(config),
            overrides: overrides_from_sidecar_config(config),
        }
    }

    /// Whether the adapter declared capability `c` in its `ready` event.
    fn has_cap(&self, c: &str) -> bool {
        self.caps
            .read()
            .map(|g| g.set.contains(c))
            .unwrap_or_else(|p| p.into_inner().set.contains(c))
    }

    /// Write a command to the sidecar process stdin.
    async fn send_command(
        &self,
        cmd: &SidecarCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        write_command(&self.stdin_tx, cmd).await
    }
}

#[async_trait]
impl ChannelAdapter for SidecarAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    fn channel_overrides(&self) -> Option<librefang_types::config::ChannelOverrides> {
        self.overrides.clone()
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        info!(
            name = %self.name,
            command = %self.command,
            "Starting supervised sidecar channel adapter"
        );

        let (tx, rx) = mpsc::channel::<ChannelMessage>(self.sup.message_buffer);
        let ctx = SpawnCtx {
            command: self.command.clone(),
            args: self.args.clone(),
            env: self.env.clone(),
            home_dir: self.home_dir.clone(),
            channel_type: self.channel_type.clone(),
            name: self.name.clone(),
            stdin_tx: self.stdin_tx.clone(),
            child: self.child.clone(),
            status: self.status.clone(),
            caps: self.caps.clone(),
            account_id_cell: self.account_id_cell.clone(),
            typing_tx: self.typing_tx.clone(),
            tx: tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
            sup: self.sup,
        };
        let mut shutdown_rx = self.shutdown_rx.clone();

        // Supervisor: owns the (re)spawn loop. The returned stream
        // outlives every child — restarts feed the same `tx`. Restart
        // on crash with exponential backoff + jitter; circuit-break
        // after the configured max retries; never restart on a clean
        // shutdown, once the bridge dropped the stream, or when
        // `restart = false`.
        tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }
                let started = std::time::Instant::now();
                match spawn_once(&ctx).await {
                    Ok((handle, ready_rx)) => {
                        // Bound time-to-ready: a child that spawns but
                        // never announces still counts as a failed try.
                        let readied = tokio::select! {
                            _ = shutdown_rx.changed() => {
                                let _ = handle.await;
                                break;
                            }
                            r = ready_rx => r.is_ok(),
                            _ = tokio::time::sleep(
                                std::time::Duration::from_secs(
                                    ctx.sup.ready_timeout_secs,
                                ),
                            ) => false,
                        };
                        if !readied {
                            warn!(
                                adapter = %ctx.name,
                                timeout_secs = ctx.sup.ready_timeout_secs,
                                "Sidecar not ready in time; restarting"
                            );
                            {
                                let mut g = ctx.child.lock().await;
                                if let Some(mut c) = g.take() {
                                    let _ = c.kill().await;
                                }
                            }
                            let _ = handle.await;
                            if !ctx.sup.restart {
                                break;
                            }
                            if attempt >= ctx.sup.max_retries {
                                trip_circuit(&ctx, attempt);
                                break;
                            }
                            let delay = backoff_with_jitter(
                                attempt,
                                ctx.sup.initial_backoff_ms,
                                ctx.sup.max_backoff_ms,
                            );
                            attempt += 1;
                            tokio::select! {
                                _ = tokio::time::sleep(delay) => {}
                                _ = shutdown_rx.changed() => break,
                            }
                            continue;
                        }
                        let exit = handle.await.unwrap_or(ReaderExit::ChildClosed);
                        match exit {
                            ReaderExit::Shutdown | ReaderExit::ReceiverGone => break,
                            ReaderExit::ChildClosed => {
                                if *shutdown_rx.borrow() || ctx.tx.is_closed() || !ctx.sup.restart {
                                    break;
                                }
                                // Stable uptime resets backoff so a
                                // long-lived adapter that crashes once
                                // doesn't inherit an old penalty.
                                if started.elapsed()
                                    >= std::time::Duration::from_secs(ctx.sup.reset_after_secs)
                                {
                                    attempt = 0;
                                }
                                if attempt >= ctx.sup.max_retries {
                                    trip_circuit(&ctx, attempt);
                                    break;
                                }
                                let delay = backoff_with_jitter(
                                    attempt,
                                    ctx.sup.initial_backoff_ms,
                                    ctx.sup.max_backoff_ms,
                                );
                                attempt += 1;
                                warn!(
                                    adapter = %ctx.name,
                                    attempt,
                                    delay_ms = delay.as_millis(),
                                    "Sidecar exited; restarting after backoff"
                                );
                                tokio::select! {
                                    _ = tokio::time::sleep(delay) => {}
                                    _ = shutdown_rx.changed() => break,
                                }
                            }
                        }
                    }
                    Err(e) => {
                        {
                            let mut s = ctx.status.lock().unwrap_or_else(|e| e.into_inner());
                            s.connected = false;
                            s.last_error = Some(e.to_string());
                        }
                        if !ctx.sup.restart {
                            break;
                        }
                        if attempt >= ctx.sup.max_retries {
                            trip_circuit(&ctx, attempt);
                            break;
                        }
                        let delay = backoff_with_jitter(
                            attempt,
                            ctx.sup.initial_backoff_ms,
                            ctx.sup.max_backoff_ms,
                        );
                        attempt += 1;
                        warn!(
                            adapter = %ctx.name,
                            attempt,
                            delay_ms = delay.as_millis(),
                            "Sidecar spawn failed: {e}; retrying after backoff"
                        );
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {}
                            _ = shutdown_rx.changed() => break,
                        }
                    }
                }
            }
            debug!(adapter = %ctx.name, "Sidecar supervisor exiting");
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Legacy adapters read only `text`; flatten best-effort.
        // New adapters read the full structured `content`.
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            other => serde_json::to_string(other)?,
        };

        let cmd = SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: user.platform_id.clone(),
                text,
                content: Some(content),
                thread_id: None,
                user: user.clone(),
            },
        };
        self.send_command(&cmd).await?;

        // Update status
        {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            s.messages_sent += 1;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(name = %self.name, "Stopping sidecar channel adapter");

        // Send shutdown command (best-effort)
        let _ = self.send_command(&SidecarCommand::Shutdown).await;

        // Signal shutdown to the reader task
        let _ = self.shutdown_tx.send(true);

        // Close stdin to signal EOF
        {
            let mut guard = self.stdin_tx.lock().await;
            *guard = None;
        }

        // Wait briefly, then kill the child process
        {
            let mut guard = self.child.lock().await;
            if let Some(ref mut child) = *guard {
                // Give the process a moment to exit gracefully
                match tokio::time::timeout(
                    std::time::Duration::from_secs(self.sup.shutdown_grace_secs),
                    child.wait(),
                )
                .await
                {
                    Ok(Ok(status)) => {
                        debug!(name = %self.name, ?status, "Sidecar process exited");
                    }
                    _ => {
                        // Force kill if it didn't exit
                        let _ = child.kill().await;
                        debug!(name = %self.name, "Sidecar process killed");
                    }
                }
            }
            *guard = None;
        }

        // Update status
        {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            s.connected = false;
        }

        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    async fn send_typing(
        &self,
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.has_cap("typing") {
            return Ok(());
        }
        self.send_command(&SidecarCommand::Typing {
            params: SidecarTypingCmdParams {
                channel_id: user.platform_id.clone(),
            },
        })
        .await
    }

    fn fetch_headers_for(&self, url: &str) -> Vec<(String, String)> {
        let host = match url_host(url) {
            Some(h) => h,
            None => return Vec::new(),
        };
        let guard = self.caps.read().unwrap_or_else(|p| p.into_inner());
        // Only emit auth for an exact host the adapter declared — a
        // credential leak to a model-controlled host would let a forged
        // inbound message exfiltrate the token (see trait doc).
        for (rule_host, headers) in &guard.header_rules {
            if rule_host.to_ascii_lowercase() == host {
                return headers.clone();
            }
        }
        Vec::new()
    }

    async fn send_reaction(
        &self,
        user: &ChannelUser,
        message_id: &str,
        reaction: &LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.has_cap("reaction") {
            return Ok(());
        }
        self.send_command(&SidecarCommand::Reaction {
            params: SidecarReactionParams {
                channel_id: user.platform_id.clone(),
                message_id: message_id.to_string(),
                reaction: reaction.emoji.clone(),
            },
        })
        .await
    }

    async fn send_interactive(
        &self,
        user: &ChannelUser,
        message: &InteractiveMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.has_cap("interactive") {
            return self
                .send_command(&SidecarCommand::Interactive {
                    params: SidecarInteractiveParams {
                        channel_id: user.platform_id.clone(),
                        message: message.clone(),
                    },
                })
                .await;
        }
        // No cap: same degraded text render as the trait default.
        let mut text = message.text.clone();
        for row in &message.buttons {
            text.push('\n');
            for btn in row {
                text.push_str(&format!("  [{}]", btn.label));
            }
        }
        self.send(user, ChannelContent::Text(text)).await
    }

    fn suppress_error_responses(&self) -> bool {
        self.caps
            .read()
            .map(|g| g.suppress_errors)
            .unwrap_or_else(|p| p.into_inner().suppress_errors)
    }

    fn typing_events(&self) -> Option<mpsc::Receiver<TypingEvent>> {
        // NOT gated on `has_cap("typing_events")`: the bridge calls
        // this synchronously right after `start()`, but `ready` (which
        // populates caps) is processed asynchronously by the
        // supervisor, so the cap is almost never set yet here and the
        // bridge never asks again. Hand out the receiver
        // unconditionally; the stdout reader only ever forwards
        // `Typing` events the sidecar actually emits, so a sidecar
        // without typing simply leaves this receiver idle.
        self.typing_rx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.has_cap("thread") {
            return self.send(user, content).await;
        }
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            other => serde_json::to_string(other)?,
        };
        self.send_command(&SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: user.platform_id.clone(),
                text,
                content: Some(content),
                thread_id: Some(thread_id.to_string()),
                user: user.clone(),
            },
        })
        .await
    }

    fn owns_formatting(&self) -> bool {
        true
    }

    fn supports_streaming(&self) -> bool {
        self.has_cap("streaming")
    }

    async fn send_streaming(
        &self,
        user: &ChannelUser,
        mut delta_rx: mpsc::Receiver<String>,
        thread_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.has_cap("streaming") {
            // Default behaviour: collect all deltas, send once. Preserve
            // thread context — `send_in_thread` itself degrades to
            // `send` when the `thread` cap is also absent.
            let mut full_text = String::new();
            while let Some(delta) = delta_rx.recv().await {
                full_text.push_str(&delta);
            }
            if !full_text.is_empty() {
                let content = ChannelContent::Text(full_text);
                match thread_id {
                    Some(tid) => self.send_in_thread(user, content, tid).await?,
                    None => self.send(user, content).await?,
                }
            }
            return Ok(());
        }
        let stream_id = uuid::Uuid::new_v4().to_string();
        self.send_command(&SidecarCommand::StreamStart {
            params: SidecarStreamStartParams {
                channel_id: user.platform_id.clone(),
                stream_id: stream_id.clone(),
                thread_id: thread_id.map(|s| s.to_string()),
            },
        })
        .await?;
        while let Some(delta) = delta_rx.recv().await {
            self.send_command(&SidecarCommand::StreamDelta {
                params: SidecarStreamDeltaParams {
                    stream_id: stream_id.clone(),
                    text: delta,
                },
            })
            .await?;
        }
        self.send_command(&SidecarCommand::StreamEnd {
            params: SidecarStreamEndParams { stream_id },
        })
        .await
    }

    fn notification_recipients(&self) -> Vec<ChannelUser> {
        self.caps
            .read()
            .map(|g| g.notification_recipients.clone())
            .unwrap_or_else(|p| p.into_inner().notification_recipients.clone())
    }

    fn account_id(&self) -> Option<&str> {
        self.account_id_cell.get().and_then(|o| o.as_deref())
    }
}

/// Decide how to populate `sender.platform_id` and the optional
/// `SENDER_USER_ID_KEY` metadata entry for an inbound sidecar message.
///
/// **The bridge convention** (`bridge.rs::build_sender_context`) is:
/// - `sender.platform_id` carries the *chat* id (group id for groups,
///   user id for DMs — same as the in-process Discord/Slack adapters).
/// - The actual sender's user id lives in
///   `metadata[SENDER_USER_ID_KEY]`, with a fallback to `platform_id`
///   when the key is absent (DM case).
///
/// **The sidecar protocol** (Python adapters, see `protocol.message`)
/// splits the two as `user_id` (the human sender) and `channel_id`
/// (the chat). Naively copying `user_id` into `platform_id` would
/// collapse a Telegram-sidecar group and DM for the same user into one
/// chat scope — re-introducing the #5227 cross-chat bleed via a
/// different path.
///
/// Rule:
/// - Sidecar supplied a `channel_id` distinct from `user_id` → use
///   `channel_id` as `platform_id`, return `user_id` for the metadata
///   stamp.
/// - Sidecar supplied no `channel_id` (notifications: ntfy / gotify)
///   or chat == user (Telegram poll-answer DM where the adapter
///   pre-sets both fields, or any single-actor adapter) → keep the
///   pre-#5227 behaviour: `platform_id = user_id`, no metadata stamp.
///
/// Pure function so the chat-vs-user split has a unit-test
/// surface that doesn't require spinning up a sidecar process.
fn derive_sidecar_sender_identity(
    user_id: &str,
    channel_id: Option<&str>,
) -> (String, Option<String>) {
    match channel_id {
        Some(ch) if ch != user_id => (ch.to_string(), Some(user_id.to_string())),
        _ => (user_id.to_string(), None),
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InteractiveButton, MediaGroupItem};

    #[test]
    fn looks_like_librefang_sdk_missing_matches_canonical_traceback_lines() {
        // The two specific Python traceback shapes operators see
        // when librefang-sdk is not installed in the daemon's
        // interpreter. Both come straight from CPython's runpy.
        assert!(looks_like_librefang_sdk_missing(
            "/usr/bin/python3: Error while finding module specification \
             for 'librefang.sidecar.adapters.telegram' \
             (ModuleNotFoundError: No module named 'librefang')"
        ));
        assert!(looks_like_librefang_sdk_missing(
            "ModuleNotFoundError: No module named 'librefang'"
        ));
        assert!(looks_like_librefang_sdk_missing(
            "Error while finding module specification for \
             'librefang.sidecar.adapters.feishu'"
        ));
    }

    #[test]
    fn looks_like_librefang_sdk_missing_does_not_fire_on_unrelated_imports() {
        // An adapter's own typo'd ImportError must surface
        // verbatim — silencing it with the install hint would
        // mask the real bug.
        assert!(!looks_like_librefang_sdk_missing(
            "ImportError: cannot import name 'foo' from \
             'librefang.sidecar.adapters.telegram'"
        ));
        assert!(!looks_like_librefang_sdk_missing(
            "ModuleNotFoundError: No module named 'requests'"
        ));
        // The word "librefang" appearing in unrelated stderr
        // output must not by itself trigger the hint — the
        // detector requires BOTH the canonical phrase AND the
        // librefang token.
        assert!(!looks_like_librefang_sdk_missing(
            "[INFO] librefang sidecar adapter started normally"
        ));
        assert!(!looks_like_librefang_sdk_missing(""));
    }

    #[test]
    fn format_librefang_sdk_missing_hint_interpolates_command_twice() {
        // The hint references the resolved interpreter command in
        // BOTH the "resolved by" phrase AND the verification
        // snippet at the end. Both substitutions must use the
        // exact command string the daemon ran so operators can
        // copy-paste the verify command directly.
        let hint = format_librefang_sdk_missing_hint(
            "/Users/e-hu/.local/share/mise/installs/python/3.13.11/bin/python3",
        );
        assert!(
            hint.contains(
                "resolved by '/Users/e-hu/.local/share/mise/installs/python/3.13.11/bin/python3'"
            ),
            "hint must name the resolved interpreter in the first \
             sentence; got: {hint}"
        );
        assert!(
            hint.contains(
                "verify with '/Users/e-hu/.local/share/mise/installs/python/3.13.11/bin/python3 -c"
            ),
            "hint's verify snippet must use the same interpreter; \
             got: {hint}"
        );
        // No backticks (would render literally in plain-text log
        // channel) — single quotes only.
        assert!(
            !hint.contains('`'),
            "hint must not contain backticks; got: {hint}"
        );
    }

    #[test]
    fn stderr_translator_first_match_warns_subsequent_match_lines_debug() {
        // A 3-line Python traceback through the same crash must
        // emit exactly ONE WARN (the install hint) and route the
        // 2 trailing matching lines to DEBUG. Without the dedupe
        // the operator would see 3 identical WARNs per restart
        // attempt — and the restart loop fires every few seconds
        // by default.
        let mut t = StderrTranslator::new("python3");
        // CPython prints the spec-lookup line first, then the
        // bare ModuleNotFoundError, then sometimes a separator.
        let line_a = "/usr/bin/python3: Error while finding module specification \
                      for 'librefang.sidecar.adapters.telegram' \
                      (ModuleNotFoundError: No module named 'librefang')";
        let line_b = "ModuleNotFoundError: No module named 'librefang'";
        let line_c = "  File \"/usr/lib/python3.13/runpy.py\", line 198, in _run_module_as_main";
        let line_d = "ModuleNotFoundError: No module named 'librefang'";
        match t.handle_line(line_a) {
            StderrAction::Warn(msg) => {
                assert!(msg.contains("librefang-sdk is not installed"));
                assert!(msg.starts_with("[sidecar stderr] "));
            }
            other => panic!("first matching line must WARN; got {other:?}"),
        }
        // Second matching line → DEBUG (raw passthrough preserved
        // so a debug-level operator can see the full traceback).
        match t.handle_line(line_b) {
            StderrAction::Debug(msg) => {
                assert!(msg.contains("ModuleNotFoundError"));
                assert!(msg.starts_with("[sidecar stderr] "));
            }
            other => panic!("second matching line must DEBUG; got {other:?}"),
        }
        // Unrelated stderr line (a non-matching traceback frame)
        // is still WARN even after the install hint fired — only
        // matching lines drop to DEBUG.
        match t.handle_line(line_c) {
            StderrAction::Warn(msg) => {
                assert!(msg.contains("runpy.py"));
            }
            other => panic!("non-matching line must WARN; got {other:?}"),
        }
        // And a fourth matching line again → DEBUG.
        match t.handle_line(line_d) {
            StderrAction::Debug(_) => {}
            other => panic!("fourth matching line must DEBUG; got {other:?}"),
        }
    }

    #[test]
    fn stderr_translator_non_matching_lines_always_warn() {
        // An adapter that crashes for a NON-SDK reason (typo'd
        // import, missing required env, raised exception in the
        // adapter's own code) must surface every stderr line at
        // WARN so operators see the real failure verbatim.
        let mut t = StderrTranslator::new("python3");
        for line in [
            "Traceback (most recent call last):",
            "  File \"/path/adapter.py\", line 12, in <module>",
            "    import requests",
            "ModuleNotFoundError: No module named 'requests'",
            "ImportError: cannot import name 'foo'",
        ] {
            match t.handle_line(line) {
                StderrAction::Warn(_) => {}
                StderrAction::Debug(msg) => panic!(
                    "non-SDK stderr line was downgraded to DEBUG: \
                     {msg} (would hide real adapter bugs from operators)"
                ),
            }
        }
    }

    #[test]
    fn stderr_translator_uses_supplied_command_in_hint() {
        // The hint's "resolved by '<command>'" sentence must
        // reference the SAME command the translator was
        // constructed with, not a hardcoded 'python3' fallback.
        // Otherwise operators with mise / pyenv / conda see a
        // misleading hint that doesn't match the binary actually
        // failing.
        let mut t = StderrTranslator::new("/Users/alice/.pyenv/versions/3.13.0/bin/python3");
        match t.handle_line("ModuleNotFoundError: No module named 'librefang'") {
            StderrAction::Warn(msg) => {
                assert!(
                    msg.contains("/Users/alice/.pyenv/versions/3.13.0/bin/python3"),
                    "hint did not propagate the constructor's command; got: {msg}"
                );
            }
            other => panic!("expected WARN; got {other:?}"),
        }
    }

    /// #5227 follow-up — chat-vs-user identity split for sidecar
    /// messages. Telegram-sidecar group: distinct chat_id and user_id
    /// → swap so `platform_id` becomes the chat and the user_id is
    /// stamped under `SENDER_USER_ID_KEY`.
    #[test]
    fn derive_sidecar_sender_identity_group_swaps_chat_into_platform_id_5227() {
        let (platform_id, sender_meta) =
            derive_sidecar_sender_identity("alice-42", Some("-100123"));
        assert_eq!(
            platform_id, "-100123",
            "group chat id must become platform_id so build_sender_context \
             derives the right chat_id"
        );
        assert_eq!(
            sender_meta.as_deref(),
            Some("alice-42"),
            "actual sender user id must be returned for SENDER_USER_ID_KEY stamp"
        );
    }

    /// #5227 follow-up — DM where the upstream sidecar already set
    /// chat_id == user_id (Telegram poll-answer case). Must collapse to
    /// the pre-fix behaviour: no swap, no extra stamp.
    #[test]
    fn derive_sidecar_sender_identity_dm_collapses_5227() {
        let (platform_id, sender_meta) =
            derive_sidecar_sender_identity("alice-42", Some("alice-42"));
        assert_eq!(platform_id, "alice-42");
        assert!(sender_meta.is_none());
    }

    /// #5227 follow-up — adapters without a chat concept (ntfy /
    /// gotify notifications, custom single-actor sidecars). channel_id
    /// is None → keep the legacy behaviour, no swap.
    #[test]
    fn derive_sidecar_sender_identity_no_chat_id_legacy_5227() {
        let (platform_id, sender_meta) = derive_sidecar_sender_identity("topic-x", None);
        assert_eq!(platform_id, "topic-x");
        assert!(sender_meta.is_none());
    }

    // ── parse_secrets_env tolerance for hand-edited dotenv conventions ──

    fn write_tmp_secrets(contents: &str) -> tempfile::NamedTempFile {
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(f.path(), contents).expect("write secrets");
        f
    }

    #[test]
    fn parse_secrets_env_strips_double_quotes() {
        let f = write_tmp_secrets("KEY=\"value\"\n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), "value".to_string())]);
    }

    #[test]
    fn parse_secrets_env_strips_single_quotes() {
        let f = write_tmp_secrets("KEY='value'\n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), "value".to_string())]);
    }

    #[test]
    fn parse_secrets_env_keeps_internal_quotes() {
        // Not a paired outer pair — the quote is in the middle of the
        // value, so the contract says leave it verbatim.
        let f = write_tmp_secrets("KEY=a\"b\n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), "a\"b".to_string())]);
    }

    #[test]
    fn parse_secrets_env_keeps_mismatched_outer_quotes() {
        // `"abc'` — outer pair are different quote types, not a match.
        let f = write_tmp_secrets("KEY=\"abc'\n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), "\"abc'".to_string())]);
    }

    #[test]
    fn parse_secrets_env_trims_whitespace() {
        let f = write_tmp_secrets("  KEY = value  \n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), "value".to_string())]);
    }

    #[test]
    fn parse_secrets_env_handles_empty_quoted_value() {
        let f = write_tmp_secrets("KEY=\"\"\n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), String::new())]);
    }

    #[test]
    fn parse_secrets_env_skips_comments_and_blanks() {
        let f = write_tmp_secrets("# a comment\n\nKEY=value\n");
        let pairs = parse_secrets_env(f.path());
        assert_eq!(pairs, vec![("KEY".to_string(), "value".to_string())]);
    }

    #[test]
    fn test_sidecar_event_message_deserialization() {
        let json = r#"{"method":"message","params":{"user_id":"u1","user_name":"Alice","text":"Hello","channel_id":"ch1","platform":"test"}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Message { params } => {
                assert_eq!(params.user_id, "u1");
                assert_eq!(params.user_name, "Alice");
                assert_eq!(params.text, Some("Hello".to_string()));
                assert_eq!(params.channel_id, Some("ch1".to_string()));
                assert_eq!(params.platform, Some("test".to_string()));
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_sidecar_event_ready_deserialization() {
        // Bare legacy `ready` must still parse, with default params.
        let json = r#"{"method":"ready"}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Ready { params } => {
                assert!(params.capabilities.is_empty());
                assert!(params.account_id.is_none());
                assert!(!params.suppress_error_responses);
                assert!(params.notification_recipients.is_empty());
                assert!(params.header_rules.is_empty());
                assert!(params.protocol_version.is_none());
            }
            other => panic!("Expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_sidecar_event_ready_null_params_parses_to_default() {
        // JSON-RPC libraries emit `params: null` for no-arg
        // notifications. `#[serde(default)]` alone only covers an
        // omitted field; explicit null must also fold to defaults or
        // the supervisor never sees `ready` and churns on restart.
        for json in [
            r#"{"method":"ready","params":null}"#,
            r#"{"method":"ready"}"#,
        ] {
            match serde_json::from_str::<SidecarEvent>(json).unwrap() {
                SidecarEvent::Ready { params } => {
                    assert!(params.capabilities.is_empty());
                    assert!(params.account_id.is_none());
                    assert!(params.protocol_version.is_none());
                }
                other => panic!("Expected Ready for {json}, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_sidecar_event_ready_with_capabilities() {
        let json = r#"{"method":"ready","params":{
            "capabilities":["typing","streaming"],
            "account_id":"bot-1",
            "suppress_error_responses":true,
            "notification_recipients":[
                {"platform_id":"adm","display_name":"Admin","librefang_user":null}
            ],
            "header_rules":[["media.example.com",[["Authorization","Bearer x"]]]],
            "protocol_version":1
        }}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        let SidecarEvent::Ready { params } = event else {
            panic!("expected Ready");
        };
        assert_eq!(params.capabilities, vec!["typing", "streaming"]);
        assert_eq!(params.account_id.as_deref(), Some("bot-1"));
        assert!(params.suppress_error_responses);
        assert_eq!(params.notification_recipients.len(), 1);
        assert_eq!(params.header_rules.len(), 1);
        assert_eq!(params.header_rules[0].0, "media.example.com");
        assert_eq!(params.protocol_version, Some(1));
    }

    #[test]
    fn test_sidecar_event_error_deserialization() {
        let json = r#"{"method":"error","params":{"message":"Connection failed"}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Error { params } => {
                assert_eq!(params.message, "Connection failed");
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn qr_ready_event_parses_with_optional_fields() {
        // Minimal — only the required qr_code is set.
        let minimal = r#"{"method":"qr_ready","params":{"qr_code":"opaque-token"}}"#;
        let SidecarEvent::QrReady { params } =
            serde_json::from_str(minimal).expect("minimal qr_ready")
        else {
            panic!("expected QrReady variant");
        };
        assert_eq!(params.qr_code, "opaque-token");
        assert!(params.qr_url.is_none());
        assert!(params.message.is_none());
        assert!(params.expires_at.is_none());

        // Fully-populated form a real adapter would emit.
        let full = r#"{"method":"qr_ready","params":{
            "qr_code":"opaque-token",
            "qr_url":"https://platform.example.com/login?code=opaque-token",
            "message":"Scan within 5 minutes",
            "expires_at":"2099-01-01T00:00:00Z"
        }}"#;
        let SidecarEvent::QrReady { params } = serde_json::from_str(full).expect("full qr_ready")
        else {
            panic!("expected QrReady variant");
        };
        assert_eq!(params.qr_code, "opaque-token");
        assert_eq!(
            params.qr_url.as_deref(),
            Some("https://platform.example.com/login?code=opaque-token"),
        );
        assert_eq!(params.message.as_deref(), Some("Scan within 5 minutes"));
        assert!(params.expires_at.is_some());
    }

    #[test]
    fn qr_status_event_parses_all_documented_states() {
        for state in &["pending", "scanning", "confirmed", "expired", "failed"] {
            let json = format!(r#"{{"method":"qr_status","params":{{"status":"{state}"}}}}"#);
            let SidecarEvent::QrStatus { params } =
                serde_json::from_str(&json).expect("qr_status parse")
            else {
                panic!("expected QrStatus variant for state={state}");
            };
            assert_eq!(params.status, *state);
            assert!(params.message.is_none());
        }
    }

    #[test]
    fn qr_status_kind_serde_roundtrips_snake_case_on_the_wire() {
        // The wire-side spec for `QrStatusKind` is snake_case. A
        // dashboard or sidecar that emits `"Pending"` would not match,
        // and a future enum rename to PascalCase would silently break
        // every existing client — this test pins both directions.
        use crate::types::QrStatusKind;
        let cases = [
            (QrStatusKind::Pending, "\"pending\""),
            (QrStatusKind::Scanning, "\"scanning\""),
            (QrStatusKind::Confirmed, "\"confirmed\""),
            (QrStatusKind::Expired, "\"expired\""),
            (QrStatusKind::Failed, "\"failed\""),
        ];
        for (kind, wire) in cases {
            let serialized = serde_json::to_string(&kind).unwrap();
            assert_eq!(serialized, wire, "serialize {kind:?}");
            let deserialized: QrStatusKind = serde_json::from_str(wire).unwrap();
            assert_eq!(deserialized, kind, "deserialize {wire}");
        }
    }

    #[test]
    fn qr_state_field_skipped_when_absent_on_channel_status() {
        // The new `qr` field MUST `#[serde(skip_serializing_if =
        // "Option::is_none")]` so every non-QR channel keeps the
        // historical `ChannelStatus` JSON shape and dashboard / SDK
        // consumers that don't know about QR don't suddenly see a
        // `"qr": null` they have to filter.
        use crate::types::ChannelStatus;
        let status = ChannelStatus::default();
        let json = serde_json::to_string(&status).unwrap();
        assert!(
            !json.contains("qr"),
            "default ChannelStatus must not emit `qr` key, got: {json}"
        );
    }

    #[test]
    fn test_sidecar_event_message_minimal() {
        let json = r#"{"method":"message","params":{"user_id":"u1","user_name":"Bot"}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Message { params } => {
                assert_eq!(params.user_id, "u1");
                assert!(params.text.is_none());
                assert!(params.channel_id.is_none());
                assert!(params.platform.is_none());
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_sidecar_command_send_serialization() {
        let cmd = SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: "ch1".to_string(),
                text: "Hello world".to_string(),
                content: None,
                thread_id: None,
                user: ChannelUser {
                    platform_id: "ch1".to_string(),
                    display_name: "Tester".to_string(),
                    librefang_user: None,
                },
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""method":"send"#));
        assert!(json.contains(r#""channel_id":"ch1"#));
        assert!(json.contains(r#""text":"Hello world"#));
    }

    #[test]
    fn test_sidecar_command_shutdown_serialization() {
        let cmd = SidecarCommand::Shutdown;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""method":"shutdown"#));
    }

    #[test]
    fn test_sidecar_command_send_roundtrip() {
        let cmd = SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: "test-channel".to_string(),
                text: "Test message with \"quotes\" and \nnewlines".to_string(),
                content: None,
                thread_id: None,
                user: ChannelUser {
                    platform_id: "test-channel".to_string(),
                    display_name: "Tester".to_string(),
                    librefang_user: None,
                },
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        // Verify it's valid JSON that can be parsed back
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["method"], "send");
        assert_eq!(value["params"]["channel_id"], "test-channel");
    }

    // ── P0 skeleton: new protocol variant roundtrips ──────────────

    #[test]
    fn test_sidecar_event_typing_deserialization() {
        let json =
            r#"{"method":"typing","params":{"user_id":"u1","user_name":"Alice","is_typing":true}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Typing { params } => {
                assert_eq!(params.user_id, "u1");
                assert_eq!(params.user_name, "Alice");
                assert!(params.is_typing);
            }
            _ => panic!("Expected Typing variant"),
        }
    }

    #[test]
    fn test_legacy_events_still_parse_after_typing_added() {
        // Regression guard: adding SidecarEvent::Typing must not change
        // parsing of the pre-existing variants.
        assert!(matches!(
            serde_json::from_str::<SidecarEvent>(r#"{"method":"ready"}"#).unwrap(),
            SidecarEvent::Ready { .. }
        ));
        assert!(matches!(
            serde_json::from_str::<SidecarEvent>(
                r#"{"method":"message","params":{"user_id":"u","user_name":"n"}}"#
            )
            .unwrap(),
            SidecarEvent::Message { .. }
        ));
    }

    #[test]
    fn test_new_command_variants_serialize_with_distinct_tags() {
        let cmds = vec![
            SidecarCommand::ReadyAck,
            SidecarCommand::Typing {
                params: SidecarTypingCmdParams {
                    channel_id: "c".to_string(),
                },
            },
            SidecarCommand::Reaction {
                params: SidecarReactionParams {
                    channel_id: "c".to_string(),
                    message_id: "m".to_string(),
                    reaction: "👍".to_string(),
                },
            },
            SidecarCommand::Interactive {
                params: SidecarInteractiveParams {
                    channel_id: "c".to_string(),
                    message: InteractiveMessage {
                        text: "pick".to_string(),
                        buttons: vec![vec![InteractiveButton {
                            label: "Yes".to_string(),
                            action: "yes".to_string(),
                            style: None,
                            url: None,
                        }]],
                    },
                },
            },
            SidecarCommand::StreamStart {
                params: SidecarStreamStartParams {
                    channel_id: "c".to_string(),
                    stream_id: "s".to_string(),
                    thread_id: None,
                },
            },
            SidecarCommand::StreamDelta {
                params: SidecarStreamDeltaParams {
                    stream_id: "s".to_string(),
                    text: "chunk".to_string(),
                },
            },
            SidecarCommand::StreamEnd {
                params: SidecarStreamEndParams {
                    stream_id: "s".to_string(),
                },
            },
            SidecarCommand::Heartbeat,
        ];

        let mut tags = std::collections::BTreeSet::new();
        for cmd in &cmds {
            let v: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(cmd).unwrap()).unwrap();
            let tag = v["method"].as_str().unwrap().to_string();
            assert!(tags.insert(tag.clone()), "duplicate method tag: {tag}");
        }
        let expected: std::collections::BTreeSet<String> = [
            "ready_ack",
            "typing",
            "reaction",
            "interactive",
            "stream_start",
            "stream_delta",
            "stream_end",
            "heartbeat",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(tags, expected);
        // Legacy tags unchanged.
        assert_eq!(
            serde_json::to_string(&SidecarCommand::Shutdown).unwrap(),
            r#"{"method":"shutdown"}"#
        );
    }

    // ── P1: structured content I/O roundtrips ─────────────────────

    fn all_channel_content_variants() -> Vec<ChannelContent> {
        let btn = InteractiveButton {
            label: "Yes".to_string(),
            action: "yes".to_string(),
            style: Some("primary".to_string()),
            url: None,
        };
        vec![
            ChannelContent::Text("hello".to_string()),
            ChannelContent::Image {
                url: "https://x/i.png".to_string(),
                caption: Some("cap".to_string()),
                mime_type: Some("image/png".to_string()),
            },
            ChannelContent::File {
                url: "https://x/f.pdf".to_string(),
                filename: "f.pdf".to_string(),
            },
            ChannelContent::FileData {
                data: vec![1, 2, 3, 4],
                filename: "b.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
            },
            ChannelContent::Voice {
                url: "https://x/v.ogg".to_string(),
                caption: None,
                duration_seconds: 5,
            },
            ChannelContent::Video {
                url: "https://x/v.mp4".to_string(),
                caption: Some("c".to_string()),
                duration_seconds: 12,
                filename: Some("v.mp4".to_string()),
            },
            ChannelContent::Location {
                lat: 51.5,
                lon: -0.12,
            },
            ChannelContent::Command {
                name: "start".to_string(),
                args: vec!["a".to_string(), "b".to_string()],
            },
            ChannelContent::Interactive {
                text: "pick".to_string(),
                buttons: vec![vec![btn.clone()]],
            },
            ChannelContent::ButtonCallback {
                action: "yes".to_string(),
                message_text: Some("orig".to_string()),
            },
            ChannelContent::DeleteMessage {
                message_id: "m1".to_string(),
            },
            ChannelContent::EditInteractive {
                message_id: "m1".to_string(),
                text: "new".to_string(),
                buttons: vec![vec![btn.clone()]],
            },
            ChannelContent::Audio {
                url: "https://x/a.mp3".to_string(),
                caption: None,
                duration_seconds: 200,
                title: Some("Song".to_string()),
                performer: Some("Artist".to_string()),
            },
            ChannelContent::Animation {
                url: "https://x/a.gif".to_string(),
                caption: None,
                duration_seconds: 3,
            },
            ChannelContent::Sticker {
                file_id: "stk_1".to_string(),
            },
            ChannelContent::MediaGroup {
                items: vec![
                    MediaGroupItem::Photo {
                        url: "https://x/1.jpg".to_string(),
                        caption: Some("one".to_string()),
                    },
                    MediaGroupItem::Video {
                        url: "https://x/2.mp4".to_string(),
                        caption: None,
                        duration_seconds: 7,
                    },
                ],
            },
            ChannelContent::Poll {
                question: "Q?".to_string(),
                options: vec!["A".to_string(), "B".to_string()],
                is_quiz: true,
                correct_option_id: Some(1),
                explanation: Some("because".to_string()),
            },
            ChannelContent::PollAnswer {
                poll_id: "p1".to_string(),
                option_ids: vec![0, 1],
            },
        ]
    }

    #[test]
    fn test_inbound_content_roundtrip_all_variants() {
        for content in all_channel_content_variants() {
            let cv = serde_json::to_value(&content).unwrap();
            let msg = serde_json::json!({
                "method": "message",
                "params": { "user_id": "u", "user_name": "n", "content": cv }
            });
            let ev: SidecarEvent = serde_json::from_value(msg).unwrap();
            match ev {
                SidecarEvent::Message { params } => {
                    let got = params
                        .content
                        .expect("content must survive the wire roundtrip");
                    assert_eq!(
                        serde_json::to_value(&got).unwrap(),
                        cv,
                        "content variant mutated across roundtrip: {cv:?}"
                    );
                }
                other => panic!("expected Message, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_inbound_structured_fields_parse() {
        let msg = serde_json::json!({
            "method": "message",
            "params": {
                "user_id": "u", "user_name": "n", "text": "hi",
                "is_group": true, "thread_id": "t1", "librefang_user": "lf",
                "username": "@handle",
                "group_members": [
                    {"user_id": "g1", "display_name": "G One", "username": "@g1"}
                ],
                "group_participants": [{"jid": "j@x", "display_name": "J"}],
                "metadata": {"k": "v"}
            }
        });
        let ev: SidecarEvent = serde_json::from_value(msg).unwrap();
        let SidecarEvent::Message { params } = ev else {
            panic!("expected Message");
        };
        assert!(params.is_group);
        assert_eq!(params.thread_id.as_deref(), Some("t1"));
        assert_eq!(params.librefang_user.as_deref(), Some("lf"));
        assert_eq!(params.username.as_deref(), Some("@handle"));
        assert_eq!(params.group_members.len(), 1);
        assert_eq!(params.group_members[0].user_id, "g1");
        assert_eq!(params.group_members[0].username.as_deref(), Some("@g1"));
        assert_eq!(params.group_participants.len(), 1);
        assert_eq!(params.group_participants[0].jid, "j@x");
        assert_eq!(
            params.metadata.get("k"),
            Some(&serde_json::Value::String("v".to_string()))
        );
        assert!(params.content.is_none());
    }

    #[test]
    fn test_legacy_text_message_falls_back_to_text() {
        // A pre-existing text-only adapter sends no `content`; the
        // reader must fall back to ChannelContent::Text(text).
        let json =
            r#"{"method":"message","params":{"user_id":"u","user_name":"n","text":"hello"}}"#;
        let ev: SidecarEvent = serde_json::from_str(json).unwrap();
        let SidecarEvent::Message { params } = ev else {
            panic!("expected Message");
        };
        let params = *params;
        assert!(params.content.is_none());
        assert!(params.group_members.is_empty());
        assert!(!params.is_group);
        let resolved = params
            .content
            .unwrap_or_else(|| ChannelContent::Text(params.text.unwrap_or_default()));
        match resolved {
            ChannelContent::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected Text fallback, got {other:?}"),
        }
    }

    #[test]
    fn test_outbound_send_params_serialization() {
        let user = ChannelUser {
            platform_id: "chan-1".to_string(),
            display_name: "Dee".to_string(),
            librefang_user: None,
        };
        let p = SidecarSendParams {
            channel_id: user.platform_id.clone(),
            text: "flat".to_string(),
            content: Some(ChannelContent::Image {
                url: "https://x/i.png".to_string(),
                caption: None,
                mime_type: None,
            }),
            thread_id: None,
            user: user.clone(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["channel_id"], "chan-1");
        assert_eq!(v["text"], "flat");
        assert_eq!(v["content"]["Image"]["url"], "https://x/i.png");
        assert_eq!(v["user"]["platform_id"], "chan-1");
        // thread_id is skipped when None.
        assert!(v.get("thread_id").is_none());

        let p2 = SidecarSendParams {
            thread_id: Some("th-9".to_string()),
            ..p
        };
        let v2 = serde_json::to_value(&p2).unwrap();
        assert_eq!(v2["thread_id"], "th-9");
    }

    #[test]
    fn test_inbound_message_id_is_preserved() {
        // A platform-native message id must survive onto
        // `SidecarMessageParams` so reactions/edits target the real
        // message rather than a fabricated UUID.
        let json = r#"{"method":"message","params":{
            "user_id":"u","user_name":"n","text":"hi","message_id":"plat-42"
        }}"#;
        let SidecarEvent::Message { params } = serde_json::from_str::<SidecarEvent>(json).unwrap()
        else {
            panic!("expected Message");
        };
        assert_eq!(params.message_id.as_deref(), Some("plat-42"));

        // Absent ⇒ None (reader then generates a UUID).
        let bare = r#"{"method":"message","params":{"user_id":"u","user_name":"n"}}"#;
        let SidecarEvent::Message { params } = serde_json::from_str::<SidecarEvent>(bare).unwrap()
        else {
            panic!("expected Message");
        };
        assert!(params.message_id.is_none());
    }

    #[test]
    fn test_stream_start_thread_id_serialization() {
        // thread_id is carried when present, omitted when absent so
        // thread-unaware adapters see the pre-thread wire shape.
        let with = serde_json::to_value(SidecarCommand::StreamStart {
            params: SidecarStreamStartParams {
                channel_id: "c".to_string(),
                stream_id: "s".to_string(),
                thread_id: Some("t-7".to_string()),
            },
        })
        .unwrap();
        assert_eq!(with["params"]["thread_id"], "t-7");

        let without = serde_json::to_value(SidecarCommand::StreamStart {
            params: SidecarStreamStartParams {
                channel_id: "c".to_string(),
                stream_id: "s".to_string(),
                thread_id: None,
            },
        })
        .unwrap();
        assert!(without["params"].get("thread_id").is_none());
    }

    // ── P2: capability negotiation ────────────────────────────────

    #[test]
    fn test_url_host_extraction() {
        assert_eq!(
            url_host("https://media.example.com/path?q=1").as_deref(),
            Some("media.example.com")
        );
        assert_eq!(
            url_host("https://user:pw@Host.EXAMPLE.com:8443/x").as_deref(),
            Some("host.example.com")
        );
        assert_eq!(url_host("not-a-url").as_deref(), None);
        assert_eq!(url_host("https:///nohost").as_deref(), None);
    }

    fn dummy_adapter() -> SidecarAdapter {
        SidecarAdapter::new(&cfg("dummy", "true", vec![]), std::env::temp_dir())
    }

    /// Build a config with all P3 supervision fields at their serde
    /// defaults (kept in sync with librefang-types, not hardcoded).
    fn cfg(
        name: &str,
        command: &str,
        args: Vec<String>,
    ) -> librefang_types::config::SidecarChannelConfig {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "command": command,
            "args": args,
        }))
        .expect("SidecarChannelConfig from minimal json")
    }

    /// #5294 — `default_agent` is `None` when absent and round-trips
    /// when explicitly set. The field exists so the router-population
    /// loop in `channel_bridge.rs` can seed `AgentRouter.channel_defaults`
    /// for sidecar adapters; missing it caused inbound traffic on sidecar
    /// channels to fall through to the non-deterministic
    /// "first available agent" branch.
    #[test]
    fn sidecar_default_agent_roundtrip_5294() {
        // Absent → None (no-op for deployments that don't need routing pin).
        let minimal = cfg("telegram", "python3", vec![]);
        assert!(minimal.default_agent.is_none());

        // Explicit value round-trips so channel_bridge.rs can seed the router.
        let c: librefang_types::config::SidecarChannelConfig =
            serde_json::from_value(serde_json::json!({
                "name": "telegram",
                "command": "python3",
                "args": ["-m", "librefang.sidecar.adapters.telegram"],
                "default_agent": "fandangorodelo",
            }))
            .unwrap();
        assert_eq!(c.default_agent.as_deref(), Some("fandangorodelo"));
    }

    // #5841 — command-policy + message-coalescing ported to sidecars.
    // `overrides_from_sidecar_config` projects the sidecar-facing fields
    // onto the `ChannelOverrides` the bridge already consults
    // (`is_command_allowed`, debouncer setup).
    fn cfg_json(value: serde_json::Value) -> librefang_types::config::SidecarChannelConfig {
        serde_json::from_value(value).expect("SidecarChannelConfig from json")
    }

    #[test]
    fn overrides_none_for_plain_sidecar_config_5841() {
        // A default sidecar (allow-all, no coalescing) carries no
        // override, so the bridge keeps falling back to the kernel lookup.
        let c = cfg("telegram", "python3", vec![]);
        assert!(super::overrides_from_sidecar_config(&c).is_none());
    }

    #[test]
    fn overrides_disable_maps_to_disable_commands_5841() {
        let c = cfg_json(serde_json::json!({
            "name": "afina-sales-bot",
            "command": "python3",
            "command_policy": "disable",
        }));
        let ov = super::overrides_from_sidecar_config(&c).expect("override built");
        assert!(ov.disable_commands);
        assert!(ov.allowed_commands.is_empty());
        assert!(ov.blocked_commands.is_empty());
        assert_eq!(ov.message_debounce_ms, 0);
    }

    #[test]
    fn overrides_allowlist_maps_to_allowed_commands_5841() {
        let c = cfg_json(serde_json::json!({
            "name": "bot",
            "command": "python3",
            "command_policy": "allowlist",
            "allowed_commands": ["start", "/help"],
        }));
        let ov = super::overrides_from_sidecar_config(&c).expect("override built");
        assert!(!ov.disable_commands);
        assert_eq!(ov.allowed_commands, vec!["start", "/help"]);
        assert!(ov.blocked_commands.is_empty());
    }

    #[test]
    fn overrides_allowlist_empty_fails_closed_5841() {
        // Regression (#5931): `allowlist` with an empty / omitted list is an
        // explicit default-deny intent and must deny ALL commands, not fall
        // through to the allow-everything path. It maps to `disable_commands`,
        // the highest-precedence deny `is_command_allowed` honours.
        let c = cfg_json(serde_json::json!({
            "name": "public-bot",
            "command": "python3",
            "command_policy": "allowlist",
        }));
        let ov = super::overrides_from_sidecar_config(&c).expect("override built");
        assert!(
            ov.disable_commands,
            "empty allowlist must fail closed (deny all)"
        );
        assert!(ov.allowed_commands.is_empty());
        assert!(ov.blocked_commands.is_empty());
    }

    #[test]
    fn overrides_blocklist_maps_to_blocked_commands_5841() {
        let c = cfg_json(serde_json::json!({
            "name": "bot",
            "command": "python3",
            "command_policy": "blocklist",
            "blocked_commands": ["new", "reboot", "agent"],
        }));
        let ov = super::overrides_from_sidecar_config(&c).expect("override built");
        assert!(!ov.disable_commands);
        assert!(ov.allowed_commands.is_empty());
        assert_eq!(ov.blocked_commands, vec!["new", "reboot", "agent"]);
    }

    #[test]
    fn overrides_coalesce_window_maps_to_debounce_ms_5841() {
        // Coalescing alone (policy left at the allow default) still yields
        // an override so the bridge's debouncer turns on.
        let c = cfg_json(serde_json::json!({
            "name": "bot",
            "command": "python3",
            "message_coalesce_window_ms": 3000,
        }));
        let ov = super::overrides_from_sidecar_config(&c).expect("override built");
        // Allow policy leaves every command-gating field untouched.
        assert!(!ov.disable_commands);
        assert!(ov.allowed_commands.is_empty());
        assert!(ov.blocked_commands.is_empty());
        assert_eq!(ov.message_debounce_ms, 3000);
    }

    #[test]
    fn adapter_exposes_built_overrides_5841() {
        // The trait method the bridge calls returns the built overrides.
        let c = cfg_json(serde_json::json!({
            "name": "bot",
            "command": "python3",
            "command_policy": "disable",
            "message_coalesce_window_ms": 1500,
        }));
        let adapter = SidecarAdapter::new(&c, std::path::PathBuf::from("/tmp"));
        let ov = adapter
            .channel_overrides()
            .expect("adapter carries overrides");
        assert!(ov.disable_commands);
        assert_eq!(ov.message_debounce_ms, 1500);
    }

    #[test]
    fn test_supcfg_defaults_and_overflow_parsing() {
        // Minimal config -> every supervision field at its serde default.
        let c = cfg("x", "true", vec![]);
        let s = SupCfg::from_config(&c);
        assert!(s.restart);
        assert_eq!(s.initial_backoff_ms, 500);
        assert_eq!(s.max_backoff_ms, 30_000);
        assert_eq!(s.max_retries, 10);
        assert_eq!(s.reset_after_secs, 60);
        assert_eq!(s.ready_timeout_secs, 30);
        assert_eq!(s.shutdown_grace_secs, 5);
        assert_eq!(s.message_buffer, 256);
        assert_eq!(s.overflow, SidecarOverflowPolicy::Block);

        // Explicit overrides round-trip, incl. snake_case enum.
        let c2: librefang_types::config::SidecarChannelConfig =
            serde_json::from_value(serde_json::json!({
                "name": "x",
                "command": "true",
                "restart": false,
                "restart_max_retries": 3,
                "message_buffer": 8,
                "overflow": "drop_newest",
            }))
            .unwrap();
        let s2 = SupCfg::from_config(&c2);
        assert!(!s2.restart);
        assert_eq!(s2.max_retries, 3);
        assert_eq!(s2.message_buffer, 8);
        assert_eq!(s2.overflow, SidecarOverflowPolicy::DropNewest);

        // message_buffer is clamped to >= 1 (mpsc::channel(0) panics).
        let c3: librefang_types::config::SidecarChannelConfig =
            serde_json::from_value(serde_json::json!({
                "name": "x", "command": "true", "message_buffer": 0
            }))
            .unwrap();
        assert_eq!(SupCfg::from_config(&c3).message_buffer, 1);
    }

    #[tokio::test]
    async fn test_cap_gated_methods_default_without_ready() {
        // No `ready` was received → no caps. Every optional method must
        // degrade exactly like the pre-P2 trait defaults (no stdin
        // touched, so these resolve without a live subprocess).
        let a = dummy_adapter();
        let user = ChannelUser {
            platform_id: "c".to_string(),
            display_name: "U".to_string(),
            librefang_user: None,
        };
        assert!(!a.has_cap("typing"));
        assert!(a.send_typing(&user).await.is_ok());
        assert!(a
            .send_reaction(
                &user,
                "m1",
                &LifecycleReaction {
                    phase: crate::types::AgentPhase::Thinking,
                    emoji: "👍".to_string(),
                    remove_previous: false,
                },
            )
            .await
            .is_ok());
        assert!(!a.supports_streaming());
        assert!(a.account_id().is_none());
        assert!(a.notification_recipients().is_empty());
        assert!(!a.suppress_error_responses());
        assert!(a.fetch_headers_for("https://x/y").is_empty());
        // `typing_events()` is deliberately NOT cap-gated (the bridge
        // queries it before the async `ready` lands): the receiver is
        // handed out unconditionally, then `None` only because it was
        // already taken.
        assert!(a.typing_events().is_some());
        assert!(a.typing_events().is_none(), "receiver handed out once");
    }

    #[tokio::test]
    async fn test_sidecar_adapter_spawn_echo() {
        // Integration test: spawn the Python echo adapter if python3 is available
        let python = which_python();
        if python.is_none() {
            // Skip test if python3 is not available
            return;
        }
        let python = python.unwrap();

        // Find the example adapter
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let adapter_path = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/sidecar-channel-python/adapter.py");

        if !adapter_path.exists() {
            // Skip if the example doesn't exist yet
            return;
        }

        let config = cfg(
            "test-echo",
            &python,
            vec!["-u".to_string(), adapter_path.to_string_lossy().to_string()],
        );

        let adapter = SidecarAdapter::new(&config, std::env::temp_dir());
        let mut stream = adapter.start().await.unwrap();

        use futures::StreamExt;

        // Wait for the process to start and emit the "ready" event.
        // The ready event is consumed by the reader task (not forwarded as a ChannelMessage),
        // so we just need a short delay for the process to boot.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Send a message to trigger an echo
        adapter
            .send(
                &ChannelUser {
                    platform_id: "test-ch".to_string(),
                    display_name: "Tester".to_string(),
                    librefang_user: None,
                },
                ChannelContent::Text("Hello sidecar!".to_string()),
            )
            .await
            .expect("Failed to send message to sidecar — process may have exited early");

        // Read the echo reply. Windows-2025 GitHub runners under load have been
        // observed to spend > 10s in Python cold-start (panicked at 11.346s in
        // CI for c176b2a — see #4676). 30s gives ample headroom while still
        // catching real hangs via nextest's overall test timeout.
        let msg = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next())
            .await
            .expect("Timed out waiting for echo reply")
            .expect("Stream ended unexpectedly");

        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Echo:"), "Expected echo response, got: {t}");
                assert!(
                    t.contains("Hello sidecar!"),
                    "Expected echoed text, got: {t}"
                );
            }
            other => panic!("Expected Text content, got: {other:?}"),
        }

        // Stop the adapter
        adapter.stop().await.unwrap();
        let status = adapter.status();
        assert!(!status.connected);
    }

    #[tokio::test]
    async fn test_supervisor_restarts_crashing_child() {
        // A child that announces ready, emits one message, then exits.
        // The supervisor must restart it so the SAME returned stream
        // keeps yielding messages across child deaths.
        let python = match which_python() {
            Some(p) => p,
            None => return,
        };
        let script = concat!(
            "import sys,json;",
            "print(json.dumps({'method':'ready'}),flush=True);",
            "print(json.dumps({'method':'message','params':",
            "{'user_id':'u','user_name':'n','text':'tick'}}),flush=True);",
            "sys.exit(0)"
        );
        let config = cfg(
            "test-restart",
            &python,
            vec!["-u".to_string(), "-c".to_string(), script.to_string()],
        );
        let adapter = SidecarAdapter::new(&config, std::env::temp_dir());
        let mut stream = adapter.start().await.unwrap();
        use futures::StreamExt;

        // Two messages can only arrive if the child was restarted at
        // least once (each child emits exactly one then exits).
        for _ in 0..2 {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next())
                .await
                .expect("timed out waiting for a message across restart")
                .expect("stream ended unexpectedly");
            match &msg.content {
                ChannelContent::Text(t) => assert_eq!(t, "tick"),
                other => panic!("expected Text, got {other:?}"),
            }
        }
        adapter.stop().await.unwrap();
        assert!(!adapter.status().connected);
    }

    /// Regression: `trip_circuit` must preserve the specific per-attempt
    /// cause in `status.last_error` after the breaker trips. Operators
    /// reading `GET /api/channels` / dashboard rely on `last_error` to
    /// see *why* a sidecar died (e.g. "TELEGRAM_BOT_TOKEN is required",
    /// missing binary, bad config); the prior implementation overwrote
    /// it with a generic "circuit-breaker tripped after N attempts"
    /// notice, hiding the actionable signal.
    ///
    /// Drives the spawn-failure branch via a non-existent command so
    /// the supervisor never even reaches `ready`; the OS spawn error
    /// is recorded as `last_error` on each attempt, and after
    /// `restart_max_retries` the trip must compose both pieces.
    #[tokio::test]
    async fn circuit_break_preserves_last_specific_cause() {
        let config: librefang_types::config::SidecarChannelConfig =
            serde_json::from_value(serde_json::json!({
                "name": "circuit-cause",
                // An absolute path that cannot exist — `Command::spawn`
                // fails synchronously with a stable OS error message
                // wrapped by `spawn_once` as "Failed to spawn sidecar
                // '<name>' (<cmd>): <os err>".
                "command": "/nonexistent/librefang-sidecar-circuit-test",
                "restart_max_retries": 1,
                "restart_initial_backoff_ms": 1,
                "restart_max_backoff_ms": 2,
            }))
            .expect("valid SidecarChannelConfig");
        let adapter = SidecarAdapter::new(&config, std::env::temp_dir());
        let _stream = adapter.start().await.unwrap();

        // Two failed spawns (attempt 0, attempt 1) then trip
        // (1 >= max_retries). Backoff is 1-2ms so the test is bounded
        // by spawn latency; 5s leaves ample headroom on slow CI.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let last_error = loop {
            if let Some(e) = adapter.status().last_error {
                if e.contains("circuit-breaker tripped") {
                    break e;
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "supervisor did not trip in time; last_error={:?}",
                    adapter.status().last_error
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        adapter.stop().await.unwrap();

        assert!(
            last_error.contains("last cause:"),
            "circuit-break message must carry the prior cause, got: {last_error}"
        );
        // The OS phrasing for "no such file" varies across platforms;
        // the supervisor's own wrapper is stable, so assert on that.
        assert!(
            last_error.contains("Failed to spawn sidecar"),
            "circuit-break message must surface the spawn-failed wrapper, got: {last_error}"
        );
    }

    /// End-to-end: drive a real child that emits `qr_ready` followed
    /// by `qr_status: confirmed`, then assert the supervisor's stdout
    /// reader populated `ChannelStatus.qr` accordingly. Catches a
    /// future refactor that breaks the reader's event-dispatch path
    /// (e.g. forgetting to wire a new variant through the match arm).
    #[tokio::test]
    async fn qr_events_land_on_channel_status() {
        let python = match which_python() {
            Some(p) => p,
            None => return,
        };
        let script = concat!(
            "import sys,json,time;",
            "print(json.dumps({'method':'ready'}),flush=True);",
            "print(json.dumps({'method':'qr_ready','params':{",
            "'qr_code':'opaque-token',",
            "'qr_url':'https://example.invalid/login?code=opaque-token',",
            "'message':'scan'}}),flush=True);",
            "print(json.dumps({'method':'qr_status','params':{",
            "'status':'confirmed','message':'logged in'}}),flush=True);",
            // Stay alive long enough for the supervisor to handle both
            // events before stdout closes; we `stop()` from the test.
            "time.sleep(2)",
        );
        let config = cfg(
            "test-qr",
            &python,
            vec!["-u".to_string(), "-c".to_string(), script.to_string()],
        );
        let adapter = SidecarAdapter::new(&config, std::env::temp_dir());
        let _stream = adapter.start().await.unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let qr = loop {
            if let Some(q) = adapter.status().qr.clone() {
                if matches!(q.status, crate::types::QrStatusKind::Confirmed) {
                    break q;
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "supervisor never reached confirmed QR state; last status={:?}",
                    adapter.status().qr
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        };
        adapter.stop().await.unwrap();

        assert_eq!(qr.qr_code, "opaque-token");
        assert_eq!(
            qr.qr_url.as_deref(),
            Some("https://example.invalid/login?code=opaque-token"),
        );
        // Last-write-wins for `message`: the qr_status arm overrides
        // the qr_ready preamble when it carries one, so the
        // confirmed-state message survives.
        assert_eq!(qr.message.as_deref(), Some("logged in"));
    }

    #[tokio::test]
    async fn test_sidecar_username_folds_to_sender_username_metadata() {
        // Regression: the reader must fold `username` into
        // `metadata["sender_username"]` — the key the bridge reads for
        // SenderContext / roster — not the dead `"username"` key. The
        // child also sends `ready` with `params: null` to exercise the
        // null-params parse end-to-end.
        let python = match which_python() {
            Some(p) => p,
            None => return,
        };
        let script = concat!(
            "import sys,json;",
            "print(json.dumps({'method':'ready','params':None}),flush=True);",
            "print(json.dumps({'method':'message','params':",
            "{'user_id':'u','user_name':'n','text':'hi',",
            "'username':'@handle'}}),flush=True);",
            "sys.exit(0)"
        );
        let config = cfg(
            "test-username",
            &python,
            vec!["-u".to_string(), "-c".to_string(), script.to_string()],
        );
        let adapter = SidecarAdapter::new(&config, std::env::temp_dir());
        let mut stream = adapter.start().await.unwrap();
        use futures::StreamExt;
        let msg = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next())
            .await
            .expect("timed out waiting for message")
            .expect("stream ended unexpectedly");
        assert_eq!(
            msg.metadata.get("sender_username").and_then(|v| v.as_str()),
            Some("@handle"),
            "username must land under the bridge-consumed key"
        );
        assert!(
            !msg.metadata.contains_key("username"),
            "the dead `username` key must not be written"
        );
        adapter.stop().await.unwrap();
    }

    // ── build_spawn_env precedence tests ───────────────────────────
    //
    // Sequential because they touch the *process* environment via
    // `std::env::set_var` / `remove_var`, which is global. Each test
    // uses a unique key prefix (`LIBREFANG_TEST_<TESTNAME>_*`) to avoid
    // accidentally aliasing keys an unrelated parallel test might also
    // touch — but the *parent-env-wins* assertion still needs the test
    // to set a key in `std::env`, observe `build_spawn_env` honouring
    // it, and clean up. We intentionally do not gate this on a mutex:
    // unique key prefixes are enough for correctness, the env var is
    // private to LibreFang test scope, and the assertions are about
    // presence/absence of *that key alone*.

    #[test]
    fn build_spawn_env_secrets_env_visible_to_child() {
        // secrets.env is the only source of a key — it must appear.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("secrets.env"),
            "LIBREFANG_TEST_BSE_SECRETS_ONLY=from_file\n",
        )
        .unwrap();
        // Make sure the parent env does not already define it.
        // SAFETY: test-local key prefix; nothing else races on it.
        unsafe {
            std::env::remove_var("LIBREFANG_TEST_BSE_SECRETS_ONLY");
        }

        let ctx_env: HashMap<String, String> = HashMap::new();
        let merged = build_spawn_env(tmp.path(), &ctx_env);
        let got: HashMap<_, _> = merged.into_iter().collect();
        assert_eq!(
            got.get("LIBREFANG_TEST_BSE_SECRETS_ONLY")
                .map(|s| s.as_str()),
            Some("from_file"),
        );
    }

    #[test]
    fn build_spawn_env_parent_env_beats_secrets() {
        // dotenv precedence: shell-exported value beats secrets.env.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("secrets.env"),
            "LIBREFANG_TEST_BSE_PARENT_WINS=from_file\n",
        )
        .unwrap();
        // SAFETY: test-local key prefix; we clean up at the end.
        unsafe {
            std::env::set_var("LIBREFANG_TEST_BSE_PARENT_WINS", "from_parent");
        }

        let ctx_env: HashMap<String, String> = HashMap::new();
        let merged = build_spawn_env(tmp.path(), &ctx_env);
        let got: HashMap<_, _> = merged.into_iter().collect();
        // The merge skipped the secrets.env entry because the parent
        // env already had the key; the child still inherits the parent
        // env (we don't call env_clear), so the *effective* value is
        // "from_parent" without us needing to re-emit it.
        assert!(
            !got.contains_key("LIBREFANG_TEST_BSE_PARENT_WINS"),
            "build_spawn_env must NOT shadow a parent-env key with a secrets.env value"
        );

        // SAFETY: cleanup of the key we just set.
        unsafe {
            std::env::remove_var("LIBREFANG_TEST_BSE_PARENT_WINS");
        }
    }

    #[test]
    fn build_spawn_env_ctx_env_beats_secrets() {
        // config.toml [sidecar_channels.env] explicit overrides win
        // over secrets.env (operator-explicit > file-loaded fallback).
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("secrets.env"),
            "LIBREFANG_TEST_BSE_CTX_WINS=from_file\n",
        )
        .unwrap();
        // SAFETY: test-local key prefix.
        unsafe {
            std::env::remove_var("LIBREFANG_TEST_BSE_CTX_WINS");
        }

        let mut ctx_env: HashMap<String, String> = HashMap::new();
        ctx_env.insert(
            "LIBREFANG_TEST_BSE_CTX_WINS".to_string(),
            "from_config".to_string(),
        );
        let merged = build_spawn_env(tmp.path(), &ctx_env);
        let got: HashMap<_, _> = merged.into_iter().collect();
        assert_eq!(
            got.get("LIBREFANG_TEST_BSE_CTX_WINS").map(|s| s.as_str()),
            Some("from_config"),
            "ctx_env must override secrets.env"
        );
    }

    #[test]
    fn build_spawn_env_missing_file_is_not_an_error() {
        // secrets.env does not exist → empty contribution, ctx_env passes through.
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx_env: HashMap<String, String> = HashMap::new();
        ctx_env.insert("FOO".to_string(), "bar".to_string());
        let merged = build_spawn_env(tmp.path(), &ctx_env);
        let got: HashMap<_, _> = merged.into_iter().collect();
        assert_eq!(got.get("FOO").map(|s| s.as_str()), Some("bar"));
    }

    /// Find python3 or python on PATH.
    fn which_python() -> Option<String> {
        for name in &["python3", "python"] {
            if std::process::Command::new(name)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
            {
                return Some(name.to_string());
            }
        }
        None
    }
}
