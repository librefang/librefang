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
//! links, block quotes, dividers. Media blocks, collages, maps and `<details>` are out of
//! scope (#8015) and degrade to their text. Footnotes and math are not parsed at all —
//! their `pulldown-cmark` options are off — so `[^1]` is an ordinary shortcut reference and
//! `$x$` is ordinary text; neither is "degraded", both are simply never recognised.
//!
//! "Degrade to their text" is the property to hold on to, and it is the one that broke:
//! block-level HTML was routed through `push_inline`, which does nothing when no inline run
//! is open, so `<details>` converted to nothing at all. The dangerous shape was not the
//! all-HTML message — that produced no blocks and fell back — but the mixed one, where the
//! surviving paragraphs kept the result non-empty and the message went out with its middle
//! removed. One exception is deliberate: GFM specifies that table cells beyond the header
//! count are ignored, so those are dropped to match every other GFM renderer.

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
/// Never fails. Anything the converter does not model becomes the text it was written as,
/// so a gap in coverage costs formatting rather than information — except where CommonMark
/// itself says the text is not content:
///
/// * table cells past the header count ("If there are greater, the excess is ignored");
/// * link reference definitions, which "do not correspond to a structural element of a
///   document" — `[1]: https://example.com` on its own renders as nothing here, in GitHub,
///   and in any conformant renderer. A definition that *is* referenced still produces its
///   link, which is the case that carries the URL to the reader.
///
/// The legacy `sendMessage` path disagrees on both, because its converter is four regexes
/// rather than a parser, and it shows the raw line. That is a divergence from this path,
/// not a bug in it.
///
/// The unqualified "never drops content" that stood here was false, and four separate
/// content-loss defects were found under it. Treat any absolute in this module as a claim
/// that owes a test — including this one.
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
    /// Consecutive block-level HTML lines, which arrive one event per line with no
    /// paragraph around them. Held here so they become one paragraph instead of several.
    html_block: String,
    /// Task-list state, one entry per open list item. A marker arrives *inside* its item,
    /// after `Tag::Item` pushed the item's block frame, so the list frame is no longer on
    /// top and the marker cannot reach it. A single cell is not enough either: a nested
    /// item closes before its parent and would take the parent's value with it, moving a
    /// "done" tick onto a sub-point.
    item_checkboxes: Vec<Option<bool>>,
    /// True while the open inline run was started by loose text rather than by a
    /// `Tag::Paragraph`, so it has to be closed by hand at the next block boundary.
    implicit_run: bool,
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

    /// Emit any buffered block-level HTML as its own paragraph.
    ///
    /// Called from `push` on the first event that is not more block HTML. `pulldown-cmark`
    /// always closes a run with `End(HtmlBlock)`, so that event is the flush point and a
    /// second one in `finish` would be unreachable — it was there, and no mutation of it
    /// could fail a test, which is how it was found.
    fn flush_html_block(&mut self) {
        if self.html_block.is_empty() {
            return;
        }
        let html = std::mem::take(&mut self.html_block);
        let text = RichText::Plain(html.trim_end_matches('\n').to_string());
        self.push_block(Block::Paragraph { text });
    }

    fn start_inline(&mut self) {
        self.inlines.push(Vec::new());
    }

    fn take_inline(&mut self) -> RichText {
        RichText::from_parts(self.inlines.pop().unwrap_or_default())
    }

    /// Add to the open inline run, opening one if there is none.
    ///
    /// A *tight* list item emits its text with no `Tag::Paragraph` around it, so without
    /// this there is no run to add to and the text is dropped. Opening it lazily — rather
    /// than at `Tag::Item` — is what lets `push_block` close it at the right place: the
    /// text before a nested block and the text after it end up in separate paragraphs,
    /// in source order, instead of being concatenated into one.
    fn push_inline(&mut self, text: RichText) {
        if self.inlines.is_empty() {
            self.inlines.push(Vec::new());
            self.implicit_run = true;
        }
        if let Some(run) = self.inlines.last_mut() {
            run.push(text);
        }
    }

    /// Append a block, closing any loose text that came before it first.
    ///
    /// Every block push goes through here so source order is preserved by construction.
    /// The two obvious alternatives are both wrong: appending the item's text after its
    /// children puts sub-points above the point they belong to, and inserting it at the
    /// front does the same to a heading or a code fence the text followed.
    fn push_block(&mut self, block: Block) {
        self.flush_implicit_run();
        self.blocks_mut().push(block);
    }

    /// Close a loose-text run as its own paragraph.
    fn flush_implicit_run(&mut self) {
        if !self.implicit_run {
            return;
        }
        self.implicit_run = false;
        let text = RichText::from_parts(self.inlines.pop().unwrap_or_default());
        if !text.is_empty() {
            let block = Block::Paragraph { text };
            self.blocks_mut().push(block);
        }
    }

    fn push(&mut self, event: Event<'_>) {
        // Any event other than more block HTML ends the run of it.
        if !matches!(event, Event::Html(_)) {
            self.flush_html_block();
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_inline(RichText::Plain(text.into_string())),
            Event::Code(text) => self.push_inline(RichText::Styled(Styled::Code {
                text: Box::new(RichText::Plain(text.into_string())),
            })),
            Event::SoftBreak => self.push_inline(RichText::Plain(" ".into())),
            Event::HardBreak => self.push_inline(RichText::Plain("\n".into())),
            Event::Rule => self.push_block(Block::Divider),
            // Raw HTML is text, not markup: this is the whole point of `blocks`. A quoted
            // `<tg-button …>` lands in a string, where no parser will look at it.
            Event::InlineHtml(html) => self.push_inline(RichText::Plain(html.into_string())),
            // `Event::Html` is the *block-level* variant and arrives with no surrounding
            // paragraph, so there is no open inline run to push into. Routing it through
            // `push_inline` dropped it silently: `<details>…</details>` converted to nothing,
            // and a message mixing prose with an HTML block was sent with its middle
            // missing. Buffered here and flushed as its own paragraph.
            Event::Html(html) => self.html_block.push_str(&html),
            Event::TaskListMarker(checked) => {
                if let Some(slot) = self.item_checkboxes.last_mut() {
                    *slot = Some(checked);
                }
            }
            // `FootnoteReference` needs `ENABLE_FOOTNOTES`, and the math events need
            // `ENABLE_MATH`; neither is on, so neither event can arrive. Handling them
            // anyway was worse than dead weight — it read as evidence that footnotes were
            // covered, and that belief reached the module docs and the changelog.
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        // Opening a container is a block boundary just like emitting one: the loose text
        // before a nested list belongs to the item that owns it, not to the first child.
        // Without this, `- a\n  - b` collected "a" and "b" into one run and delivered "ab"
        // inside the nested item, with the parent's own text gone.
        // Only containers that push a *new* `Frame::Blocks` matter, because that is what
        // `flush_implicit_run` writes into. `Tag::List` and `Tag::Table` push other frame
        // kinds, which `blocks_mut` walks straight past, so listing them changed nothing.
        if matches!(tag, Tag::Item | Tag::BlockQuote(_)) {
            self.flush_implicit_run();
        }
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
                self.item_checkboxes.push(None);
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
                self.push_block(Block::Paragraph { text });
            }
            TagEnd::Heading(level) => {
                let text = self.take_inline();
                self.push_block(Block::Heading {
                    text,
                    size: heading_size(level),
                });
            }
            TagEnd::CodeBlock => {
                let text = self.take_inline();
                let language = self.code_language.take().flatten();
                self.push_block(Block::Pre {
                    text: trim_trailing_newline(text),
                    language,
                });
            }
            TagEnd::List(_) => {
                if let Some(Frame::ListItems { items, .. }) = self.frames.pop() {
                    self.push_block(Block::List { items });
                }
            }
            TagEnd::Item => {
                // Trailing loose text closes here; anything earlier was already closed at
                // the block that followed it.
                self.flush_implicit_run();
                let checkbox = self.item_checkboxes.pop().flatten();
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
                    items.push(ListItem {
                        blocks,
                        value,
                        label_type: value.map(|_| "1".to_string()),
                        has_checkbox: checkbox.map(|_| true),
                        // Only ever `Some(true)`: the spec types the field as `True`, so an
                        // unchecked box omits it rather than sending `false`.
                        is_checked: checkbox.filter(|c| *c),
                    });
                }
            }
            TagEnd::BlockQuote(_) => {
                if let Some(Frame::Blocks(blocks)) = self.frames.pop() {
                    self.push_block(Block::Quote { blocks });
                }
            }
            TagEnd::Table => {
                if let Some(Frame::TableRows { rows, .. }) = self.frames.pop() {
                    self.push_block(Block::Table { cells: rows });
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
        // Both stacks are popped together, before the run check can bail out. A span with
        // no text inside it — `- ![](x)` as a tight item's whole content — closes without
        // any run open, and popping only `styles` left the two stacks different lengths.
        let start = self.style_starts.pop().unwrap_or(0);
        let Some(run) = self.inlines.last_mut() else {
            return;
        };
        let start = start.min(run.len());
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
        // The style stacks move in lockstep, so both must be empty here. Asserting it is
        // what makes the invariant testable at all: an imbalance has no effect on the
        // output — the stray entry sits at the bottom and never resurfaces — so no
        // assertion on converted text can catch it, and a mutation that reintroduces it
        // otherwise survives every test in this file.
        debug_assert_eq!(
            self.styles.len(),
            self.style_starts.len(),
            "style stacks drifted apart"
        );
        debug_assert!(
            self.style_starts.is_empty(),
            "unclosed style spans: {:?}",
            self.style_starts
        );
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

    /// `Event::Html` is the block-level variant and arrives with no paragraph around it,
    /// so there is no open inline run to push into. Routing it through `push_inline` made
    /// it vanish: a `<details>` block converted to nothing, and — worse — a message mixing
    /// prose with an HTML block was *sent* with its middle missing, because the surviving
    /// paragraphs kept the conversion non-empty and the guard never fired.
    #[test]
    fn block_level_html_is_kept_as_text() {
        let details = "<details>\n<summary>why</summary>\n</details>";
        assert_eq!(
            json(details),
            serde_json::json!([{"type": "paragraph", "text": details}])
        );

        // The dangerous shape: content on both sides keeps the result non-empty, so
        // nothing would have reported the loss.
        assert_eq!(
            json("before\n\n<div>middle</div>\n\nafter"),
            serde_json::json!([
                {"type": "paragraph", "text": "before"},
                {"type": "paragraph", "text": "<div>middle</div>"},
                {"type": "paragraph", "text": "after"},
            ])
        );
    }

    /// A task-list marker arrives *inside* the item, after `Tag::Item` has pushed the item's
    /// block frame — so the list frame is no longer on top. The original handler looked for
    /// it there anyway, never matched, and both the marker and its companion branch in
    /// `TagEnd::Item` were dead code: `- [ ] x` and `- [x] x` serialised identically, with
    /// the `[ ]` / `[x]` characters consumed by the parser and never given back.
    #[test]
    fn task_list_checkboxes_survive() {
        assert_eq!(
            json("- [ ] todo\n- [x] done"),
            serde_json::json!([{"type": "list", "items": [
                {"blocks": [{"type": "paragraph", "text": "todo"}], "has_checkbox": true},
                {"blocks": [{"type": "paragraph", "text": "done"}],
                 "has_checkbox": true, "is_checked": true},
            ]}])
        );
        // A plain item must not grow a checkbox from the previous item's marker.
        let plain = json("- [x] done\n- plain");
        assert!(
            plain[0]["items"][1].get("has_checkbox").is_none(),
            "{plain}"
        );
    }

    /// A nested list closes while its parent item is still open, so it lands in the item's
    /// frame before the item's own text is appended — which printed every sub-point above
    /// the point it belongs to.
    #[test]
    fn nested_list_items_keep_source_order() {
        assert_eq!(
            json("- a\n  - b"),
            serde_json::json!([{"type": "list", "items": [{"blocks": [
                {"type": "paragraph", "text": "a"},
                {"type": "list", "items": [
                    {"blocks": [{"type": "paragraph", "text": "b"}]},
                ]},
            ]}]}])
        );
    }

    /// GFM, "Tables (extension)": "If there are greater, the excess is ignored." Pinned so
    /// the truncation reads as conformance rather than as the content loss it resembles.
    #[test]
    fn table_rows_follow_gfm_cell_counting() {
        assert_eq!(
            json("| a | b |\n|---|---|\n| 1 |\n| 1 | 2 | 3 |"),
            serde_json::json!([{"type": "table", "cells": [
                [{"text": "a", "is_header": true}, {"text": "b", "is_header": true}],
                [{"text": "1"}, {"text": ""}],
                [{"text": "1"}, {"text": "2"}],
            ]}])
        );
    }

    /// A tight item emits its text with no paragraph events, so all of it landed in one
    /// run — text before a nested block and text after it were concatenated with no
    /// separator at all. `- alpha\n  ***\n  beta` delivered `alphabeta`: the characters
    /// were gone from the message, not merely styled differently.
    #[test]
    fn loose_text_around_a_block_stays_separate_and_ordered() {
        assert_eq!(
            json("- alpha\n  ***\n  beta"),
            serde_json::json!([{"type": "list", "items": [{"blocks": [
                {"type": "paragraph", "text": "alpha"},
                {"type": "divider"},
                {"type": "paragraph", "text": "beta"},
            ]}]}])
        );
        // A heading the text follows must stay above it. Appending the item's text put
        // sub-points above their parent; inserting it at the front did this instead.
        assert_eq!(
            json("- # h\n  text"),
            serde_json::json!([{"type": "list", "items": [{"blocks": [
                {"type": "heading", "text": "h", "size": 1},
                {"type": "paragraph", "text": "text"},
            ]}]}])
        );
    }

    /// The shape an agent actually writes: a step, the command, then what to do next.
    #[test]
    fn a_checklist_step_around_a_code_fence_survives_intact() {
        assert_eq!(
            json("- [ ] Install deps\n  ```sh\n  npm i\n  ```\n  Then run it"),
            serde_json::json!([{"type": "list", "items": [{
                "blocks": [
                    {"type": "paragraph", "text": "Install deps"},
                    {"type": "pre", "text": "npm i", "language": "sh"},
                    {"type": "paragraph", "text": "Then run it"},
                ],
                "has_checkbox": true,
            }]}])
        );
    }

    /// One cell of checkbox state is not enough: a nested item closes before its parent and
    /// took the parent's tick with it, so `- [x] a\n  - b` marked `b` done and left `a`
    /// a plain bullet. One slot per open item.
    #[test]
    fn a_checkbox_belongs_to_its_own_item_not_a_nested_one() {
        let value = json("- [x] Ship it\n  - [ ] write tests");
        let outer = &value[0]["items"][0];
        assert_eq!(outer["is_checked"], serde_json::json!(true), "{value}");
        assert_eq!(outer["blocks"][0]["text"], "Ship it", "{value}");

        let inner = &outer["blocks"][1]["items"][0];
        assert_eq!(inner["has_checkbox"], serde_json::json!(true), "{value}");
        assert!(
            inner.get("is_checked").is_none(),
            "unchecked must be absent: {value}"
        );
    }

    /// `Tag::BlockQuote` is the one container start that has to close a loose run, and
    /// nothing pinned it: removing it passed all 144 tests. The damage is not loss or
    /// reordering — it is attribution. The item's own sentence is delivered *inside* the
    /// quotation, so on a change whose whole purpose is keeping quoted material
    /// distinguishable from the agent's own words, the agent's words become quoted.
    #[test]
    fn an_items_own_text_stays_outside_a_quote_it_contains() {
        assert_eq!(
            json("- mine\n  > theirs"),
            serde_json::json!([{"type": "list", "items": [{"blocks": [
                {"type": "paragraph", "text": "mine"},
                {"type": "blockquote", "blocks": [{"type": "paragraph", "text": "theirs"}]},
            ]}]}])
        );
    }

    /// A definition that is referenced still carries its URL to the reader; only an
    /// unreferenced one renders as nothing, which is what CommonMark specifies for it.
    #[test]
    fn reference_style_links_resolve_and_are_filtered() {
        assert_eq!(
            json("See [the spec][1].\n\n[1]: https://example.com/a"),
            serde_json::json!([{"type": "paragraph", "text": [
                "See ",
                {"type": "url", "text": "the spec", "url": "https://example.com/a"},
                ".",
            ]}])
        );
        // The scheme allowlist has to hold on this path too, where no `[text](url)` is
        // written anywhere in the source.
        assert_eq!(
            json("See [this][1].\n\n[1]: tg://resolve?domain=evil"),
            // The rejected link's text is plain, so it merges with its neighbours.
            serde_json::json!([{"type": "paragraph", "text": "See this."}])
        );
        // An unreferenced definition is not a structural element — CommonMark, and so
        // every conformant renderer, produces nothing for it.
        assert_eq!(
            json("Sources:\n\n[1]: https://example.com/a"),
            serde_json::json!([{"type": "paragraph", "text": "Sources:"}])
        );
    }

    /// A span carrying no text closes with no inline run open. Popping only `styles` there
    /// left `styles` and `style_starts` at different depths — harmless today because the
    /// stray entry sits at the bottom and never resurfaces, but the invariant is what the
    /// next change in this area would rely on.
    #[test]
    fn an_empty_span_leaves_the_style_stacks_balanced() {
        for input in ["- ![](x)", "- [](https://e.com)", "![]()", "*[]()*"] {
            let _ = markdown_to_blocks(input);
        }
        // Observable proxy for the invariant: a later span must still wrap the right text.
        assert_eq!(
            json("- ![](x)\n- **bold**"),
            serde_json::json!([{"type": "list", "items": [
                {"blocks": []},
                {"blocks": [{"type": "paragraph", "text": {"type": "bold", "text": "bold"}}]},
            ]}])
        );
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
