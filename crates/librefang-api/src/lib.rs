//! HTTP/WebSocket API server for the LibreFang Agent OS daemon.
//!
//! Exposes agent management, status, and chat via JSON REST endpoints.
//! The kernel runs in-process; the CLI connects over HTTP.

pub mod channel_bridge;
pub mod middleware;
pub mod oauth;
pub mod openai_compat;
pub mod openapi;
pub mod password_hash;
pub mod rate_limiter;
pub mod routes;
pub mod server;
pub mod stream_chunker;
pub mod stream_dedup;
pub mod terminal;
pub mod types;
pub mod validation;
pub mod versioning;
pub mod webchat;
pub mod webhook_store;
pub mod ws;

#[cfg(feature = "telemetry")]
pub mod telemetry;
