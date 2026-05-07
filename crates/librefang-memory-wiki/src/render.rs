use serde::{Deserialize, Serialize};

/// Markdown render flavor for vault pages.
///
/// The two modes only differ in how cross-references between pages are
/// emitted. Frontmatter and prose body are identical.
///
/// * `Native` — plain Markdown links: `[topic](topic.md)`.
/// * `Obsidian` — Obsidian / Logseq wiki-link syntax: `[[topic]]`.
///
/// Both are valid CommonMark in their respective ecosystems and the body is
/// otherwise unchanged, so a vault can be re-rendered in the other mode
/// without losing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
    #[default]
    Native,
    Obsidian,
}

impl From<librefang_types::config::MemoryWikiRenderMode> for RenderMode {
    fn from(m: librefang_types::config::MemoryWikiRenderMode) -> Self {
        match m {
            librefang_types::config::MemoryWikiRenderMode::Native => Self::Native,
            librefang_types::config::MemoryWikiRenderMode::Obsidian => Self::Obsidian,
        }
    }
}

impl RenderMode {
    /// Render a single cross-reference to `topic` in the active flavor.
    pub fn link(self, topic: &str) -> String {
        match self {
            RenderMode::Native => format!("[{topic}]({topic}.md)"),
            RenderMode::Obsidian => format!("[[{topic}]]"),
        }
    }

    /// Substitute every `[[link]]` placeholder in `body` with the active
    /// flavor. The placeholder is the canonical authoring form so a body
    /// authored once is portable across render modes — `wiki_write`
    /// callers always pass `[[topic]]` and the vault rewrites at flush
    /// time.
    pub fn rewrite_links(self, body: &str) -> String {
        let mut out = String::with_capacity(body.len());
        let mut rest = body;
        while let Some(open) = rest.find("[[") {
            out.push_str(&rest[..open]);
            let after_open = &rest[open + 2..];
            if let Some(close) = after_open.find("]]") {
                let topic = &after_open[..close];
                if topic.is_empty() || topic.contains('\n') {
                    out.push_str("[[");
                    rest = after_open;
                    continue;
                }
                out.push_str(&self.link(topic));
                rest = &after_open[close + 2..];
            } else {
                out.push_str("[[");
                rest = after_open;
            }
        }
        out.push_str(rest);
        out
    }

    /// Extract every wiki link reference in body order. Recognises both the
    /// canonical authoring form `[[topic]]` and the rewritten native form
    /// `[topic](topic.md)`, so the backlinks index is invariant under
    /// render-mode flips and works against pages on disk regardless of
    /// which mode wrote them.
    pub fn extract_links(body: &str) -> Vec<String> {
        let mut out = Vec::new();

        // (1) [[topic]] — obsidian / authoring placeholder.
        let mut rest = body;
        while let Some(open) = rest.find("[[") {
            let after_open = &rest[open + 2..];
            if let Some(close) = after_open.find("]]") {
                let topic = &after_open[..close];
                if !topic.is_empty() && !topic.contains('\n') {
                    out.push(topic.to_string());
                }
                rest = &after_open[close + 2..];
            } else {
                break;
            }
        }

        // (2) [text](target.md) — native render output. We accept it as a
        // backlink only when the visible text equals the target stem
        // (`[foo](foo.md)`), the form `WikiVault` always emits. That
        // narrows the false-positive risk from arbitrary `.md` links a
        // human might paste in (e.g. `[see this](docs/intro.md)`).
        let mut rest = body;
        while let Some(close_text) = rest.find("](") {
            let before = &rest[..close_text];
            let after_open = &rest[close_text + 2..];
            let close_paren = match after_open.find(')') {
                Some(c) => c,
                None => break,
            };
            let target = &after_open[..close_paren];
            let advance = close_text + 2 + close_paren + 1;
            let topic_target = target.strip_suffix(".md");
            if let Some(open_text) = before.rfind('[') {
                let text = &before[open_text + 1..];
                if let Some(topic) = topic_target {
                    if !topic.is_empty()
                        && !topic.contains('/')
                        && !topic.contains('\n')
                        && text == topic
                    {
                        out.push(topic.to_string());
                    }
                }
            }
            rest = &rest[advance..];
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_link_matches_filename() {
        assert_eq!(RenderMode::Native.link("foo"), "[foo](foo.md)");
    }

    #[test]
    fn obsidian_link_uses_wiki_syntax() {
        assert_eq!(RenderMode::Obsidian.link("foo"), "[[foo]]");
    }

    #[test]
    fn rewrite_links_native() {
        let body = "see [[foo]] and also [[bar]] for context";
        assert_eq!(
            RenderMode::Native.rewrite_links(body),
            "see [foo](foo.md) and also [bar](bar.md) for context"
        );
    }

    #[test]
    fn rewrite_links_obsidian_is_identity() {
        let body = "see [[foo]] and also [[bar]]";
        assert_eq!(RenderMode::Obsidian.rewrite_links(body), body);
    }

    #[test]
    fn extract_links_finds_all_obsidian_form() {
        let body = "see [[foo]], [[bar]], and [[foo]] again";
        assert_eq!(
            RenderMode::extract_links(body),
            vec!["foo".to_string(), "bar".to_string(), "foo".to_string()]
        );
    }

    #[test]
    fn extract_links_recognises_native_form() {
        let body = "see [foo](foo.md) and [bar](bar.md) plus [unrelated](docs/x.md)";
        assert_eq!(
            RenderMode::extract_links(body),
            vec!["foo".to_string(), "bar".to_string()]
        );
    }

    #[test]
    fn extract_links_skips_native_when_text_differs_from_target() {
        // `[click here](foo.md)` is a generic link, not a topic backlink.
        let body = "[click here](foo.md)";
        assert_eq!(RenderMode::extract_links(body), Vec::<String>::new());
    }

    #[test]
    fn extract_links_ignores_unterminated() {
        assert_eq!(
            RenderMode::extract_links("[[foo]] and [[unclosed"),
            vec!["foo"]
        );
    }

    #[test]
    fn extract_links_ignores_empty() {
        assert_eq!(RenderMode::extract_links("[[]]"), Vec::<String>::new());
    }
}
