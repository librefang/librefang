//! Text formatting pipeline for outbound Telegram messages.
//!
//! The preferred path is `rich_blocks` → `sendRichMessage`: the agent's Markdown is
//! parsed here into `InputRichBlock` values, so Telegram runs no parser over our text at
//! all and quoted content cannot become markup. The limit is 32768 characters rather
//! than 4096.
//!
//! `rich_sanitize` → `rich_message.markdown` remains only as a guard for the case where
//! conversion yields nothing for non-empty text, which would mean a converter bug.
//!
//! The stages below are the fallback for Bot API servers older than 10.1, and mirror
//! the Python adapter's three-stage pipeline:
//! 1. `markdown` — Markdown → Telegram HTML
//! 2. `sanitize` — drop disallowed HTML tags, balance unclosed, enforce href allowlist
//! 3. `chunk` — split into <= 4096 UTF-16 code-unit chunks for sendMessage / editMessageText

pub mod chunk;
pub mod markdown;
pub mod rich_blocks;
pub mod rich_sanitize;
pub mod sanitize;

pub use chunk::{split_to_utf16_chunks, truncate_to_utf16_limit, TELEGRAM_MSG_LIMIT};
pub use markdown::markdown_to_telegram_html;
pub use rich_blocks::{markdown_to_blocks, Block};
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

/// Prepare agent text for `sendRichMessage` as structured blocks, which is the preferred
/// rich path: Telegram runs no parser over a block's text, so quoted content cannot become
/// markup and code samples keep their angle brackets.
///
/// Returns `None` when the caller should not use blocks — either the text does not fit the
/// rich limit, or conversion produced nothing for text that was not empty. The second case
/// should not happen; it is a guard, because losing the whole message to a converter bug is
/// far worse than falling back to a lower-fidelity path.
pub fn prepare_rich_blocks(text: &str) -> Option<Vec<Block>> {
    let blocks = markdown_to_blocks(text);
    if blocks.is_empty() && !text.trim().is_empty() {
        return None;
    }
    // Measured on the blocks, not on the source Markdown: the syntax characters the
    // converter consumes never reach Telegram, and for a table they are a large share of
    // the source — exactly the input this path exists for.
    (rich_blocks::text_len(&blocks) <= RICH_MSG_LIMIT).then_some(blocks)
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

#[cfg(test)]
mod rich_path_tests {
    use super::*;

    /// The guard belongs to `prepare_rich_blocks`, not to `text_len`: a unit test on the
    /// helper passes whichever value the caller measures. This one drives the decision
    /// function with a table whose *source* is over the limit while its delivered text is
    /// far under it — measuring the source turns it away, and a Markdown-heavy table is
    /// exactly the input the rich path exists for.
    #[test]
    fn a_table_is_measured_on_its_cells_not_its_syntax() {
        let row = "| aaaa | bbbb |\n";
        let mut table = String::from("| a | b |\n|:--|--:|\n");
        while table.chars().count() + row.chars().count() <= RICH_MSG_LIMIT {
            table.push_str(row);
        }
        table.push_str(row); // now the source is over the limit
        assert!(table.chars().count() > RICH_MSG_LIMIT);

        let blocks = prepare_rich_blocks(&table).expect("a table this size still fits");
        assert!(rich_blocks::text_len(&blocks) <= RICH_MSG_LIMIT);
    }

    /// The limit still has to bite when the *text* is genuinely too long.
    #[test]
    fn text_over_the_limit_is_still_turned_away() {
        let long = "x".repeat(RICH_MSG_LIMIT + 1);
        assert!(prepare_rich_blocks(&long).is_none());
    }

    /// Losing the whole message to a converter bug is the worst outcome available, so an
    /// empty conversion of non-empty input must fall through rather than send nothing.
    #[test]
    fn an_empty_conversion_of_non_empty_text_is_refused() {
        assert!(prepare_rich_blocks("hello").is_some());
        assert_eq!(prepare_rich_blocks("   "), Some(Vec::new()));
    }
}
