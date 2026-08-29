//! Text formatting pipeline for outbound Telegram messages.
//!
//! The preferred path is `rich_sanitize` → `sendRichMessage`: Telegram's own
//! GFM-compatible parser handles the text, so tables, `_italic_` and nested emphasis
//! work, and the limit is 32768 characters rather than 4096.
//!
//! The stages below are the fallback for Bot API servers older than 10.1, and mirror
//! the Python adapter's three-stage pipeline:
//! 1. `markdown` — Markdown → Telegram HTML
//! 2. `sanitize` — drop disallowed HTML tags, balance unclosed, enforce href allowlist
//! 3. `chunk` — split into <= 4096 UTF-16 code-unit chunks for sendMessage / editMessageText

pub mod chunk;
pub mod markdown;
pub mod rich_sanitize;
pub mod sanitize;

pub use chunk::{split_to_utf16_chunks, truncate_to_utf16_limit, TELEGRAM_MSG_LIMIT};
pub use markdown::markdown_to_telegram_html;
pub use rich_sanitize::sanitize_rich_markdown;
pub use sanitize::sanitize_telegram_html;

/// Rich message text limit: "Up to 32768 UTF-8 characters in the rich message text"
/// (Bot API, Rich Message Limits). Counted in characters, not UTF-16 code units — the
/// legacy `sendMessage` path keeps its own [`TELEGRAM_MSG_LIMIT`] in code units.
pub const RICH_MSG_LIMIT: usize = 32_768;

/// Prepare agent text for `sendRichMessage`, returning `None` when it does not fit the
/// rich limit and the caller should fall back to the chunking HTML pipeline.
pub fn prepare_rich_markdown(text: &str) -> Option<String> {
    let sanitized = sanitize_rich_markdown(text);
    (sanitized.chars().count() <= RICH_MSG_LIMIT).then_some(sanitized)
}

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
