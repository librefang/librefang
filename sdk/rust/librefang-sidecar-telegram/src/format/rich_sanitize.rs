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
//! * `&` becomes `&amp;` and `<` becomes `&lt;`, unconditionally. No parity to track and
//!   no state to get wrong: the output simply contains no `<`, so no tag can be parsed.
//! * `!` before `[` is backslash-escaped, so `![](url)` stays inert text rather than
//!   becoming a media block fetched from that URL.
//!
//! Author-written backslashes are left alone, so `\*not italic\*` and `[a\](https://x)`
//! still mean what they meant. The one place this shows is `\<`: the backslash is kept and
//! the `<` still becomes `&lt;`, so the reader sees a stray backslash. That is a fidelity
//! cost of the guarantee, not a rewrite of intent — the author asked for a literal `<` and
//! gets one.
//!
//! `&lt;` rather than a backslash. The first version reasoned the other way — the Bot API
//! documents backslash for the `markdown` field by example (`\#hashtag` appears in its own
//! Rich Markdown sample) and says nothing about entity decoding there — and shipped. It was
//! wrong, and only a live bot could show it: **Telegram does not treat `\<` as an escape.**
//! The backslash is delivered as a character and the tag is parsed anyway, so a quoted
//! `<tg-button type="callback_data">` became a real button (#8127). Escaping does work for
//! Markdown syntax, which is why `\!` below is fine and why the mistake was plausible.
//!
//! Measured against `sendRichMessage`, which echoes its own parse back in the response:
//!
//! ```text
//! <tg-button …>T</tg-button>    -> {"type":"button", …}   raw
//! \<tg-button …>T</tg-button>   -> ["\\", {"type":"button", …}]   backslash, then a button
//! &lt;tg-button …&gt;T&lt;/tg-button&gt;  -> text
//! ```
//!
//! Every `<` needs it, not just the opening one: escaping only the first leaves
//! `</tg-button>` to be parsed and dropped, which silently truncates the quoted text. `&` is
//! escaped first so an author's literal `&lt;` arrives as those four characters rather than
//! decoding into a `<` this pass never inspected. `>` is left alone — with no `<` there is
//! no tag for it to close — so the output reads `&lt;/tg-button>`; the sample above escapes
//! both only because that is how the Bot API prints entities in its own examples.
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
//! The escapes are applied everywhere, including inside code spans and fenced blocks —
//! where, verified live, entities are *not* decoded, so `Vec<String>` in a fence reads
//! `Vec&lt;String>`. Uglier than the `Vec\<String>` it replaces and equally wrong; the
//! difference is that this version is actually safe. Every Rich HTML construct is lost,
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
    // Length of the run of backslashes immediately before the current position. Only the
    // `!` rule needs it: an odd run there is already an escape the author wrote.
    let mut backslashes = 0_usize;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                out.push('\\');
                backslashes += 1;
                i += 1;
            }
            // Escaped first so an author's literal `&lt;` survives as those four
            // characters instead of being decoded into a `<` we never inspected.
            b'&' => {
                out.push_str("&amp;");
                backslashes = 0;
                i += 1;
            }
            // The whole security property, in one arm: no `<` survives as itself, so no
            // tag can be parsed, whatever the surrounding text looks like.
            b'<' => {
                out.push_str("&lt;");
                backslashes = 0;
                i += 1;
            }
            // `![...](...)` is a real media block in Rich Markdown, fetched from the
            // URL — today it is inert text. Escaping the `!` keeps it that way, and
            // backslash *does* work for Markdown syntax characters, unlike for tags.
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

    /// The guarantee, as a predicate: no `<` survives as itself. `&lt;` is what Telegram
    /// actually honours — a backslash is not an escape for a tag, only for Markdown
    /// syntax, which is how the previous version let a quoted `<tg-button>` through as a
    /// live button while passing two tests named for the guarantee.
    fn no_bare_angle_bracket(out: &str) -> bool {
        !out.contains('<')
    }

    /// Pinned against the live API: `\\<tg-button …>` is parsed as a button, `&lt;…` is
    /// not. Both rows were run through `sendRichMessage` on a real bot, which echoes its
    /// parse back in the response.
    #[test]
    fn the_predicate_rejects_the_backslash_form_telegram_ignores() {
        assert!(no_bare_angle_bracket("a &lt;tg-button"));
        assert!(no_bare_angle_bracket("&amp;lt;tg-button"));
        // The shape that shipped: Telegram delivers the backslash as a character and
        // parses the tag anyway.
        assert!(!no_bare_angle_bracket("a \\<tg-button"));
        assert!(!no_bare_angle_bracket("a <tg-button"));
    }

    /// The one piece of sequencing in this pass, pinned on the output rather than on the
    /// order of the arms.
    ///
    /// Swapping the two `match` arms is a no-op — they match distinct bytes — so that
    /// particular mutation cannot fail. What the contract has to survive is a *rewrite*:
    /// the obvious `replace('<', "&lt;").replace('&', "&amp;")` produces `&amp;lt;` for a
    /// plain `<` and silently stops escaping anything. Asserting both directions catches
    /// it; asserting the predicate alone does not.
    #[test]
    fn an_emitted_entity_is_not_re_escaped_and_an_authors_is() {
        // What we emit must arrive as an entity, not as an escaped ampersand.
        assert_eq!(sanitize_rich_markdown("<x"), "&lt;x");
        // What the author wrote must arrive as their four characters.
        assert_eq!(sanitize_rich_markdown("&lt;x"), "&amp;lt;x");
        assert_eq!(sanitize_rich_markdown("&"), "&amp;");
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
            assert!(no_bare_angle_bracket(&out), "unescaped `<` survived: {out}");
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
            "\\`not code\\`",
            "\\!\\[not an image](x)",
            // The `!` arm's parity guard: here `!` really is followed by `[`, so the
            // arm is reached and the guard is what stops it from escaping again. The
            // line above never reaches it — `!` is followed by `\\`.
            "\\![alt](https://example.com/x.jpg)",
        ] {
            assert_eq!(sanitize_rich_markdown(source), source);
        }
        // `<` is the exception, and deliberately so: the author's backslash is kept but
        // cannot carry the escape, because Telegram does not honour `\\<`. The reader sees
        // a stray backslash before the character — the cost of the guarantee holding.
        assert_eq!(sanitize_rich_markdown("a \\< b"), "a \\&lt; b");
        assert_eq!(sanitize_rich_markdown("\\\\<b>"), "\\\\&lt;b>");
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
        // Verified against the live API: entities are *not* decoded inside a fence, so a
        // reader sees `Vec&lt;String>` verbatim. Worse-looking than the old `Vec\\<String>`
        // and equally wrong; `InputRichMessage.blocks` (#8015) is what removes it, since a
        // preformatted block's text is a plain string nothing parses.
        assert_eq!(
            sanitize_rich_markdown("```rust\nlet v: Vec<String>;\n```"),
            "```rust\nlet v: Vec&lt;String>;\n```"
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
        assert!(out.contains("&lt;tg-button"));
        assert!(no_bare_angle_bracket(&out));
    }

    #[test]
    fn edge_inputs_do_not_panic() {
        for input in ["", " ", "<", "[", "![", "\\", "`", "\u{feff}", "\r", "&#"] {
            let _ = sanitize_rich_markdown(input);
        }
    }
}
