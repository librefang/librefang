//! Build `InputRichMessage.blocks` from the Markdown an agent writes.
//!
//! # Why this exists
//!
//! The `markdown` field hands Telegram a string and lets *Telegram* parse it. Our text is
//! model output that routinely quotes untrusted content, and Rich Markdown admits arbitrary
//! HTML, so a quoted `<tg-button type="callback_data">` would come back to the adapter as a
//! genuine `ButtonCallback` with an attacker-chosen payload. `rich_sanitize` defends against
//! that by escaping every `<` — provable, but it also escapes inside fenced code, so a code
//! sample containing `Vec<String>` reaches the reader as `Vec\<String>`.
//!
//! `blocks` removes the problem instead of mitigating it. Telegram runs no Markdown or HTML
//! parser over a block's text: `{"type":"paragraph","text":"…"}` carries a literal string.
//! A button can only exist as `RichTextButton`, an object we simply never construct, so no
//! input can produce one. The parser moves to our side of the wire, where a mistake in it
//! makes the italics wrong rather than making a button real. That is the same reason a
//! parameterised query beats escaping quotes.
//!
//! # Scope
//!
//! What an agent actually writes: paragraphs, headings, lists, tables, code, emphasis,
//! links, block quotes, dividers. Media blocks, collages, maps, `<details>`, math and
//! footnotes are out of scope (#8015) and degrade to their text, never to nothing.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Serialize;

/// One `InputRichBlock`. Only the variants we build; the spec has two dozen more.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum Block {
    #[serde(rename = "paragraph")]
    Paragraph { text: RichText },
    #[serde(rename = "heading")]
    Heading { text: RichText, size: u8 },
    #[serde(rename = "pre")]
    Pre {
        text: RichText,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    #[serde(rename = "list")]
    List { items: Vec<ListItem> },
    #[serde(rename = "blockquote")]
    Quote { blocks: Vec<Block> },
    #[serde(rename = "table")]
    Table { cells: Vec<Vec<TableCell>> },
    #[serde(rename = "divider")]
    Divider,
}

/// An `InputRichBlockListItem`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListItem {
    pub blocks: Vec<Block>,
    /// Ordered lists carry their own numbering; Telegram renders bullets when it is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// Spec field name is bare `type`. The prose reads "…value of the item label" /
    /// "type — the type of the item label", which is easy to misread as one field
    /// called `label_type`; Telegram ignores unknown fields silently, so that spelling
    /// failed without any error and the numbering style was simply lost.
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub label_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_checkbox: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_checked: Option<bool>,
}

/// A `RichBlockTableCell`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableCell {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<RichText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_header: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
}

/// A `RichText`: a bare string, a sequence, or one styled span wrapping more `RichText`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RichText {
    Plain(String),
    Seq(Vec<RichText>),
    Styled(Styled),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum Styled {
    #[serde(rename = "bold")]
    Bold { text: Box<RichText> },
    #[serde(rename = "italic")]
    Italic { text: Box<RichText> },
    #[serde(rename = "strikethrough")]
    Strikethrough { text: Box<RichText> },
    #[serde(rename = "code")]
    Code { text: Box<RichText> },
    #[serde(rename = "url")]
    Url { text: Box<RichText>, url: String },
}

impl RichText {
    /// Whether this carries no characters at all. Used to tell a tight list item's text
    /// from the empty run a loose item leaves behind.
    fn is_empty(&self) -> bool {
        match self {
            RichText::Plain(s) => s.is_empty(),
            RichText::Seq(parts) => parts.iter().all(RichText::is_empty),
            RichText::Styled(_) => false,
        }
    }

    /// Collapse a run of spans into the simplest equivalent shape. A single `Plain` stays a
    /// bare string rather than becoming a one-element array, which keeps the payload small
    /// and the golden tests readable.
    fn from_parts(mut parts: Vec<RichText>) -> RichText {
        // Adjacent plain text arrives split across events (`Text`, `SoftBreak`, `Text`);
        // merging keeps the tree shallow.
        let mut merged: Vec<RichText> = Vec::with_capacity(parts.len());
        for part in parts.drain(..) {
            match (merged.last_mut(), part) {
                (Some(RichText::Plain(prev)), RichText::Plain(next)) => prev.push_str(&next),
                (_, part) => merged.push(part),
            }
        }
        match merged.len() {
            0 => RichText::Plain(String::new()),
            1 => merged.pop().expect("length checked"),
            _ => RichText::Seq(merged),
        }
    }
}

/// Convert an agent's Markdown into rich blocks.
///
/// Never fails and never drops content: anything the converter does not model becomes the
/// text it was written as, so a gap in coverage costs formatting, not information.
pub fn markdown_to_blocks(markdown: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut state = Builder {
        // The document frame; `blocks_mut` relies on it always being there.
        frames: vec![Frame::Blocks(Vec::new())],
        ..Builder::default()
    };
    for event in Parser::new_ext(markdown, options) {
        state.push(event);
    }
    state.finish()
}

/// A container being filled. Blocks nest (a list item holds blocks, a quote holds blocks),
/// so the builder keeps an explicit stack rather than recursing over the event stream.
#[derive(Debug)]
enum Frame {
    Blocks(Vec<Block>),
    ListItems {
        items: Vec<ListItem>,
        /// `Some(n)` for an ordered list, counting up from its start.
        next_value: Option<u64>,
    },
    TableRows {
        rows: Vec<Vec<TableCell>>,
        in_header: bool,
        alignments: Vec<Option<String>>,
    },
    TableRow {
        cells: Vec<TableCell>,
    },
}

#[derive(Default, Debug)]
struct Builder {
    /// Stack of block containers; the bottom frame is the document.
    frames: Vec<Frame>,
    /// Stack of inline runs; the bottom is the current block's text.
    inlines: Vec<Vec<RichText>>,
    /// Pending styled wrappers, innermost last.
    styles: Vec<StyleKind>,
    /// Where each pending wrapper started inside the current run, so closing it takes
    /// exactly the children it collected and not whatever preceded it.
    style_starts: Vec<usize>,
    /// Set while inside a fenced or indented code block, holding its info string.
    code_language: Option<Option<String>>,
}

#[derive(Debug, Clone)]
enum StyleKind {
    Bold,
    Italic,
    Strikethrough,
    Url(String),
}

impl Builder {
    fn blocks_mut(&mut self) -> &mut Vec<Block> {
        // Every path that pushes a block has a `Frame::Blocks` on top; `start_blocks` is
        // called for each container before its children are parsed.
        for frame in self.frames.iter_mut().rev() {
            if let Frame::Blocks(blocks) = frame {
                return blocks;
            }
        }
        unreachable!("the document frame is always a Frame::Blocks")
    }

    fn start_inline(&mut self) {
        self.inlines.push(Vec::new());
    }

    fn take_inline(&mut self) -> RichText {
        RichText::from_parts(self.inlines.pop().unwrap_or_default())
    }

    fn push_inline(&mut self, text: RichText) {
        if let Some(run) = self.inlines.last_mut() {
            run.push(text);
        }
    }

    fn push(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_inline(RichText::Plain(text.into_string())),
            Event::Code(text) => self.push_inline(RichText::Styled(Styled::Code {
                text: Box::new(RichText::Plain(text.into_string())),
            })),
            Event::SoftBreak => self.push_inline(RichText::Plain(" ".into())),
            Event::HardBreak => self.push_inline(RichText::Plain("\n".into())),
            Event::Rule => {
                let block = Block::Divider;
                self.blocks_mut().push(block);
            }
            // Raw HTML is text, not markup: this is the whole point of `blocks`. A quoted
            // `<tg-button …>` lands in a paragraph's string, where no parser will look at it.
            Event::Html(html) | Event::InlineHtml(html) => {
                self.push_inline(RichText::Plain(html.into_string()))
            }
            Event::TaskListMarker(checked) => {
                if let Some(Frame::ListItems { items, .. }) = self.frames.last_mut() {
                    // The marker arrives *inside* the item, so the item is not built yet;
                    // stash the state on the list and apply it when the item closes.
                    items.push(ListItem {
                        blocks: Vec::new(),
                        value: None,
                        label_type: None,
                        has_checkbox: Some(true),
                        is_checked: checked.then_some(true),
                    });
                }
            }
            Event::FootnoteReference(name) => {
                self.push_inline(RichText::Plain(format!("[^{name}]")))
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                self.push_inline(RichText::Plain(text.into_string()))
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph | Tag::Heading { .. } => self.start_inline(),
            Tag::CodeBlock(kind) => {
                self.code_language = Some(match kind {
                    CodeBlockKind::Fenced(info) => {
                        // Only the first word is the language; ```rust,no_run is common.
                        let lang = info.split(|c: char| c.is_whitespace() || c == ',').next();
                        lang.filter(|l| !l.is_empty()).map(str::to_string)
                    }
                    CodeBlockKind::Indented => None,
                });
                self.start_inline();
            }
            Tag::List(start) => self.frames.push(Frame::ListItems {
                items: Vec::new(),
                next_value: start,
            }),
            Tag::Item => {
                self.frames.push(Frame::Blocks(Vec::new()));
                // A *tight* list emits its item text with no surrounding paragraph, so
                // without a run open here that text has nowhere to land and is silently
                // dropped. A loose item pushes its own run on top of this one and leaves
                // this one empty, which `TagEnd::Item` discards.
                self.start_inline();
            }
            Tag::BlockQuote(_) => self.frames.push(Frame::Blocks(Vec::new())),
            Tag::Table(alignments) => self.frames.push(Frame::TableRows {
                rows: Vec::new(),
                in_header: false,
                alignments: alignments.iter().map(alignment_name).collect(),
            }),
            Tag::TableHead => {
                if let Some(Frame::TableRows { in_header, .. }) = self.frames.last_mut() {
                    *in_header = true;
                }
                self.frames.push(Frame::TableRow { cells: Vec::new() });
            }
            Tag::TableRow => self.frames.push(Frame::TableRow { cells: Vec::new() }),
            Tag::TableCell => self.start_inline(),
            Tag::Emphasis => self.open_style(StyleKind::Italic),
            Tag::Strong => self.open_style(StyleKind::Bold),
            Tag::Strikethrough => self.open_style(StyleKind::Strikethrough),
            Tag::Link { dest_url, .. } => self.open_style(StyleKind::Url(dest_url.into_string())),
            // An image is not a media block here (out of scope): its alt text is kept so the
            // reader still sees what it described.
            Tag::Image { .. } => self.open_style(StyleKind::Italic),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let text = self.take_inline();
                let block = Block::Paragraph { text };
                self.blocks_mut().push(block);
            }
            TagEnd::Heading(level) => {
                let text = self.take_inline();
                let block = Block::Heading {
                    text,
                    size: heading_size(level),
                };
                self.blocks_mut().push(block);
            }
            TagEnd::CodeBlock => {
                let text = self.take_inline();
                let language = self.code_language.take().flatten();
                let block = Block::Pre {
                    text: trim_trailing_newline(text),
                    language,
                };
                self.blocks_mut().push(block);
            }
            TagEnd::List(_) => {
                if let Some(Frame::ListItems { items, .. }) = self.frames.pop() {
                    let block = Block::List { items };
                    self.blocks_mut().push(block);
                }
            }
            TagEnd::Item => {
                let loose_text = self.take_inline();
                if !loose_text.is_empty() {
                    let block = Block::Paragraph { text: loose_text };
                    self.blocks_mut().push(block);
                }
                let blocks = match self.frames.pop() {
                    Some(Frame::Blocks(blocks)) => blocks,
                    other => {
                        if let Some(frame) = other {
                            self.frames.push(frame);
                        }
                        return;
                    }
                };
                if let Some(Frame::ListItems { items, next_value }) = self.frames.last_mut() {
                    let value = next_value.inspect(|v| *next_value = Some(v + 1));
                    // A task-list marker pre-created the item; fill it in rather than
                    // adding a second one.
                    match items.last_mut() {
                        Some(item) if item.blocks.is_empty() && item.has_checkbox.is_some() => {
                            item.blocks = blocks;
                            item.value = value;
                            item.label_type = value.map(|_| "1".to_string());
                        }
                        _ => items.push(ListItem {
                            blocks,
                            value,
                            label_type: value.map(|_| "1".to_string()),
                            has_checkbox: None,
                            is_checked: None,
                        }),
                    }
                }
            }
            TagEnd::BlockQuote(_) => {
                if let Some(Frame::Blocks(blocks)) = self.frames.pop() {
                    let block = Block::Quote { blocks };
                    self.blocks_mut().push(block);
                }
            }
            TagEnd::Table => {
                if let Some(Frame::TableRows { rows, .. }) = self.frames.pop() {
                    let block = Block::Table { cells: rows };
                    self.blocks_mut().push(block);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Frame::TableRow { cells }) = self.frames.pop() {
                    if let Some(Frame::TableRows {
                        rows, in_header, ..
                    }) = self.frames.last_mut()
                    {
                        *in_header = false;
                        rows.push(cells);
                    }
                }
            }
            TagEnd::TableCell => {
                let text = self.take_inline();
                let (is_header, align) = match self
                    .frames
                    .iter()
                    .rev()
                    .find_map(|f| match f {
                        Frame::TableRows {
                            in_header,
                            alignments,
                            ..
                        } => Some((*in_header, alignments.clone())),
                        _ => None,
                    })
                    .unzip()
                {
                    (Some(h), Some(a)) => (h, a),
                    _ => (false, Vec::new()),
                };
                let column = match self.frames.last() {
                    Some(Frame::TableRow { cells }) => cells.len(),
                    _ => 0,
                };
                if let Some(Frame::TableRow { cells }) = self.frames.last_mut() {
                    cells.push(TableCell {
                        text: Some(text),
                        is_header: is_header.then_some(true),
                        align: align.get(column).cloned().flatten(),
                    });
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Link
            | TagEnd::Image => self.close_style(),
            _ => {}
        }
    }

    /// Remember where a styled span begins so `close_style` can take exactly its children.
    fn open_style(&mut self, kind: StyleKind) {
        self.style_starts
            .push(self.inlines.last().map_or(0, Vec::len));
        self.styles.push(kind);
    }

    /// Wrap everything the span collected. The children were appended to the *current* run,
    /// so the wrapper takes them from where the span started.
    fn close_style(&mut self) {
        let Some(kind) = self.styles.pop() else {
            return;
        };
        let Some(run) = self.inlines.last_mut() else {
            return;
        };
        let start = self.style_starts.pop().unwrap_or(0).min(run.len());
        let inner = RichText::from_parts(run.split_off(start));
        let text = Box::new(inner);
        run.push(RichText::Styled(match kind {
            StyleKind::Bold => Styled::Bold { text },
            StyleKind::Italic => Styled::Italic { text },
            StyleKind::Strikethrough => Styled::Strikethrough { text },
            // A link is the one interactive element this converter builds out of model
            // output, so its destination is the one place quoted content could still steer
            // something. Unlike #8003's sanitiser — which was guessing where a link *might*
            // be inside someone else's parse — we hold the destination here, exactly the
            // position `sanitize_telegram_html` is in when it filters an `<a href>` it built
            // itself. Filtering is cheap and correct rather than a guess.
            //
            // A rejected scheme drops the *link*, not the text: the reader still sees what
            // was written, the same way an out-of-scope image degrades to its alt text.
            StyleKind::Url(url) if !scheme_is_allowed(&url) => return run.push(*text),
            StyleKind::Url(url) => Styled::Url { text, url },
        }));
    }

    fn finish(mut self) -> Vec<Block> {
        while self.frames.len() > 1 {
            self.frames.pop();
        }
        match self.frames.pop() {
            Some(Frame::Blocks(blocks)) => blocks,
            _ => Vec::new(),
        }
    }
}

/// Characters of visible text a block tree carries, which is what Telegram's 32768-character
/// rich-message limit counts.
///
/// Measuring the *source* Markdown instead over-counts by every syntax character the
/// converter consumes — a table's `|`, `-` and `:` are a large share of its source and none
/// of them survive — so text whose block form fits comfortably could be turned away.
pub fn text_len(blocks: &[Block]) -> usize {
    blocks.iter().map(block_text_len).sum()
}

fn block_text_len(block: &Block) -> usize {
    match block {
        Block::Paragraph { text } | Block::Heading { text, .. } | Block::Pre { text, .. } => {
            rich_text_len(text)
        }
        Block::List { items } => items.iter().map(|i| text_len(&i.blocks)).sum(),
        Block::Quote { blocks } => text_len(blocks),
        Block::Table { cells } => cells
            .iter()
            .flatten()
            .map(|c| c.text.as_ref().map_or(0, rich_text_len))
            .sum(),
        Block::Divider => 0,
    }
}

fn rich_text_len(text: &RichText) -> usize {
    match text {
        RichText::Plain(s) => s.chars().count(),
        RichText::Seq(parts) => parts.iter().map(rich_text_len).sum(),
        RichText::Styled(styled) => rich_text_len(match styled {
            Styled::Bold { text }
            | Styled::Italic { text }
            | Styled::Strikethrough { text }
            | Styled::Code { text }
            | Styled::Url { text, .. } => text,
        }),
    }
}

/// Schemes a converted link may carry.
///
/// Matches the allowlist `sanitize::sanitize_telegram_html` enforces on the fallback path,
/// minus `tg:`. The two paths should not disagree about what is a safe destination, and
/// `tg:` is the one scheme where a tap has an in-app consequence rather than opening a page
/// behind the client's "Open this link?" confirmation — a deep link can join a channel or
/// open a bot, which is not something quoted content should be able to offer.
const ALLOWED_LINK_SCHEMES: [&str; 3] = ["https", "http", "mailto"];

/// Whether a link destination may be kept.
///
/// A destination with no scheme at all is rejected: Telegram resolves such a link against
/// its own base, and what that resolves to is not ours to reason about.
fn scheme_is_allowed(url: &str) -> bool {
    match url.split_once(':') {
        // A `/` or `?` before the colon means the colon belongs to the path or query, not to
        // a scheme — `foo/bar:baz` is relative, not a `foo/bar` scheme.
        Some((scheme, _)) if !scheme.contains('/') && !scheme.contains('?') => ALLOWED_LINK_SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed)),
        _ => false,
    }
}

fn heading_size(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn alignment_name(alignment: &pulldown_cmark::Alignment) -> Option<String> {
    use pulldown_cmark::Alignment::*;
    match alignment {
        None => Option::None,
        Left => Some("left".into()),
        Center => Some("center".into()),
        Right => Some("right".into()),
    }
}

/// Fenced code arrives with the newline that closed its last line; Telegram renders it.
fn trim_trailing_newline(text: RichText) -> RichText {
    match text {
        RichText::Plain(s) => RichText::Plain(s.trim_end_matches('\n').to_string()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(markdown: &str) -> serde_json::Value {
        serde_json::to_value(markdown_to_blocks(markdown)).expect("blocks serialise")
    }

    /// The reason this module exists. Under `markdown` a quoted button is a live button
    /// unless a sanitiser catches it; here it is a string in a field Telegram never parses,
    /// and `RichTextButton` — the only shape a button can take — is never constructed.
    #[test]
    fn quoted_html_becomes_text_not_markup() {
        let quoted = r#"the page said <tg-button type="callback_data" data="wipe">Tap</tg-button>"#;
        assert_eq!(
            json(quoted),
            serde_json::json!([{"type": "paragraph", "text": quoted}])
        );
    }

    /// The fidelity the escaping pass costs: `rich_sanitize` turns this into `Vec\<String>`
    /// because Markdown does not process escapes inside code, and the reader sees the
    /// backslash. A preformatted block carries its text verbatim.
    #[test]
    fn code_samples_keep_their_angle_brackets() {
        assert_eq!(
            json("```rust\nlet v: Vec<String> = vec![];\n```"),
            serde_json::json!([{
                "type": "pre",
                "text": "let v: Vec<String> = vec![];",
                "language": "rust",
            }])
        );
    }

    #[test]
    fn inline_styles_nest() {
        assert_eq!(
            json("**bold _italic_** ~~gone~~ `code` [x](https://e.com)"),
            serde_json::json!([{"type": "paragraph", "text": [
                {"type": "bold", "text": ["bold ", {"type": "italic", "text": "italic"}]},
                " ",
                {"type": "strikethrough", "text": "gone"},
                " ",
                {"type": "code", "text": "code"},
                " ",
                {"type": "url", "text": "x", "url": "https://e.com"},
            ]}])
        );
    }

    /// A tight list emits no paragraph events around its item text. An earlier revision had
    /// nowhere to put that text and dropped it: every item came out `{"blocks": []}`, so the
    /// list rendered as the right number of empty bullets.
    #[test]
    fn tight_list_items_keep_their_text() {
        assert_eq!(
            json("- one\n- **two**"),
            serde_json::json!([{"type": "list", "items": [
                {"blocks": [{"type": "paragraph", "text": "one"}]},
                {"blocks": [{"type": "paragraph", "text": {"type": "bold", "text": "two"}}]},
            ]}])
        );
    }

    /// The list-item label field is spelled `type`, not `label_type`. Telegram ignores
    /// unknown fields without error, so the wrong spelling lost the numbering silently.
    #[test]
    fn ordered_lists_carry_their_numbering() {
        let value = json("3. three\n4. four");
        let items = value[0]["items"].as_array().expect("items");
        assert_eq!(items[0]["value"], 3);
        assert_eq!(items[0]["type"], "1");
        assert_eq!(items[1]["value"], 4);
        assert!(
            items[0].get("label_type").is_none(),
            "the field is `type`; `label_type` is silently dropped by Telegram"
        );
    }

    #[test]
    fn tables_carry_headers_and_alignment() {
        assert_eq!(
            json("| a | b |\n|:--|--:|\n| 1 | 2 |"),
            serde_json::json!([{"type": "table", "cells": [
                [{"text": "a", "is_header": true, "align": "left"},
                 {"text": "b", "is_header": true, "align": "right"}],
                [{"text": "1", "align": "left"}, {"text": "2", "align": "right"}],
            ]}])
        );
    }

    #[test]
    fn headings_blockquotes_and_dividers() {
        assert_eq!(
            json("### h\n\n> quoted\n\n---"),
            serde_json::json!([
                {"type": "heading", "text": "h", "size": 3},
                {"type": "blockquote", "blocks": [{"type": "paragraph", "text": "quoted"}]},
                {"type": "divider"},
            ])
        );
    }

    /// Out-of-scope constructs must degrade to their text rather than vanish. Losing content
    /// is worse than losing formatting: the reader cannot tell it happened.
    #[test]
    fn unmodelled_constructs_keep_their_text() {
        for (markdown, needle) in [
            ("![alt text](https://e.com/a.png)", "alt text"),
            ("term\n: definition", "definition"),
            ("$x^2$", "x^2"),
        ] {
            let rendered = json(markdown).to_string();
            assert!(
                rendered.contains(needle),
                "{markdown:?} lost {needle:?}: {rendered}"
            );
        }
    }

    /// A link is the one interactive element built from model output, so a quoted
    /// `[Tap here](tg://resolve?domain=x)` would otherwise become a tappable deep link —
    /// quoted content turning itself into something interactive, which is what this module
    /// exists to prevent. The link goes; the text stays.
    #[test]
    fn only_safe_link_schemes_survive() {
        assert_eq!(
            json("[ok](https://example.com)"),
            serde_json::json!([{"type": "paragraph", "text":
                {"type": "url", "text": "ok", "url": "https://example.com"}}])
        );
        // Schemes are case-insensitive, so an uppercase one must still be kept.
        assert_eq!(json("[u](HTTPS://e.com)")[0]["text"]["type"], "url");
        assert_eq!(json("[m](mailto:a@b.c)")[0]["text"]["type"], "url");

        for markdown in [
            "[Tap here](tg://resolve?domain=evil)",
            "[Tap here](javascript:alert(1))",
            "[Tap here](data:text/html,<script>)",
            "[Tap here](/relative/path)",
            "[Tap here](#anchor)",
        ] {
            let value = json(markdown);
            assert_eq!(
                value[0]["text"],
                serde_json::json!("Tap here"),
                "{markdown} kept its destination: {value}"
            );
        }
    }

    #[test]
    fn text_len_counts_only_delivered_text() {
        let blocks = markdown_to_blocks("| a | b |\n|:--|--:|\n| 1 | 2 |");
        assert_eq!(text_len(&blocks), 4, "only the four cell characters count");

        // Headings, quotes, list items and code all contribute; a divider does not.
        let mixed = markdown_to_blocks("# hi\n\n> q\n\n- x\n\n```\nc\n```\n\n---");
        assert_eq!(text_len(&mixed), 5);
    }

    #[test]
    fn edge_inputs_do_not_panic() {
        for input in [
            "", " ", "#", "- ", "|", "```", "> ", "***", "\u{feff}", "\r\n",
        ] {
            let _ = markdown_to_blocks(input);
        }
    }
}
