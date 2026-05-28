//! Text formatting pipeline for outbound Telegram messages.
//!
//! Mirror of the Python adapter's three-stage pipeline:
//! 1. `markdown` — Markdown → Telegram HTML
//! 2. `sanitize` — drop disallowed HTML tags, balance unclosed, enforce href allowlist
//! 3. `chunk` — split into <= 4096 UTF-16 code-unit chunks for sendMessage / editMessageText

pub mod chunk;
pub mod markdown;
pub mod sanitize;

pub use chunk::{split_to_utf16_chunks, TELEGRAM_MSG_LIMIT};
pub use markdown::markdown_to_telegram_html;
pub use sanitize::sanitize_telegram_html;

/// One-stop Markdown → sanitised Telegram HTML.
pub fn format_and_sanitize(text: &str) -> String {
    sanitize_telegram_html(&markdown_to_telegram_html(text))
}
