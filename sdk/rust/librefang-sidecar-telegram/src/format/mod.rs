//! Text formatting pipeline for outbound Telegram messages.
//!
//! Mirror of the Python adapter's three-stage pipeline:
//! 1. `markdown` — Markdown → Telegram HTML
//! 2. `sanitize` — drop disallowed HTML tags, balance unclosed, enforce href allowlist
//! 3. `chunk` — split into <= 4096 UTF-16 code-unit chunks for sendMessage / editMessageText

pub mod chunk;
pub mod markdown;
pub mod sanitize;

pub use chunk::{split_to_utf16_chunks, truncate_to_utf16_limit, TELEGRAM_MSG_LIMIT};
pub use markdown::markdown_to_telegram_html;
pub use sanitize::sanitize_telegram_html;

/// Two-stage Markdown → sanitized Telegram HTML.
///
/// This intentionally does not enforce Telegram's message-size limit. Use
/// [`format_sanitize_and_chunk`] for ordinary `sendMessage` text; captions and
/// streaming edits have separate size/lifecycle rules and use this helper.
pub fn format_and_sanitize(text: &str) -> String {
    sanitize_telegram_html(&markdown_to_telegram_html(text))
}

/// Complete outbound text pipeline: Markdown → sanitized Telegram HTML →
/// chunks bounded to [`TELEGRAM_MSG_LIMIT`] UTF-16 code units.
pub fn format_sanitize_and_chunk(text: &str) -> Vec<String> {
    split_to_utf16_chunks(&format_and_sanitize(text), TELEGRAM_MSG_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_pipeline_formats_sanitizes_and_chunks() {
        let input = "x".repeat(TELEGRAM_MSG_LIMIT + 1);
        let chunks = format_sanitize_and_chunk(&input);
        assert_eq!(chunks.len(), 2);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.encode_utf16().count() <= TELEGRAM_MSG_LIMIT));
        assert_eq!(chunks.concat(), format_and_sanitize(&input));

        assert_eq!(format_sanitize_and_chunk("**bold**"), ["<b>bold</b>\n"]);
    }
}
