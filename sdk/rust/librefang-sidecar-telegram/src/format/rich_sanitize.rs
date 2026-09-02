//! Neutralise "active" constructs in agent-authored Rich Markdown before it is
//! handed to `sendRichMessage`.
//!
//! Rich Markdown "can contain arbitrary HTML" (Bot API 10.1+). That is a problem for
//! us: the text we send is model output, and model output routinely *quotes* untrusted
//! content — a fetched web page, an email body, a file the agent read. Without this
//! pass, quoted content could render itself inline buttons:
//!
//! ```text
//! <tg-button type="callback_data" data="anything">Click me</tg-button>
//! ```
//!
//! A tap then arrives back at the adapter as a `ButtonCallback` event with an
//! attacker-chosen payload. Interactive buttons must stay an explicit
//! `ChannelContent::Interactive` feature, never a side effect of formatting text.
//!
//! # What this pass does, and what it deliberately does not
//!
//! Two character-local rules. No lookahead, no scanning, no attempt to locate a Markdown
//! construct or to find where one ends:
//!
//! * `<` is escaped so that the run of backslashes preceding it is **odd**, which is
//!   what makes Markdown treat it as literal. A `<` the author already escaped is left
//!   exactly as it is; only an unescaped one gains a backslash. The parity matters: the
//!   Bot API warns that "'\' character usually must be escaped with a preceding '\'
//!   character", and an even run leaves the `<` bare.
//! * `!` before `[` is backslash-escaped, so `![](url)` stays inert text rather than
//!   becoming a media block fetched from that URL.
//!
//! Author-written backslashes are left alone. An earlier revision doubled every one of
//! them, which reached the same parity but silently rewrote the text: `\*not italic\*`
//! became `\\*not italic\\*`, so the emphasis the author had escaped came back with
//! stray backslashes, and `[a\](https://x)` — not a link at all — turned into one.
//!
//! Backslash rather than `&lt;`, because the Bot API documents that mechanism for the
//! `markdown` field by example — `\#hashtag` appears in its own Rich Markdown sample —
//! and says nothing about entity decoding there; the entity note appears only under the
//! two HTML styles.
//!
//! **Link destinations are not filtered.** Earlier revisions checked them against
//! `sanitize`'s scheme allowlist, which meant locating Markdown links: a label scanner
//! with a length cap, a per-message budget, a forward cursor, reference-definition and
//! title parsing. Five rounds of adversarial review found a defect in that machinery
//! every single time, four of them introduced by the fix for the previous one, and the
//! last of them turned an input containing no link at all into a live `javascript:` one.
//! Nine of this module's ten functions existed to serve that check; every defect lived
//! in them, and none ever touched the rule above.
//!
//! The distinction that matters: `sanitize::sanitize_telegram_html` filters schemes
//! correctly because it *constructs* the `<a href>` itself and therefore knows where the
//! href is. Here we would be guessing at someone else's parse of someone else's text.
//! The established libraries for this problem — `telegramify-markdown` on
//! pulldown-cmark, GramIO's entity builders — all parse with a real parser rather than
//! scan. We have no parser on this path, so we do not pretend to have one.
//!
//! What guards a hostile link instead: Telegram renders only schemes it supports ("other
//! *supported* links are rendered as regular inline links"), the client shows an "Open
//! this link?" confirmation carrying the full URL, and a chat client has no script
//! context. The legacy `sendMessage` path we fall back to on a 4xx still filters, since
//! it builds the HTML itself.
//!
//! # Cost
//!
//! The escapes are applied everywhere, including inside code spans and fenced blocks
//! where Markdown does not process them, so `Vec<String>` in a fence reads
//! `Vec\<String>` and a literal backslash is doubled. Every Rich HTML construct is lost,
//! since they all start with `<`: `<u>`, `<ins>`, `<sub>`, `<sup>`, `<br>`,
//! `<details>` / `<summary>`, `<aside>` / `<cite>`, `<a name>` anchors, `<tg-map>`,
//! `<tg-collage>`, `<tg-slideshow>`, `<tg-emoji>`, `<tg-time>`. Emphasis, strikethrough,
//! spoilers (`||…||`), highlight (`==…==`), tables, lists, headings, code, math and
//! footnotes are plain Markdown and unaffected.
//!
//! `InputRichMessage.blocks` removes both the cost and the need for this pass entirely —
//! Telegram runs no parser on a block's text — and is tracked separately.

/// Escape agent text so Telegram's Rich Markdown parser cannot be steered into
/// producing interactive or media-fetching elements.
pub fn sanitize_rich_markdown(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    // Length of the run of backslashes immediately before the current position. An odd
    // run already escapes whatever follows it, so a `<` there needs nothing added.
    let mut backslashes = 0_usize;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                out.push('\\');
                backslashes += 1;
                i += 1;
            }
            // The whole security property, in one arm: every `<` ends up behind an odd
            // run of backslashes, so no bare `<` survives and no raw HTML can reach
            // Telegram regardless of what the surrounding text looks like.
            b'<' => {
                if backslashes % 2 == 0 {
                    out.push('\\');
                }
                out.push('<');
                backslashes = 0;
                i += 1;
            }
            // `![...](...)` is a real media block in Rich Markdown, fetched from the
            // URL — today it is inert text. Escaping the `!` keeps it that way.
            b'!' if bytes.get(i + 1) == Some(&b'[') && backslashes % 2 == 0 => {
                out.push_str("\\!");
                backslashes = 0;
                i += 1;
            }
            b => {
                let len = utf8_char_len(b);
                out.push_str(&input[i..i + len]);
                backslashes = 0;
                i += len;
            }
        }
    }

    out
}

/// Length in bytes of the UTF-8 character starting with `first`.
fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const BUTTON: &str = r#"<tg-button type="callback_data" data="wipe">Tap</tg-button>"#;

    /// The guarantee, as a predicate: every `<` in the output is preceded by an **odd**
    /// run of backslashes, so Markdown reads it as escaped. Asserting merely "there is a
    /// backslash before it" is a weaker property — `\\<` satisfies that and is a bare
    /// `<` to the parser, which is how a leak once passed two tests named for the
    /// guarantee.
    fn every_less_than_is_escaped(out: &str) -> bool {
        out.match_indices('<').all(|(idx, _)| {
            out.as_bytes()[..idx]
                .iter()
                .rev()
                .take_while(|&&c| c == b'\\')
                .count()
                % 2
                == 1
        })
    }

    #[test]
    fn the_escape_predicate_rejects_an_even_run_of_backslashes() {
        assert!(every_less_than_is_escaped("a \\<tg-button"));
        assert!(every_less_than_is_escaped("a \\\\\\<tg-button"));
        assert!(!every_less_than_is_escaped("a <tg-button"));
        // The shape that once passed two tests named for the guarantee.
        assert!(!every_less_than_is_escaped("a \\\\<tg-button"));
    }

    /// The guarantee: no bare `<` survives, in any context. These are every input the
    /// review rounds found against the earlier exemption-based and scanner-based
    /// designs; not one of them needs a special case now.
    #[test]
    fn no_raw_html_survives_any_context() {
        for context in [
            BUTTON.to_string(),
            format!("a `hello\n\n{BUTTON}\n\n` b"),
            format!("a `hello\r> {BUTTON}\r> ` b"),
            format!("a \\` {BUTTON} \\` b"),
            format!("\\{BUTTON}"),
            // Two `<` in a row after an author backslash: the first consumes the run,
            // so the second must be judged on its own. Dropping the counter reset in
            // the `<` arm leaves this one bare — a live tag — and every other test
            // still passes.
            format!("\\<{BUTTON}"),
            format!("\\\\<{BUTTON}"),
            format!("\\\\\\{BUTTON}"),
            format!("<b {BUTTON}"),
            format!("    ```\n{BUTTON}\n"),
            format!("```a`b\n{BUTTON}\n"),
            format!("```\n{BUTTON}\n```\n"),
            format!(r#"<b a="1"b="{BUTTON}">"#),
            format!("<a title=\"`\" href=\"https://ok\">t</a> {BUTTON}"),
        ] {
            let out = sanitize_rich_markdown(&context);
            assert!(
                every_less_than_is_escaped(&out),
                "unescaped `<` survived: {out}"
            );
        }
    }

    /// The pass must not change what the text *means*, only neutralise raw HTML. An
    /// earlier revision doubled every backslash, which un-escaped whatever the author
    /// had escaped: `[a\](https://x)` is not a link, and doubling made it one.
    #[test]
    fn author_written_escapes_are_preserved() {
        for source in [
            "\\*not italic\\*",
            "[a\\](https://example.com)",
            "a \\< b",
            "\\`not code\\`",
            "\\!\\[not an image](x)",
            // The `!` arm's parity guard: here `!` really is followed by `[`, so the
            // arm is reached and the guard is what stops it from escaping again. The
            // line above never reaches it — `!` is followed by `\\`.
            "\\![alt](https://example.com/x.jpg)",
        ] {
            assert_eq!(sanitize_rich_markdown(source), source);
        }
        // An even run is still not an escape, so the `<` gains one.
        assert_eq!(sanitize_rich_markdown("\\\\<b>"), "\\\\\\<b>");
    }

    #[test]
    fn markdown_formatting_is_untouched() {
        for source in [
            "| a | b |\n|:--|--:|\n| _x_ | **y _z_** |\n",
            "**bold _italic_ bold** ~~strike~~ ||spoiler|| `code`",
            "# Heading\n\n- item\n- item\n\n1. one\n2. two\n",
            "> quote\n> more\n\n---\n",
            "```rust\nfn main() {}\n```\n",
            "[x](https://example.com/a_(b)_c)",
            "[x][ref]\n\n[ref]: https://example.com",
            "[^id2]: Warning: do not do this.",
            "[call](tel:+123456789)",
        ] {
            assert_eq!(sanitize_rich_markdown(source), source);
        }
    }

    /// The documented cost of the guarantee, pinned so the trade-off is visible rather
    /// than discovered: Markdown does not process escapes inside code.
    #[test]
    fn escapes_land_inside_code_samples_too() {
        assert_eq!(
            sanitize_rich_markdown("```rust\nlet v: Vec<String>;\n```"),
            "```rust\nlet v: Vec\\<String>;\n```"
        );
        assert_eq!(
            sanitize_rich_markdown("```\n![x](https://a/b.png)\n```"),
            "```\n\\![x](https://a/b.png)\n```"
        );
    }

    #[test]
    fn image_syntax_is_escaped_so_media_is_not_fetched() {
        assert_eq!(
            sanitize_rich_markdown("see ![alt](https://evil.example/x.jpg)"),
            "see \\![alt](https://evil.example/x.jpg)"
        );
    }

    /// Link destinations are deliberately not filtered — see the module comment. Pinned
    /// so the decision stays visible and cannot be reverted by accident.
    #[test]
    fn link_destinations_are_passed_through_unfiltered() {
        for source in [
            "[x](javascript:alert(1))",
            "[y][x]\n\n[x]: javascript:alert(1)",
        ] {
            assert_eq!(sanitize_rich_markdown(source), source);
        }
    }

    /// Every rule is character-local, so the pass is linear on any input. These shapes
    /// each defeated the scanner-based design; the last cost 15 s on a megabyte.
    #[test]
    fn every_input_shape_is_processed_linearly() {
        for input in [
            "[".repeat(1_000_000),
            "[".repeat(1_000_000) + "]",
            ("[".repeat(998) + "]").repeat(1_001),
            "[]".repeat(500_000),
            "\\".repeat(1_000_000),
            "<".repeat(1_000_000),
        ] {
            let start = Instant::now();
            let _ = sanitize_rich_markdown(&input);
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "took {:?} on a {}-byte input",
                start.elapsed(),
                input.len()
            );
        }
    }

    #[test]
    fn multibyte_text_is_preserved() {
        let s = "таблица — да, 🎉 <tg-button>нет</tg-button>";
        let out = sanitize_rich_markdown(s);
        assert!(out.contains("таблица — да"));
        assert!(out.contains('🎉'));
        assert!(out.contains("\\<tg-button"));
        assert!(every_less_than_is_escaped(&out));
    }

    #[test]
    fn edge_inputs_do_not_panic() {
        for input in ["", " ", "<", "[", "![", "\\", "`", "\u{feff}", "\r", "&#"] {
            let _ = sanitize_rich_markdown(input);
        }
    }
}
