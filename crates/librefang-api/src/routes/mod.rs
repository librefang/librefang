//! Route handlers for the LibreFang API.
//!
//! 每个领域子模块导出一个 `router()` 函数来构建自己的路由树。
//! `server.rs` 通过 `.merge()` 组合所有子路由器，避免在单一函数中维护数百行路由注册。
//!
//! 处理函数仍通过 glob re-export 暴露，以保持 `routes::handler_name` 的向后兼容性
//! （特别是 openapi.rs 的 utoipa 宏需要此路径格式）。

// 各模块都导出 `router()` 函数，glob re-export 会产生同名歧义，
// 但 `router()` 只通过限定路径访问（如 `routes::agents::router()`），不会实际冲突。
#![allow(ambiguous_glob_reexports)]

pub mod agents;
pub mod budget;
pub mod channels;
pub mod config;
pub mod goals;
pub mod memory;
pub mod network;
pub mod plugins;
pub mod providers;
pub mod skills;
pub mod system;
pub mod workflows;

// 通过 glob re-export 保持 `routes::handler_name` 向后兼容
// （openapi.rs 的 utoipa 宏、ws.rs 等都依赖此路径格式）。
//
// 原先 system.rs 和 workflows.rs 都导出了 `list_templates` / `get_template`，
// 导致 E0659 名称歧义。已将 workflows.rs 中的版本重命名为
// `list_workflow_templates` / `get_workflow_template` 以消除冲突。
//
// 各模块都导出了 `router()` 函数，glob re-export 会产生歧义警告，
// 但 `router()` 只通过限定路径（如 `routes::agents::router()`）访问，不会实际冲突。
pub use agents::*;
pub use budget::*;
pub use channels::*;
pub use config::*;
pub use goals::*;
pub use memory::*;
pub use network::*;
pub use plugins::*;
pub use providers::*;
pub use skills::*;
pub use system::*;
pub use workflows::*;

use crate::middleware::RequestLanguage;
use dashmap::DashMap;
use librefang_kernel::LibreFangKernel;
use librefang_types::i18n::{self, ErrorTranslator};
use std::sync::Arc;
use std::time::Instant;

/// Extract an [`ErrorTranslator`] from the request extensions.
///
/// Uses the language resolved by the `accept_language` middleware, or falls
/// back to English if the middleware hasn't run (e.g. in tests).
#[allow(dead_code)]
pub(crate) fn translator_from_extensions(extensions: &axum::http::Extensions) -> ErrorTranslator {
    let lang = extensions
        .get::<RequestLanguage>()
        .map(|rl| rl.0)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);
    ErrorTranslator::new(lang)
}

/// Resolve the client language from an optional `RequestLanguage` extension.
#[allow(dead_code)]
pub(crate) fn resolve_lang(lang: Option<&axum::Extension<RequestLanguage>>) -> &'static str {
    lang.map(|l| l.0 .0).unwrap_or(i18n::DEFAULT_LANGUAGE)
}

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<LibreFangKernel>,
    pub started_at: Instant,
    /// Optional peer registry for OFP mesh networking status.
    pub peer_registry: Option<Arc<librefang_wire::registry::PeerRegistry>>,
    /// Channel bridge manager — held behind a Mutex so it can be swapped on hot-reload.
    pub bridge_manager: tokio::sync::Mutex<Option<librefang_channels::bridge::BridgeManager>>,
    /// Live channel config — updated on every hot-reload so list_channels() reflects reality.
    pub channels_config: tokio::sync::RwLock<librefang_types::config::ChannelsConfig>,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// ClawHub response cache — prevents 429 rate limiting on rapid dashboard refreshes.
    /// Maps cache key → (fetched_at, response_json) with 120s TTL.
    pub clawhub_cache: DashMap<String, (Instant, serde_json::Value)>,
    /// Skillhub response cache — prevents rate limiting on rapid dashboard refreshes.
    /// Maps cache key → (fetched_at, response_json) with TTL.
    pub skillhub_cache: DashMap<String, (Instant, serde_json::Value)>,
    /// Probe cache for local provider health checks (ollama/vllm/lmstudio).
    /// Avoids blocking the `/api/providers` endpoint on TCP timeouts to
    /// unreachable local services. 60-second TTL.
    pub provider_probe_cache: librefang_runtime::provider_health::ProbeCache,
    /// Webhook subscription store for outbound event notifications.
    pub webhook_store: crate::webhook_store::WebhookStore,
}
