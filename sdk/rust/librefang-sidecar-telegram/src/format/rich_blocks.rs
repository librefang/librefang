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
//! links, block quotes, dividers, spoilers and highlights. The last two follow Telegram's
//! own pairing rules, read off the live parser rather than guessed — see `scan_delimiters`. Media blocks, collages, maps and `<details>` are out of
//! scope (#8015) and degrade to their text. Footnotes and math are not parsed at all —
//! their `pulldown-cmark` options are off — so `[^1]` is an ordinary shortcut reference and
//! `$x$` is ordinary text; neither is "degraded", both are simply never recognised.
//!
//! One divergence from Telegram is known and left alone: a *single* tilde, `~x~`, becomes
//! strikethrough here and is literal text to Telegram. `pulldown-cmark` reads one tilde as
//! strikethrough whenever `ENABLE_STRIKETHROUGH` is on and `ENABLE_SUBSCRIPT` is not, and
//! offers no way to require the doubled form. Cosmetic — emphasis the author did not ask
//! for — and recorded here so it is not rediscovered as a bug (#8347).
//!
//! Subscript and superscript are deliberately *not* enabled although the parser offers them
//! and Telegram accepts the types: Telegram's own Markdown leaves `~x~` and `^x^` as plain
//! text, so emitting them would invent formatting from syntax the author's target parser
//! ignores.
//!
//! "Degrade to their text" is the property to hold on to, and it is the one that broke:
//! block-level HTML was routed through `push_inline`, which does nothing when no inline run
//! is open, so `<details>` converted to nothing at all. The dangerous shape was not the
//! all-HTML message — that produced no blocks and fell back — but the mixed one, where the
//! surviving paragraphs kept the result non-empty and the message went out with its middle
//! removed. One exception is deliberate: GFM specifies that table cells beyond the header
//! count are ignored, so those are dropped to match every other GFM renderer.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
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
    #[serde(rename = "spoiler")]
    Spoiler { text: Box<RichText> },
    #[serde(rename = "marked")]
    Marked { text: Box<RichText> },
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

/// Telegram's own Rich Markdown understands `||spoiler||` and `==highlight==`; CommonMark
/// does not, so `pulldown-cmark` hands them through as ordinary characters and they were
/// lost when this path replaced the Markdown one. Both worked before, because the sanitiser
/// had no reason to touch `|` or `=`.
///
/// # Matching Telegram rather than guessing
///
/// The rules below are read off the live parser, which echoes its own parse back in
/// `sendRichMessage`'s response. Every one of them is a measurement; the recording they come
/// from is `telegram_oracle.json` and `pairing_matches_telegram` is the diff against it.
///
/// ```text
/// ||a||        -> spoiler      || a||     -> literal (opener followed by a space)
/// x||a||y      -> spoiler      ||a ||     -> literal (closer preceded by a space)
/// ||a\u{a0}||  -> spoiler      ||a\t||    -> literal (only ASCII counts as space)
/// ||a\nb||     -> literal      ||||       -> literal (the content may not be empty)
/// =====        -> marked "="   \|\|a\|\|  -> literal (nobody wrote a delimiter)
/// ```
///
/// Four of these cost a round of review each, so they are worth stating plainly:
///
/// * **The space is ASCII.** `char::is_whitespace` is the Unicode property, and Telegram
///   does not use it: NBSP, thin space and the rest are content, so `||секрет\u{a0}||` is
///   a spoiler there. NBSP is common in Russian text and in model output.
/// * **A pair does not cross a line break.** Not because a newline is whitespace — it is a
///   barrier: `||a\nb||` is literal even though `a` and `b` flank the delimiters. This is
///   the form agents write most, a long phrase wrapped by source width.
/// * **Rejecting a closer shifts by one character, not by two.** `=====` is `marked` over a
///   single `=`, and a rule line of twenty `=` is four of them. A version that stepped by a
///   whole delimiter mangled every long run differently and pinned the result in a test as
///   a known divergence, which is how a bug gets to look like a decision.
/// * **A delimiter that was not written as one is text.** `\|\|a\|\|` and
///   `&#124;&#124;a&#124;&#124;` are both literal for Telegram. `pulldown-cmark` hands over a
///   decoded `|` either way, so the builder compares each text event with the source it came
///   from and records the characters nobody typed; `Flat::seams` holds them, and a delimiter
///   touching one is not a delimiter.
///
/// # What is deliberately not modelled
///
/// Telegram serialises styles that cover exactly the same characters in an order of its own,
/// and that order is not a property of the styles: `**==a==**` comes back with `marked`
/// outside `bold` while `==**a**==` comes back the other way round. The rendering is
/// identical either way and the set of styles is the same, so this converter emits them in
/// source order and `styles_match_telegram_up_to_order` pins exactly that — the same styles
/// over the same text, in whatever order. A previous version invented a fixed ranking,
/// wrote it into the docs as Telegram's, and was wrong on four of the six forms it covered.
///
/// # Shape
///
/// The run is flattened once into a byte string — `Plain` text verbatim, one `OPAQUE`
/// placeholder per node the scan must not look inside — and all positions are offsets into
/// it. That single coordinate system is what makes "the content is not empty" a subtraction:
/// an earlier version compared `(node, offset)` tuples, and the end of node `i` and the start
/// of node `i + 1` are the same point written two ways, so a pair straddling a node boundary
/// passed the check and produced an empty span with the text inside it gone.
///
/// Work is linear in the length of the run, and the reason is that **every pointer only moves
/// forward**: the next opener, the next valid closer, and the cursor. Two earlier versions
/// were quadratic — 12 s, then 16.8 s, then 56 s on a megabyte — and each time the cause was
/// a search that started over: first from the beginning, then from the cursor, then from the
/// cursor whenever the *other* delimiter had consumed the last candidate. A closer is now
/// found once and kept until it is used or passed, which is sound because a later opener's
/// content starts later: a closer rejected for landing at or before the content start is
/// rejected for every later opener too, and the "not preceded by a space" test does not
/// depend on the opener at all.
///
/// Recursion goes only *into* matched content, never along the tail: 4 500 pairs — a 22 KB
/// message, well inside the 32 768 limit — overflowed a 2 MiB tokio worker stack and aborted
/// the sidecar process, taking the whole channel down rather than one message.
fn scan_delimiters(parts: Vec<RichText>) -> Vec<RichText> {
    let flat = Flat::new(&parts);
    let mut out = Vec::new();
    let mut line = 0;
    // A pair never crosses a line break, so each line is paired on its own. This is also what
    // makes "this delimiter has no closer left" sound: it is a statement about one line.
    for (at, byte) in flat.text.bytes().enumerate() {
        // A newline that arrived as `&#10;` is not a line break: Telegram pairs across it and
        // shows it as a space.
        if byte == b'\n' && flat.seams.binary_search(&at).is_err() {
            out.extend(flat.pair(line, at));
            // A soft break renders as the space it stood for; a hard one stays a newline.
            out.push(RichText::Plain(
                if flat.soft.binary_search(&at).is_ok() {
                    " "
                } else {
                    "\n"
                }
                .to_string(),
            ));
            line = at + 1;
        }
    }
    out.extend(flat.pair(line, flat.text.len()));
    out
}

const SPOILER: &str = "||";
const MARKED: &str = "==";

/// Stands in for a node the scan must not look inside — a link, a code span, anything
/// already styled. It is one byte, is not a delimiter character and is not ASCII whitespace,
/// which is how Telegram treats such a span: content that cannot be split. A real U+0001 in
/// text is treated the same way, so the collision is harmless — nothing maps back through the
/// character, only through recorded offsets. The previous version used U+0000 as a second
/// sentinel on the claim that `pulldown-cmark` cannot emit one; it can, and does, so the soft
/// break is now marked by a node variant instead of by a character.
const OPAQUE: char = '\u{1}';

/// What the builder leaves in a run for the scan to find, in the places where the shape of
/// the run does not carry the information by itself.
///
/// | marker | meaning | what writes it |
/// |---|---|---|
/// | `soft_break_marker` | a line break the scan may not pair across | `Event::SoftBreak` |
/// | `escape_marker` | the *first* character of the next node was not written as itself | an escape, a character reference |
///
/// A third marker stood here, meaning "the whole next node is not prose", and its removal is
/// why this list is short again. Marking part of a run opaque put three meanings into
/// `Flat::seams` — an escape, a character reference, a stretch of non-prose — and the three
/// rules that read it were wired one at a time: a newline inside an HTML tag stopped being a
/// line break, and a spoiler swallowed the paragraph after it. Non-prose is now handled where
/// it cannot interact with anything: a run holding any of it is not scanned at all.
///
/// Both are sequences, which is a variant `from_parts` never builds — it collapses an empty
/// run to `Plain("")` and a single part to that part — and which no text can spell. The
/// previous marker was the character U+0000, chosen on the claim that `pulldown-cmark`
/// replaces it with U+FFFD as CommonMark requires. It does not, so a NUL in a message was
/// read as a line break and came out as a space.
fn soft_break_marker() -> RichText {
    RichText::Seq(Vec::new())
}

/// Stands immediately before a text node whose first character was not written as itself —
/// an escape or a character reference.
fn escape_marker() -> RichText {
    RichText::Seq(vec![RichText::Plain(String::new())])
}

/// A run flattened into bytes, with everything the scan needs to know about where the bytes
/// came from recorded by offset.
struct Flat<'a> {
    text: String,
    /// `(offset of the placeholder, the node it stands for)`, in order.
    opaque: Vec<(usize, &'a RichText)>,
    /// Where a character sits that was not written as itself — an escape (`\|`) or a
    /// character reference (`&#124;`). A delimiter touching one of these is text, because
    /// nobody wrote a delimiter; the builder marks them from the source offsets as it reads
    /// the events. Adjacency in the finished run says nothing about this: a link whose scheme
    /// was rejected also leaves two `Plain` nodes side by side, and `||[a](/d)||` is a
    /// spoiler.
    seams: Vec<usize>,
    /// Offsets of soft line breaks, which are barriers here and spaces on the screen.
    soft: Vec<usize>,
}

/// A matched delimiter pair, as offsets into `Flat::text`.
#[derive(Clone, Copy)]
struct Pair {
    delim: &'static str,
    open: usize,
    close: usize,
}

/// One delimiter's place in a line. Every field only ever moves forward, which is the whole
/// of the linearity argument.
struct Scan {
    /// Where to look for the next opener.
    opener_from: usize,
    /// Where to look for the next closer.
    closer_from: usize,
    /// The first valid closer at or after `closer_from`, once it has been found. Kept across
    /// openers: a closer is valid or not on its own terms, so the search never restarts.
    ///
    /// There is deliberately no "this delimiter is finished" flag beside these. An earlier
    /// version had one and it was the third attempt at making the scan linear; with the
    /// closer search itself moving only forward the flag became unreachable state — no
    /// mutation of it could change an output or a timing — and unreachable state is what the
    /// last four reviews of this file kept finding.
    closer: Option<usize>,
}

impl Scan {
    fn new(from: usize) -> Self {
        Self {
            opener_from: from,
            closer_from: from,
            closer: None,
        }
    }
}

impl<'a> Flat<'a> {
    fn new(parts: &'a [RichText]) -> Self {
        let mut text = String::new();
        let mut opaque = Vec::new();
        let mut seams = Vec::new();
        let mut soft = Vec::new();
        let mut unwritten_next = false;
        for part in parts {
            match part {
                _ if part == &soft_break_marker() => {
                    soft.push(text.len());
                    text.push('\n');
                }
                _ if part == &escape_marker() => unwritten_next = true,
                RichText::Plain(s) => {
                    if std::mem::take(&mut unwritten_next) {
                        seams.push(text.len());
                    }
                    text.push_str(s);
                }
                node => {
                    opaque.push((text.len(), node));
                    text.push(OPAQUE);
                }
            }
        }
        Self {
            text,
            opaque,
            seams,
            soft,
        }
    }

    /// Pair delimiters in `from..to`, which holds no line break.
    fn pair(&self, from: usize, to: usize) -> Vec<RichText> {
        let mut out = Vec::new();
        let mut cursor = from;
        // Both delimiters are tracked at once and the earliest pair wins, because Telegram
        // nests them in either order: `||a ==b== c||` and `==a ||b|| c==` alike.
        let mut scans = [Scan::new(from), Scan::new(from)];
        loop {
            let mut best: Option<Pair> = None;
            for (slot, delim) in [SPOILER, MARKED].into_iter().enumerate() {
                let found = self.next_pair(delim, &mut scans[slot], cursor, to);
                if let Some(pair) = found {
                    if best.is_none_or(|b| pair.open < b.open) {
                        best = Some(pair);
                    }
                }
            }
            let Some(found) = best else { break };
            out.extend(self.slice(cursor, found.open));
            let inner = self.pair(found.open + found.delim.len(), found.close);
            let text = Box::new(RichText::from_parts(inner));
            out.push(RichText::Styled(match found.delim {
                SPOILER => Styled::Spoiler { text },
                _ => Styled::Marked { text },
            }));
            cursor = found.close + found.delim.len();
            // Nothing is reset here on purpose. Neither delimiter may pair across the span
            // just built, and both pointers already say so: `next_pair` holds its opener at
            // the cursor, and a cached closer behind the cursor is behind every later content
            // start too, so `closer_after` drops it on the test it already makes.
        }
        out.extend(self.slice(cursor, to));
        merge_adjacent(out)
    }

    /// The earliest pair of `delim` with its opener at or after `cursor`, or `None` — and
    /// `None` means there is none later in the line either, not just none from here.
    fn next_pair(
        &self,
        delim: &'static str,
        scan: &mut Scan,
        cursor: usize,
        to: usize,
    ) -> Option<Pair> {
        scan.opener_from = scan.opener_from.max(cursor);
        loop {
            let Some(open) = self.occurrence(delim, scan.opener_from, to) else {
                // Nothing of this delimiter left in the line. Parking the pointer at the end
                // is what makes the next call free: a text with a million `|` and no `=` was
                // re-reading the whole tail for the `=` once per pair it found.
                scan.opener_from = to;
                return None;
            };
            // An opener may not be followed by a space, and the end of the line counts as
            // one. Rejecting it is final: nothing about a later cursor changes the character
            // after it, so the pointer moves past it for good.
            let content = open + delim.len();
            let rejected = content >= to || self.is_written_space(content);
            if rejected {
                scan.opener_from = open + 1;
                continue;
            }
            // No closer for this opener means none for any later one either: a later opener's
            // content starts later, so its candidates are a subset of the ones just rejected.
            // The pointer is parked at the end rather than left on this opener, or the next
            // call would walk the tail again to re-find it — which is exactly how the second
            // version of this scan spent 16 s on a megabyte of `||a||` that ended in `==a`.
            let Some(close) = self.closer_after(delim, scan, content, to) else {
                scan.opener_from = to;
                return None;
            };
            // Not `open + 1`: the caller asks both delimiters for a pair and keeps the
            // earlier one, so this pair may lose and be needed again. Stepping past its
            // opener here dropped it for good, and `||секрет|| и ==важно==` — two pairs of
            // different delimiters in one line, which is ordinary prose — lost the second.
            // Leaving the pointer on the opener re-derives the same pair in O(1) if it is
            // asked for again, and the cursor moves it on once the pair is consumed.
            scan.opener_from = open;
            return Some(Pair { delim, open, close });
        }
    }

    /// The first valid closer strictly after `content`, remembered between openers.
    fn closer_after(
        &self,
        delim: &str,
        scan: &mut Scan,
        content: usize,
        to: usize,
    ) -> Option<usize> {
        loop {
            if let Some(at) = scan.closer {
                if at > content {
                    return Some(at);
                }
                // Landing at or before the content start makes the span empty, and it will
                // be at or before every later content start too, so it is gone for good.
                scan.closer = None;
                scan.closer_from = at + 1;
            }
            scan.closer_from = scan.closer_from.max(content);
            // `occurrence` searches inside `to`, so a closer found here always fits.
            let at = self.occurrence(delim, scan.closer_from, to)?;
            scan.closer_from = at + 1;
            // A closer may not be preceded by a space. That test does not mention the
            // opener, which is why one forward pass over the line is enough for all of them.
            if !self.is_written_space(at - 1) {
                scan.closer = Some(at);
            }
        }
    }

    /// Whether the byte at `at` is a space the author typed. A space that arrived as
    /// `&#32;` is not one — Telegram pairs `||a&#32;||` into a spoiler, because the
    /// character that would have broken the pair was never written. The same question the
    /// delimiters are asked, asked about whitespace.
    fn is_written_space(&self, at: usize) -> bool {
        self.text.as_bytes()[at].is_ascii_whitespace() && self.seams.binary_search(&at).is_err()
    }

    /// The next `delim` at or after `from` and inside `to`, skipping one that is written with
    /// an escape — an escaped delimiter is text, not markup, and either half of it counts.
    fn occurrence(&self, delim: &str, from: usize, to: usize) -> Option<usize> {
        let mut from = from;
        loop {
            let offset = self.text.get(from..to)?.find(delim)?;
            let at = from + offset;
            let escaped =
                (at..at + delim.len()).any(|byte| self.seams.binary_search(&byte).is_ok());
            if !escaped {
                return Some(at);
            }
            from = at + 1;
        }
    }

    /// Copy `from..to` of the flat text back into nodes.
    fn slice(&self, from: usize, to: usize) -> Vec<RichText> {
        let mut out = Vec::new();
        let mut at = from;
        let first = self.opaque.partition_point(|(offset, _)| *offset < from);
        for (offset, node) in &self.opaque[first..] {
            if *offset >= to {
                break;
            }
            if *offset > at {
                out.push(RichText::Plain(self.text[at..*offset].to_string()));
            }
            out.push((*node).clone());
            at = offset + OPAQUE.len_utf8();
        }
        if at < to {
            out.push(RichText::Plain(self.text[at..to].to_string()));
        }
        out
    }
}

/// Telegram merges two spans of the same kind that end up next to each other, so
/// `x||a||||b||y` comes back as one spoiler holding `a` and `b` rather than two spoilers.
/// The pieces stay separate inside it, which is what the live parser returns.
fn merge_adjacent(parts: Vec<RichText>) -> Vec<RichText> {
    let mut out: Vec<RichText> = Vec::with_capacity(parts.len());
    for part in parts {
        match (out.last(), part) {
            (Some(RichText::Styled(prev)), RichText::Styled(next)) if mergeable(prev, &next) => {
                let Some(RichText::Styled(prev)) = out.pop() else {
                    unreachable!("just matched a styled span")
                };
                let spoiler = matches!(next, Styled::Spoiler { .. });
                let mut pieces = pieces_of(into_inner(prev));
                pieces.extend(pieces_of(into_inner(next)));
                let text = Box::new(RichText::Seq(pieces));
                out.push(RichText::Styled(if spoiler {
                    Styled::Spoiler { text }
                } else {
                    Styled::Marked { text }
                }));
            }
            (_, part) => out.push(part),
        }
    }
    out
}

/// Only the two kinds this scan builds merge; `**a****b**` is `pulldown-cmark`'s business
/// and arrives already shaped.
fn mergeable(left: &Styled, right: &Styled) -> bool {
    matches!(
        (left, right),
        (Styled::Spoiler { .. }, Styled::Spoiler { .. })
            | (Styled::Marked { .. }, Styled::Marked { .. })
    )
}

fn into_inner(style: Styled) -> RichText {
    match style {
        Styled::Bold { text }
        | Styled::Italic { text }
        | Styled::Strikethrough { text }
        | Styled::Code { text }
        | Styled::Url { text, .. }
        | Styled::Spoiler { text }
        | Styled::Marked { text } => *text,
    }
}

fn pieces_of(text: RichText) -> Vec<RichText> {
    match text {
        RichText::Seq(parts) => parts,
        other => vec![other],
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
    // A delimiter is markup only if it was *written* as one. Two ways of putting the
    // character on the screen without writing it are an escape (`\|`) and a character
    // reference (`&#124;`), and Telegram honours both: `\||секрет||` and
    // `&#124;&#124;секрет&#124;&#124;` are text there. Neither is visible in the event —
    // `pulldown-cmark` hands over a bare `|` in all three cases — so the source range is
    // compared with the text it produced, which separates them without guessing.
    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        let written_literally = match &event {
            Event::Text(text) => {
                markdown.get(range.clone()) == Some(text.as_ref())
                    && !starts_escaped(markdown, range.start)
            }
            _ => true,
        };
        state.push(event, !written_literally);
    }
    state.finish()
}

/// Whether the character at `at` was escaped by the backslash in front of it.
///
/// The run is counted because that is what escaping means: an even one escapes the
/// backslashes and leaves the next character alone. It is redundancy rather than a rule that
/// carries weight — with the reference check beside it, a sweep of 21 864 inputs over
/// backslashes, delimiters and numeric references finds none that tells this apart from
/// looking at a single byte. It stays because the version that dropped it justified the drop
/// by saying no input could tell the difference, a reviewer found one (`\\&#124;|a||`, back
/// when references went unchecked), and one line is a cheap way not to owe that claim again.
fn starts_escaped(markdown: &str, at: usize) -> bool {
    markdown.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
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
    /// One flag per open inline run: true once the run has collected something that is not
    /// prose — raw inline HTML, or an autolink whose text is its own URL. Such a run is not
    /// scanned for delimiters at all.
    ///
    /// Standing the whole run down is deliberately blunt. `<b>||a||</b>` loses a spoiler
    /// Telegram would make, which is a formatting loss and is recorded as a divergence; the
    /// version that tried to be precise about it — marking just the non-prose bytes — hid a
    /// paragraph behind a spoiler instead, because the marks were also read by the whitespace
    /// and line-break rules. A loss of formatting is not in the same class as a loss of text.
    no_scan: Vec<bool>,
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
        self.no_scan.push(false);
    }

    /// Mark the open run as holding something that is not prose, so nothing in it is paired.
    fn stop_scanning(&mut self) {
        if self.inlines.is_empty() {
            self.start_inline();
            self.implicit_run = true;
        }
        if let Some(flag) = self.no_scan.last_mut() {
            *flag = true;
        }
    }

    /// Whether the run being closed may be scanned at all.
    fn scannable(&mut self) -> bool {
        !self.no_scan.pop().unwrap_or(false)
    }

    /// Scan a run, or — when it holds something that is not prose — resolve the markers by
    /// hand and hand it back unpaired.
    ///
    /// Dropping the markers is not enough: a soft break is a marker standing in for the space
    /// it renders as, so filtering it out glued the words either side of a wrapped line
    /// together. `scan_delimiters` puts that space back on the scanned path; this one has to
    /// do the same.
    fn scan_if_prose(parts: Vec<RichText>, scannable: bool) -> Vec<RichText> {
        if scannable {
            return scan_delimiters(parts);
        }
        parts
            .into_iter()
            .filter_map(|part| {
                if part == soft_break_marker() {
                    Some(RichText::Plain(" ".to_string()))
                } else if part == escape_marker() {
                    None
                } else {
                    Some(part)
                }
            })
            .collect()
    }

    /// Close the open inline run, applying the delimiter scans.
    fn take_inline(&mut self) -> RichText {
        let scannable = self.scannable();
        let parts = self.inlines.pop().unwrap_or_default();
        RichText::from_parts(Self::scan_if_prose(parts, scannable))
    }

    /// Close the run without scanning. A fenced block's content is literal — `||` inside a
    /// code sample is two pipes, not a spoiler — and it is the one caller that reaches here
    /// with text the author did not intend as markup.
    fn take_inline_literal(&mut self) -> RichText {
        let _ = self.scannable();
        // No markers can be here: an escape is not processed inside code, so the builder
        // never marks one, and a soft break inside a fence is a newline in the text rather
        // than an event.
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
            self.start_inline();
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
        // Scanned like any other run. A *tight* list item closes here rather than through
        // `take_inline`, and missing that meant `- ||секрет||` kept its pipes while the same
        // text in a loose list became a spoiler — one construct rendering two ways depending
        // on a blank line, in the form agents write most.
        let scannable = self.scannable();
        let parts = self.inlines.pop().unwrap_or_default();
        let text = RichText::from_parts(Self::scan_if_prose(parts, scannable));
        if !text.is_empty() {
            let block = Block::Paragraph { text };
            self.blocks_mut().push(block);
        }
    }

    fn push(&mut self, event: Event<'_>, escaped_start: bool) {
        // Any event other than more block HTML ends the run of it.
        if !matches!(event, Event::Html(_)) {
            self.flush_html_block();
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                // The character arrives decoded and looks like any other, so the fact that it
                // was written as an escape or a reference is recorded here for the scan.
                if escaped_start {
                    self.push_inline(escape_marker());
                }
                self.push_inline(RichText::Plain(text.into_string()));
            }
            Event::Code(text) => self.push_inline(RichText::Styled(Styled::Code {
                text: Box::new(RichText::Plain(text.into_string())),
            })),
            // Not the space it renders as: the scan has to see that a line ended, because
            // Telegram does not pair a delimiter across one. `scan_delimiters` turns it back
            // into a space.
            Event::SoftBreak => self.push_inline(soft_break_marker()),
            Event::HardBreak => self.push_inline(RichText::Plain("\n".into())),
            Event::Rule => self.push_block(Block::Divider),
            // Raw HTML is text, not markup: this is the whole point of `blocks`. A quoted
            // `<tg-button …>` lands in a string, where no parser will look at it.
            Event::InlineHtml(html) => {
                // Kept as text — that is the point of this path — but not as prose: Telegram
                // drops the tag outright, so an attribute holding `||` is nobody's spoiler.
                // The whole run stands down rather than the tag being made opaque inside it;
                // see `no_scan`.
                self.stop_scanning();
                self.push_inline(RichText::Plain(html.into_string()));
            }
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
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                // An autolink's text is its own URL; everything inside it is the address, not
                // prose, so the scan is kept out of it.
                // An autolink's text is its own URL. Half an address disappearing behind a
                // spoiler is worse than a spoiler not appearing, so the run stands down.
                if matches!(link_type, LinkType::Autolink | LinkType::Email) {
                    self.stop_scanning();
                }
                self.open_style(StyleKind::Url(dest_url.into_string()));
            }
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
                // `sendRichMessage` refuses a heading with no text — the whole payload comes
                // back `RICH_MESSAGE_CONTENT_REQUIRED` — and Telegram's own parse of `## `
                // emits nothing for it. A lone `## ` is something a model writes.
                if !text.is_empty() {
                    self.push_block(Block::Heading {
                        text,
                        size: heading_size(level),
                    });
                }
            }
            TagEnd::CodeBlock => {
                let text = self.take_inline_literal();
                let language = self.code_language.take().flatten();
                let text = trim_trailing_newline(text);
                // Same refusal, and the same answer from Telegram's own parse: an empty fence
                // is nothing. A model that opens a block and closes it without writing the
                // sample produced one.
                if !text.is_empty() {
                    self.push_block(Block::Pre { text, language });
                }
            }
            TagEnd::List(_) => {
                if let Some(Frame::ListItems { items, .. }) = self.frames.pop() {
                    // `list must be non-empty` is a separate refusal with its own message.
                    if !items.is_empty() {
                        self.push_block(Block::List { items });
                    }
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
                    // An item that collected nothing is refused with the rest of the payload,
                    // so it carries an empty paragraph instead — which is exactly what
                    // Telegram's own parse of `- a\n- \n- b` puts there. Dropping the item
                    // would renumber an ordered list and lose a line the reader can see.
                    let blocks = if blocks.is_empty() {
                        vec![Block::Paragraph {
                            text: RichText::Plain(String::new()),
                        }]
                    } else {
                        blocks
                    };
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
                    // A quote with nothing in it is refused as well, and `> ` on its own is
                    // how a model starts a quote and then changes its mind.
                    if !blocks.is_empty() {
                        self.push_block(Block::Quote { blocks });
                    }
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
        // The flag belongs to the run, not to the span: a paragraph holding raw HTML stands
        // down everywhere in it, including inside a link that came before the tag.
        let scannable = !self.no_scan.last().copied().unwrap_or(false);
        let Some(run) = self.inlines.last_mut() else {
            return;
        };
        let inner = RichText::from_parts(Self::scan_if_prose(run.split_off(start), scannable));
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
            | Styled::Url { text, .. }
            | Styled::Spoiler { text }
            | Styled::Marked { text } => text,
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
        // The first item holds an empty paragraph rather than nothing — an item with no
        // blocks is refused by `sendRichMessage` along with the whole message.
        assert_eq!(
            json("- ![](x)\n- **bold**"),
            serde_json::json!([{"type": "list", "items": [
                {"blocks": [{"type": "paragraph", "text": ""}]},
                {"blocks": [{"type": "paragraph", "text": {"type": "bold", "text": "bold"}}]},
            ]}])
        );
    }

    /// The battery is a recording, not a table of expectations: every case in
    /// `telegram_oracle.json` was sent to a live bot as `rich_message.markdown` and the parse
    /// `sendRichMessage` echoes back was written down verbatim. This test is the diff.
    ///
    /// Of 196 recorded forms, 146 match byte for byte. The other 50 are named in
    /// `DIVERGENCES`, and for the 30 that are not content-level the difference is *checked*
    /// rather than described: same characters, same multiset of styles, written down
    /// differently. That check is the point. An earlier version of this test listed 23
    /// hand-picked shapes and "22 of 23 match" went into the architecture docs as a property
    /// of the converter; a wider battery then found five classes it had missed, and a later
    /// one found that a "known divergence" pinned in a test was a stepping bug in disguise.
    #[test]
    fn pairing_matches_telegram() {
        let cases = oracle();
        assert!(
            cases.len() >= 130,
            "the battery lost cases: {}",
            cases.len()
        );
        let mut diverged = Vec::new();
        for (source, blocks) in &cases {
            let ours = json(source);
            if ours == *blocks {
                continue;
            }
            diverged.push(source.clone());
            if CONTENT_DIVERGENCES.contains(&source.as_str()) {
                continue;
            }
            assert_eq!(
                visible(&ours),
                visible(blocks),
                "{source:?} обещано расхождением только в записи, а текст другой"
            );
            let (mut ours_styles, mut their_styles) = (style_kinds(&ours), style_kinds(blocks));
            ours_styles.sort();
            their_styles.sort();
            assert_eq!(
                ours_styles, their_styles,
                "{source:?} обещано расхождением только в записи, а стили другие"
            );
        }
        let mut expected: Vec<String> = DIVERGENCES.iter().map(|(s, _)| (*s).into()).collect();
        expected.sort();
        diverged.sort();
        assert_eq!(diverged, expected, "diff against the live parser moved");
    }

    /// Every character a reader would see, in order.
    fn visible(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(parts) => parts.iter().map(visible).collect(),
            serde_json::Value::Object(fields) => fields
                .iter()
                .filter(|(key, _)| matches!(key.as_str(), "text" | "blocks" | "items" | "cells"))
                .map(|(_, value)| visible(value))
                .collect(),
            _ => String::new(),
        }
    }

    /// Every styled span in the tree, by kind.
    fn style_kinds(value: &serde_json::Value) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Array(parts) => parts.iter().for_each(|p| walk(p, out)),
                serde_json::Value::Object(fields) => {
                    if let Some(serde_json::Value::String(kind)) = fields.get("type") {
                        if matches!(
                            kind.as_str(),
                            "bold"
                                | "italic"
                                | "strikethrough"
                                | "code"
                                | "url"
                                | "spoiler"
                                | "marked"
                                | "mention"
                                | "hashtag"
                                | "bot_command"
                        ) {
                            out.push(kind.clone());
                        }
                    }
                    for (key, value) in fields {
                        if matches!(key.as_str(), "text" | "blocks" | "items" | "cells") {
                            walk(value, out);
                        }
                    }
                }
                _ => {}
            }
        }
        walk(value, &mut out);
        out
    }

    /// The recorded parses, as `(source, blocks)`.
    fn oracle() -> Vec<(String, serde_json::Value)> {
        let raw = include_str!("telegram_oracle.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(raw).expect("oracle parses");
        cases
            .into_iter()
            .map(|case| {
                (
                    case["source"].as_str().expect("source").to_string(),
                    case["blocks"].clone(),
                )
            })
            .collect()
    }

    /// A delimiter nobody wrote is text, and there are two ways to put the character on the
    /// screen without writing it: an escape (`\|`) and a character reference (`&#124;`).
    /// Telegram honours both, and `pulldown-cmark` hands over a decoded `|` for all three
    /// spellings, so the builder tells them apart by comparing each text event with the
    /// source range it came from.
    ///
    /// The rule before it — "two text events in a row mean an escape" — was false in both
    /// directions and, by accident, covered references: a reference always makes its own
    /// event. Replacing it with a rule about escapes alone therefore *removed* protection
    /// nobody had noticed was there, and `&#124;&#124;значение&#124;&#124;` became a spoiler
    /// hiding the text. Both classes are in the recording now.
    #[test]
    fn an_escaped_delimiter_is_text() {
        for source in [
            r"\|\|a\|\|",
            r"\==a\==",
            r"\|\|a||",
            r"||a\|\|",
            // An escape at the very start of a run: `pulldown-cmark` emits one text event for
            // the whole thing, so a rule that read escapes off the event stream saw nothing
            // here and made a spoiler where Telegram shows text.
            r"\||секрет||",
            r"\==важно==",
            // Character references, the class the escape-only rule dropped.
            "&#124;&#124;секрет&#124;&#124;",
            "&#61;&#61;важно&#61;&#61;",
            "таблица: &#124;&#124;значение&#124;&#124; конец",
        ] {
            let rendered = json(source);
            assert!(
                style_kinds(&rendered).is_empty(),
                "{source:?} дал разметку: {rendered}"
            );
        }
        // An even run of backslashes escapes the backslashes, not what follows them.
        assert_eq!(
            json(r"\\||a||"),
            serde_json::json!([{"type": "paragraph", "text": [
                "\\", {"type": "spoiler", "text": "a"},
            ]}])
        );
        // Everything else that splits a text event — an entity, an unresolved `[`, a `*`, a
        // stray backtick, a styled span before it — is not an escape, and the pair around it
        // is real. Reading escapes off the event stream broke every one of these.
        for (source, text) in [
            ("a&amp;||секрет||", "секрет"),
            ("||a&amp;||", "a&"),
            ("||a[^1]||", "a[^1]"),
            ("||a*b||", "a*b"),
            ("||a`b||", "a`b"),
        ] {
            let rendered = json(source).to_string();
            assert!(
                rendered.contains(&format!(r#"{{"text":"{text}","type":"spoiler"}}"#)),
                "{source:?} потерял пару: {rendered}"
            );
        }
        assert_eq!(
            json("`c`||a||"),
            serde_json::json!([{"type": "paragraph", "text": [
                {"type": "code", "text": "c"},
                {"type": "spoiler", "text": "a"},
            ]}])
        );
    }

    /// Two pairs of *different* delimiters in one line — ordinary prose, and broken for a
    /// day by the fix that made the scan linear. Both delimiters are asked for a pair and the
    /// earlier one wins; the loser's opener pointer had already stepped past its own opener,
    /// so the second pair was gone for good.
    ///
    /// The differential test missed it because the shortest witness is ten characters
    /// (`||a||==a==`) and it stopped at eight; the recorded battery missed it because all
    /// thirteen of its two-delimiter forms were nested or interleaved, never sequential. Both
    /// have been widened, and these are here by name as well.
    #[test]
    fn two_pairs_of_different_delimiters_both_survive() {
        let spoiler = |t: &str| serde_json::json!({"type": "spoiler", "text": t});
        let marked = |t: &str| serde_json::json!({"type": "marked", "text": t});
        for (source, expected) in [
            (
                "||секрет|| и ==важно==",
                serde_json::json!([spoiler("секрет"), " и ", marked("важно")]),
            ),
            ("||a||==a==", serde_json::json!([spoiler("a"), marked("a")])),
            (
                "==a== ||b||",
                serde_json::json!([marked("a"), " ", spoiler("b")]),
            ),
            (
                "||a|| ==b== ||c|| ==d==",
                serde_json::json!([
                    spoiler("a"),
                    " ",
                    marked("b"),
                    " ",
                    spoiler("c"),
                    " ",
                    marked("d"),
                ]),
            ),
        ] {
            assert_eq!(
                json(source),
                serde_json::json!([{"type": "paragraph", "text": expected}]),
                "{source:?}"
            );
        }
    }

    /// A run holding something that is not prose is not scanned at all.
    ///
    /// Three ways a delimiter can appear without anybody writing one, all of which the scan
    /// used to read as markup: inside an HTML attribute, inside an autolink whose text is its
    /// own URL, and — separately — a space or newline that arrived as `&#32;` / `&#10;`.
    ///
    /// The first two are handled by standing the whole run down, which is blunter than
    /// Telegram: it pairs the authored delimiters and drops the tag. That costs a spoiler in
    /// `<b>||a||</b>` and is recorded as a divergence. The version that tried to be precise —
    /// marking only the non-prose bytes — put a third meaning into the same `seams` vector
    /// that the whitespace and line-break rules read, and a newline inside `<b\nid=x>` stopped
    /// being a line break: `||a <b\nid=x> c||` hid the paragraph behind a spoiler while
    /// Telegram left it literal. Losing formatting and losing text are not the same failure.
    #[test]
    fn a_run_that_is_not_all_prose_is_not_scanned() {
        for source in [
            r#"Тег <input value="a||b"> и ||секрет||"#,
            "||a <b\nid=x> c||",
            "<a||b@ya.ru> и ||секрет||",
            r#"[<input value="a||b">](/rel) и ||секрет||"#,
            "<b>||a||</b>",
        ] {
            let rendered = json(source).to_string();
            assert!(
                !rendered.contains(r#""type":"spoiler""#),
                "{source:?} спарил в непрозаическом прогоне: {rendered}"
            );
            assert!(
                rendered.contains("||"),
                "{source:?} потерял символы: {rendered}"
            );
        }
        // The markers are the scan's private notation. A run that stands down still has to
        // resolve them: a soft break is the space a wrapped line renders as, and filtering it
        // out glued the words either side of it together.
        assert_eq!(
            json("<b>x</b> и\nпродолжение"),
            serde_json::json!([{"type": "paragraph", "text": "<b>x</b> и продолжение"}])
        );
        for source in [
            r"<b>x</b> и \|\|a\|\|",
            "<b>x</b> и\nпродолжение",
            r"<https://ya.ru> и \|a",
        ] {
            let rendered = json(source).to_string();
            assert!(
                !rendered.contains("[]") && !rendered.contains(r#"[""]"#),
                "{source:?} протёк маркером: {rendered}"
            );
        }
        // The autolink keeps its address whole, and the prose after it is a separate run.
        assert_eq!(
            json("<https://ya.ru/||a||>"),
            serde_json::json!([{"type": "paragraph", "text": {
                "type": "url",
                "url": "https://ya.ru/||a||",
                "text": "https://ya.ru/||a||",
            }}])
        );
        // A character reference is a different question and still answered precisely: the
        // space was never written, so it does not break the pair.
        assert_eq!(
            json("||a&#32;||"),
            serde_json::json!([{"type": "paragraph", "text": {"type": "spoiler", "text": "a "}}])
        );
        assert_eq!(
            json("||&#32;a||"),
            serde_json::json!([{"type": "paragraph", "text": {"type": "spoiler", "text": " a"}}])
        );
        assert_eq!(
            json("||a&#10;b||"),
            serde_json::json!([{"type": "paragraph", "text": {"type": "spoiler", "text": "a\nb"}}])
        );
    }

    /// A tight list item closes its run through `flush_implicit_run`, not `take_inline`.
    /// The first version scanned only the latter, so `- ||секрет||` kept its pipes while the
    /// same text in a *loose* list became a spoiler — one construct rendering two ways
    /// depending on a blank line, in the form agents write most. Telegram makes a spoiler in
    /// both, as do these.
    #[test]
    fn pairs_work_in_every_container() {
        for source in [
            "- ||секрет||",
            "- [x] ||секрет||",
            "1. ||секрет||",
            "> ||секрет||",
            "# ||секрет||",
            "- ||секрет||\n\n- второй",
            "| a | b |\n| --- | --- |\n| ==c== | d |",
        ] {
            let rendered = json(source).to_string();
            assert!(
                rendered.contains(r#""type":"spoiler""#) || rendered.contains(r#""type":"marked""#),
                "{source:?} не дал пару: {rendered}"
            );
        }
    }

    /// A pair straddling a node boundary used to pass the "content is not empty" check,
    /// because the end of one node and the start of the next are the same point written as
    /// two different `(node, offset)` tuples. The result was an empty span with the text
    /// between the delimiters *gone* — the sixth content-loss defect in this module, and the
    /// reason the scan now works in one coordinate system.
    ///
    /// A link with a relative destination is what splits the run in practice: its scheme is
    /// rejected, the text stays, and the run gains a boundary exactly where the delimiters
    /// meet. Agents write relative links constantly.
    #[test]
    fn a_pair_across_a_node_boundary_keeps_its_text() {
        assert_eq!(
            json("a==[==](/d)b"),
            serde_json::json!([{"type": "paragraph", "text": "a====b"}])
        );
        assert_eq!(
            json("||[a](/d)||"),
            serde_json::json!([{"type": "paragraph", "text": {"type": "spoiler", "text": "a"}}])
        );
        // The same shape with a *kept* link: the node is opaque, so it is content — the pair
        // is real and the link survives inside it.
        assert_eq!(
            json("==[a](https://ya.ru)=="),
            serde_json::json!([{"type": "paragraph", "text": {
                "type": "marked",
                "text": {"type": "url", "url": "https://ya.ru", "text": "a"},
            }}])
        );
    }

    /// Code is never scanned, pinned as a test rather than as a comment: a `||` inside a code
    /// span turning into a spoiler would not be a formatting bug but a content one, silently
    /// eating the literal text someone was quoting. Raised as a question on the pull request,
    /// and a question about an invariant is a missing test.
    ///
    /// Two separate mechanisms carry it, so both are checked: an inline span is already a
    /// `Styled::Code` node before the run is assembled, which makes it opaque to the scan, and
    /// a fenced or indented block closes its run through `take_inline_literal`, which does not
    /// scan at all.
    #[test]
    fn code_is_never_scanned() {
        for (source, code) in [
            ("`a || b`", "a || b"),
            ("`||в коде||`", "||в коде||"),
            ("`==x==`", "==x=="),
        ] {
            assert_eq!(
                json(source),
                serde_json::json!([{"type": "paragraph", "text": {"type": "code", "text": code}}]),
                "{source:?}"
            );
        }
        // The span is opaque *content*: the pair around it is real, the code inside is
        // untouched, and the boundary it creates in the run is the one that used to produce
        // empty spans.
        assert_eq!(
            json("||секрет `код` конец||"),
            serde_json::json!([{"type": "paragraph", "text": {"type": "spoiler", "text": [
                "секрет ", {"type": "code", "text": "код"}, " конец",
            ]}}])
        );
        assert_eq!(
            json("```\n==x==\n||y||\n```"),
            serde_json::json!([{"type": "pre", "text": "==x==\n||y||"}])
        );
        assert_eq!(
            json("    ||a||"),
            serde_json::json!([{"type": "pre", "text": "||a||"}])
        );
        // An *indented* block arrives one text event per line, so the builder marks every
        // line boundary as an escape — there is no way to tell the two apart at that point.
        // Nothing consumes those markers on this path, so it strips them itself; a marker in
        // a payload is a `Seq` where Telegram expects a string.
        assert_eq!(
            json("    один\n    два"),
            serde_json::json!([{"type": "pre", "text": "один\nдва"}])
        );
    }

    /// A soft break reaches the scan as a marker rather than as the space it renders as,
    /// because Telegram does not pair a delimiter across a line break. The marker is a node
    /// variant no text can spell: the previous one was U+0000, chosen on the claim that
    /// `pulldown-cmark` cannot emit that character — it can, so a NUL in a message would have
    /// been read as a line break and rewritten as a space.
    #[test]
    fn a_soft_break_renders_as_a_space_and_never_leaks() {
        assert_eq!(
            json("a\nb"),
            serde_json::json!([{"type": "paragraph", "text": "a b"}])
        );
        assert_eq!(
            json("a\u{0}b"),
            serde_json::json!([{"type": "paragraph", "text": "a\u{0}b"}])
        );
        for source in [
            "a\nb",
            "||a\nb||",
            "- a\nb",
            "- ||a\nb||",
            "> a\nb",
            "# a\nb",
            "| a\nb | c |\n| --- | --- |\n| d | e |",
            "**a\nb**",
            "[a\nb](https://ya.ru)",
            "a\n\nb",
            "a  \nb",
        ] {
            let rendered = json(source).to_string();
            assert!(
                !rendered.contains('\u{1}'),
                "{source:?} протёк маркером: {rendered}"
            );
        }
    }

    /// The optimised scan against an obviously-correct one: same rules, no cache, no
    /// exhaustion flags, every opener tried against every later closer. Exhaustive over the
    /// delimiters, a letter and a space up to eight characters — 87 380 runs — and again with
    /// every one-cut split into two text nodes, which is how an escape reaches the scan.
    ///
    /// This is the test the module was missing. Three versions in a row were quadratic, and
    /// each fix was a new piece of state carrying a claim about what could be skipped: "no
    /// occurrence left", "no pair left", "the closer is still ahead". A differential against
    /// brute force checks the claims instead of restating them.
    #[test]
    fn pairing_matches_a_naive_reference() {
        let alphabet = ['|', '=', 'a', ' '];
        let mut inputs = vec![String::new()];
        let mut all = Vec::new();
        for _ in 0..9 {
            inputs = inputs
                .iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |c| {
                        let mut next = prefix.clone();
                        next.push(*c);
                        next
                    })
                })
                .collect();
            all.extend(inputs.iter().cloned());
        }
        for input in &all {
            let parts = vec![RichText::Plain(input.clone())];
            let flat = Flat::new(&parts);
            let end = flat.text.len();
            assert_eq!(
                RichText::from_parts(flat.pair(0, end)),
                RichText::from_parts(naive_pair(&flat, 0, end)),
                "быстрый скан разошёлся с эталоном на {input:?}"
            );
        }
        // Two pairs of different delimiters need ten characters — `||a||==a==` is the
        // shortest — and the version of this test that stopped at eight passed while that
        // exact shape was broken. The alphabet drops the space here to keep the count sane;
        // the run above covers spaces at shorter lengths.
        let mut inputs = vec![String::new()];
        let mut long = Vec::new();
        for _ in 0..10 {
            inputs = inputs
                .iter()
                .flat_map(|prefix| {
                    ['|', '=', 'a'].iter().map(move |c| {
                        let mut next = prefix.clone();
                        next.push(*c);
                        next
                    })
                })
                .collect();
            long.extend(inputs.iter().cloned());
        }
        for input in &long {
            let parts = vec![RichText::Plain(input.clone())];
            let flat = Flat::new(&parts);
            let end = flat.text.len();
            assert_eq!(
                RichText::from_parts(flat.pair(0, end)),
                RichText::from_parts(naive_pair(&flat, 0, end)),
                "быстрый скан разошёлся с эталоном на {input:?}"
            );
        }
        // Splitting a run into two `Plain` nodes proves nothing: `Flat` concatenates them and
        // comes out byte-identical, so the loop that did that compared a thing with itself.
        // A seam is what an escape or a character reference leaves behind, and it is put here
        // the same way the builder puts it — with a marker between the parts.
        for input in all.iter().filter(|s| s.len() <= 6) {
            for cut in 1..input.len() {
                let parts = vec![
                    RichText::Plain(input[..cut].to_string()),
                    escape_marker(),
                    RichText::Plain(input[cut..].to_string()),
                ];
                let flat = Flat::new(&parts);
                let end = flat.text.len();
                assert!(
                    !flat.seams.is_empty(),
                    "шов не записан, тест сравнивает сам с собой"
                );
                assert_eq!(
                    RichText::from_parts(flat.pair(0, end)),
                    RichText::from_parts(naive_pair(&flat, 0, end)),
                    "быстрый скан разошёлся с эталоном на {input:?} со швом в {cut}"
                );
            }
        }
    }

    /// The rules, written without a single piece of state that remembers anything.
    fn naive_pair(flat: &Flat, from: usize, to: usize) -> Vec<RichText> {
        let bytes = flat.text.as_bytes();
        let is = |at: usize, delim: &str| {
            at + delim.len() <= to
                && bytes[at..at + delim.len()] == *delim.as_bytes()
                && !(at..at + delim.len()).any(|b| flat.seams.binary_search(&b).is_ok())
        };
        let mut out = Vec::new();
        let mut cursor = from;
        loop {
            let mut best: Option<Pair> = None;
            for delim in [SPOILER, MARKED] {
                for open in cursor..to {
                    if !is(open, delim) {
                        continue;
                    }
                    let content = open + delim.len();
                    let space = |at: usize| {
                        bytes[at].is_ascii_whitespace() && flat.seams.binary_search(&at).is_err()
                    };
                    if content >= to || space(content) {
                        continue;
                    }
                    let close = (content + 1..to).find(|&at| is(at, delim) && !space(at - 1));
                    if let Some(close) = close {
                        if best.is_none_or(|b| open < b.open) {
                            best = Some(Pair { delim, open, close });
                        }
                    }
                    break;
                }
            }
            let Some(found) = best else { break };
            out.extend(flat.slice(cursor, found.open));
            let inner = naive_pair(flat, found.open + found.delim.len(), found.close);
            let text = Box::new(RichText::from_parts(inner));
            out.push(RichText::Styled(match found.delim {
                SPOILER => Styled::Spoiler { text },
                _ => Styled::Marked { text },
            }));
            cursor = found.close + found.delim.len();
        }
        out.extend(flat.slice(cursor, to));
        merge_adjacent(out)
    }

    /// Exhaustive over every string of up to six characters drawn from the delimiters, two
    /// letters and a space — 19 530 inputs — checking that nothing *except* the delimiters is
    /// lost or moved, and that no span comes out empty.
    ///
    /// The delimiters themselves are excluded from the comparison because a paired one is
    /// consumed by design, so what happens to them is checked by
    /// `pairing_matches_a_naive_reference` instead. An earlier version of this docblock — and
    /// the commit message under it — claimed this compared every character, which it never
    /// did; the letters-only version before that hid a lost space as well.
    #[test]
    fn no_input_loses_or_reorders_its_text() {
        let alphabet = ['|', '=', 'a', 'b', ' '];
        let mut inputs = vec![String::new()];
        let mut all = Vec::new();
        for _ in 0..6 {
            inputs = inputs
                .iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |c| {
                        let mut next = prefix.clone();
                        next.push(*c);
                        next
                    })
                })
                .collect();
            all.extend(inputs.iter().cloned());
        }
        for input in &all {
            let rendered = json(input);
            let kept: String = visible(&rendered)
                .chars()
                .filter(|c| *c != '|' && *c != '=')
                .collect();
            let expected: String = input.chars().filter(|c| *c != '|' && *c != '=').collect();
            assert_eq!(
                kept.trim(),
                expected.trim(),
                "текст изменился на входе {input:?}: {rendered}"
            );
            assert!(
                !rendered.to_string().contains(r#""text":"""#),
                "пустой спан на входе {input:?}: {rendered}"
            );
        }
    }

    /// Scanning is linear in the length of the run. Three versions in a row were not, and each
    /// time the test missed it by a handful of characters:
    ///
    /// * v1 re-scanned from the beginning — 12 s on a megabyte;
    /// * v2 gave up on a delimiter only when the tail held no occurrence of it, so three
    ///   characters of `==` after a megabyte of `||a||` cost 16.8 s;
    /// * v3 cached a *pair*, and the cache died whenever the other delimiter consumed its
    ///   opener, so `"==a ||b== ".repeat(n) + "x||"` — the previous test's own input plus
    ///   three characters — cost 56 s.
    ///
    /// The last two entries are that third input in both orientations. The budget matches
    /// `rich_sanitize`'s equivalent test: a megabyte under two seconds.
    #[test]
    fn delimiter_scanning_is_linear() {
        for input in [
            "||a||".repeat(200_000),
            "|".repeat(1_000_000),
            "=".repeat(1_000_000),
            "||аб|| ".repeat(100_000),
            "|| a|| ".repeat(100_000),
            format!("{}==a", "||a|| ".repeat(200_000)),
            format!("{}||a", "==a== ".repeat(200_000)),
            // The same shapes with the tail delimiter *paired*. Without the closer the pair
            // is never returned, so nothing exercises the pointer that holds the opener in
            // place — and two mutations of it survived the whole suite while costing 26 s.
            format!("{}==a==", "||a|| ".repeat(200_000)),
            format!("{}||a||", "==a== ".repeat(200_000)),
            format!("||a{}", " ||".repeat(300_000)),
            format!("==a{}", " ==".repeat(300_000)),
            "||a ==b|| ".repeat(100_000),
            "a\n||b|| ".repeat(100_000),
            format!("{}x==", "||a ==b|| ".repeat(100_000)),
            format!("{}x||", "==a ||b== ".repeat(100_000)),
            // Many valid openers, no valid closer anywhere: without giving up on a delimiter
            // after the first failed search, this is one full scan of the tail per opener.
            "||a ".repeat(300_000),
            "==a ".repeat(300_000),
        ] {
            let start = std::time::Instant::now();
            let _ = markdown_to_blocks(&input);
            assert!(
                start.elapsed() < std::time::Duration::from_secs(2),
                "{:?} на входе {} байт",
                start.elapsed(),
                input.len()
            );
        }
    }

    /// The forms we knowingly do not reproduce, each with the reason. Every one is asserted
    /// to *still* differ, so closing one of these gaps fails the diff rather than passing
    /// unnoticed, and every one outside `CONTENT_DIVERGENCES` is additionally checked to
    /// carry Telegram's own text and Telegram's own set of styles.
    const DIVERGENCES: &[(&str, &str)] = &[
        ("&#32;||a||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("&vert;&vert;a&vert;&vert;", "Telegram decodes numeric character references and leaves named ones alone, so it shows `&vert;` as written; CommonMark decodes both. Neither side makes markup of it â the difference is the text, and it is `pulldown-cmark`'s, not the scan's"),
        ("<b>||a||</b>", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("<https://ya.ru/||a||> и ||секрет||", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("<mailto:a||b@ya.ru>", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("<span data=\"||\">||секрет||</span>", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("a==[==](/d)b", "a relative link: Telegram keeps the raw text and finds a bot command in `/d`, while this converter drops the link and keeps its text (the policy on `close_style`). Before the scan was rewritten this input produced an empty `marked` span with the link text destroyed, so it is the regression case too"),
        ("| a | b |\n| --- | --- |\n| ||c|| | d |", "GFM reads the pipes of `||c||` as cell separators, and a row with more cells than the header is truncated per the spec; Telegram is not a GFM parser and keeps the text"),
        ("||\tсекрет||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("||#хэштег||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("||<b>a</b>||", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("||@durov||", "Telegram detects a mention inside the spoiler; this converter detects no entities at all"),
        ("||a&#10;b||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("||a&#9;b||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("||a|| <br> ||b||", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("||https://ya.ru||", "same, for a bare URL"),
        ("||до|| <https://ya.ru/b> ||после||", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("||секрет\t||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("||секрет\\||b||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("Тег <input value=\"a||b\"> и ||секрет||", "a run holding raw HTML or an autolink is not scanned at all, so the pairs elsewhere in it are lost. Telegram makes them; the alternative — being precise about which characters are not prose — is what hid a paragraph behind a spoiler, and losing a spoiler is not in the same class as losing text"),
        ("**==a==**", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("**==||a||==**", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("**==важно==**", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("**[a](https://ya.ru)**", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("**`c`**", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("**||a||**", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("*==a==*", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("*`c`*", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("*||a||*", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("- ||a\n- b||", "Telegram labels list items with a bullet; `Block::List` carries no label"),
        ("- ||a|| и ==b==", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("==**a**==", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("==`c`==", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("==||a||==", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("[**a**](https://ya.ru)", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("[==a==](https://ya.ru)", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("[`c`](https://ya.ru)", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("[||a||](https://ya.ru)", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("[спойлер ||тут||](https://ya.ru)", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("[ссылка](https://ya.ru) и ||секрет||", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("a==[b](https://ya.ru)==c", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("| a | b |\n| --- | --- |\n| ==c== | d |", "Telegram fills in table chrome this converter leaves out (alignment, borders, stripes)"),
        ("| a | b |\n| --- | --- |\n| c\\|d | e |", "Telegram fills in table chrome this converter leaves out (alignment, borders, stripes)"),
        ("||==`c`==||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("||[a](https://ya.ru)||", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("||[ссылка](https://ya.ru)||", "Telegram normalises the host with a trailing slash, and orders equal-range styles its own way"),
        ("||`c`||", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("~~`c`~~", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("~~||a||~~", "equal-range styles, serialised in Telegram's order rather than the source's"),
        ("~~||s||~~", "equal-range styles, serialised in Telegram's order rather than the source's"),
    ];

    /// The divergences that reach the text or the styles themselves rather than how they are
    /// written down. Everything not in here is checked to carry the same characters and the
    /// same set of styles as Telegram's own parse.
    const CONTENT_DIVERGENCES: &[&str] = &[
        "&#32;||a||",
        "&vert;&vert;a&vert;&vert;",
        "<b>||a||</b>",
        "<https://ya.ru/||a||> и ||секрет||",
        "<mailto:a||b@ya.ru>",
        "<span data=\"||\">||секрет||</span>",
        "a==[==](/d)b",
        "| a | b |\n| --- | --- |\n| ||c|| | d |",
        "||\tсекрет||",
        "||#хэштег||",
        "||<b>a</b>||",
        "||@durov||",
        "||a&#10;b||",
        "||a&#9;b||",
        "||a|| <br> ||b||",
        "||https://ya.ru||",
        "||до|| <https://ya.ru/b> ||после||",
        "||секрет\t||",
        "||секрет\\||b||",
        "Тег <input value=\"a||b\"> и ||секрет||",
    ];

    /// Every block this converter emits has to be something `sendRichMessage` accepts, and
    /// four shapes were not: a heading with no text, an empty fenced block, an empty
    /// blockquote and a list item that collected nothing. Each makes the API refuse the
    /// *whole* message with `RICH_MESSAGE_CONTENT_REQUIRED`, after which `send_text` falls
    /// back to the legacy Markdown path — so the message arrives, the rich path is silently
    /// skipped, and the log says the converter failed when it had not.
    ///
    /// All four sources below are ordinary model output. The expectations are Telegram's own
    /// parse of the same Markdown: it drops the heading, the fence and the quote, and puts an
    /// empty paragraph inside the empty item. Dropping the item instead would renumber an
    /// ordered list and lose a line the reader can see.
    ///
    /// This was missed for three weeks because every check on this converter compared *our
    /// parse with Telegram's parse of the same source*. That question never asks whether our
    /// output is valid input.
    #[test]
    fn every_block_is_one_telegram_accepts() {
        assert_eq!(
            json("- a\n- \n- b"),
            serde_json::json!([{"type": "list", "items": [
                {"blocks": [{"type": "paragraph", "text": "a"}]},
                {"blocks": [{"type": "paragraph", "text": ""}]},
                {"blocks": [{"type": "paragraph", "text": "b"}]},
            ]}])
        );
        assert_eq!(
            json("## \n\nтекст"),
            serde_json::json!([{"type": "paragraph", "text": "текст"}])
        );
        assert_eq!(
            json("текст\n\n```\n```\n\nещё"),
            serde_json::json!([
                {"type": "paragraph", "text": "текст"},
                {"type": "paragraph", "text": "ещё"},
            ])
        );
        assert_eq!(
            json("> \n\nтекст"),
            serde_json::json!([{"type": "paragraph", "text": "текст"}])
        );
        // The rule, rather than the four examples: nothing empty reaches a payload.
        for source in [
            "- a\n- \n- b",
            "## ",
            "#\n\nтекст",
            "```\n```",
            "> ",
            "- ![](x)",
            "1. a\n2. \n3. b",
            "- [ ] \n- [x] сделано",
        ] {
            let rendered = json(source).to_string();
            assert!(
                !rendered.contains(r#""blocks":[]"#) && !rendered.contains(r#""items":[]"#),
                "{source:?} отдал пустой блок: {rendered}"
            );
            for empty in [
                r#"{"text":"","type":"heading""#,
                r#"{"text":"","type":"pre""#,
            ] {
                assert!(
                    !rendered.contains(empty),
                    "{source:?} отдал пустой блок: {rendered}"
                );
            }
        }
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
