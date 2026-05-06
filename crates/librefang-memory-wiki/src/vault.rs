//! Isolated vault store. v1 of issue #3329.
//!
//! Layout under `<vault_path>/`:
//!
//! ```text
//! <topic>.md             # one page per topic, frontmatter + body
//! index.md               # auto-generated alphabetical index
//! _meta/
//!   compile-state.json   # mtime + sha256 of every page on the last compile
//!   backlinks.json       # { target -> [source, ...] } from every [[link]]
//! ```
//!
//! Authoring contract: `wiki_write` callers pass body markdown that uses
//! `[[topic]]` placeholders for cross-references. The vault rewrites those
//! into the active render flavor (`Native` -> `[topic](topic.md)`,
//! `Obsidian` -> `[[topic]]`) at flush time, so the same body is portable
//! across modes without re-authoring.
//!
//! Hand-edit safety (issue #3329 acceptance criterion 4): every write
//! compares the on-disk mtime *and* the body sha256 against the compiler
//! state from the previous run. If either drifts, the page was edited
//! externally; the write is rejected with `WikiError::HandEditConflict`
//! unless the caller passes `force = true`. The forced path preserves the
//! external edit by treating it as the new base — only the provenance list
//! is augmented, the on-disk body is left intact except for link rewriting.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

pub use librefang_types::config::{MemoryWikiConfig, MemoryWikiIngestFilter, MemoryWikiMode};

use crate::error::{WikiError, WikiResult};
use crate::frontmatter::{self, Frontmatter, ProvenanceEntry};
use crate::render::RenderMode;

const RESERVED_TOPIC_INDEX: &str = "index";
const META_DIR: &str = "_meta";
const COMPILE_STATE_FILE: &str = "compile-state.json";
const BACKLINKS_FILE: &str = "backlinks.json";
const MAX_TOPIC_LEN: usize = 100;
/// Soft cap on a single page body. Sized to comfortably hold a long-form
/// agent note (essays, research summaries) while preventing a runaway LLM
/// from filling the disk page-by-page. Bytes are counted **after**
/// `[[link]]` rewrite, since that is what actually lands on disk.
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub topic: String,
    pub frontmatter: Frontmatter,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiWriteOutcome {
    pub topic: String,
    pub path: PathBuf,
    pub content_sha256: String,
    /// `true` if the caller passed `force = true` and the previous on-disk
    /// content had drifted from the last compiler run. Tells the caller a
    /// human edit was preserved instead of overwritten.
    pub merged_with_external_edit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub topic: String,
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklinkEntry {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CompileState {
    /// Topic -> last-known disk state.
    #[serde(default)]
    pages: BTreeMap<String, PageState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PageState {
    /// `SystemTime` modified, expressed as nanoseconds since the UNIX
    /// epoch. Stored as a string because some json consumers can't take a
    /// 128-bit number; precision varies by filesystem but is self-
    /// consistent within a single host. Format is canonical decimal with
    /// no leading zeros and no thousands separators (`u128::to_string`),
    /// so equality across two snapshots is a stable byte compare.
    mtime_ns: String,
    /// `Frontmatter::hash_body(rendered_body)` of the page body emitted by
    /// the last successful compile. Diverges immediately if a human saves
    /// the file from outside the vault, so it survives filesystems with
    /// 1-second mtime precision (HFS+).
    sha256: String,
}

#[derive(Debug)]
pub struct WikiVault {
    root: PathBuf,
    render_mode: RenderMode,
    /// Reserved for future memory-event subscription path; v1 honours this
    /// only at the trait layer (`MemoryWikiIngestFilter::Tagged` is the
    /// effective behaviour today because every `wiki_write` call already
    /// carries an explicit topic).
    #[allow(dead_code)]
    ingest_filter: MemoryWikiIngestFilter,
    write_lock: Mutex<()>,
}

impl WikiVault {
    /// Construct a new isolated-mode vault rooted under `home_dir`.
    ///
    /// `home_dir` is consulted only when `config.vault_path` is unset —
    /// it is the kernel's own home directory (`KernelConfig.home_dir`)
    /// rather than the env-derived `LIBREFANG_HOME`, so embedded
    /// profiles and tests don't silently mix data with a developer's
    /// `~/.librefang/wiki/main`.
    ///
    /// Returns `WikiError::Disabled` when the operator has not flipped
    /// `enabled = true`, and `WikiError::ModeNotImplemented` for the
    /// `bridge` / `unsafe_local` modes that v1 does not wire.
    pub fn new(config: &MemoryWikiConfig, home_dir: &Path) -> WikiResult<Self> {
        if !config.enabled {
            return Err(WikiError::Disabled);
        }
        match config.mode {
            MemoryWikiMode::Isolated => {}
            MemoryWikiMode::Bridge => {
                return Err(WikiError::ModeNotImplemented("bridge"));
            }
            MemoryWikiMode::UnsafeLocal => {
                return Err(WikiError::ModeNotImplemented("unsafe_local"));
            }
        }
        // `ingest_filter = All` is reserved for the future memory-event
        // subscription path. v1 ingests via explicit `wiki_write` only, so
        // the field has no behavioural effect today. Tell operators
        // loudly rather than silently — a non-default value is a usable
        // signal of misconfigured expectations.
        if matches!(config.ingest_filter, MemoryWikiIngestFilter::All) {
            tracing::warn!(
                "[memory_wiki] ingest_filter = \"all\" has no effect in v1 — \
                 the field is reserved for future memory-event ingest \
                 (issue #3329 follow-up). Today every wiki_write is \
                 accepted regardless of this setting."
            );
        }
        let root = config.resolved_vault_path(home_dir);
        Self::with_root(
            root,
            RenderMode::from(config.render_mode),
            config.ingest_filter,
        )
    }

    /// Construct a vault rooted at `root`. Bypasses the `enabled` check —
    /// used by tests and by the `KernelHandle` impl after it has already
    /// validated config.
    pub fn with_root(
        root: PathBuf,
        render_mode: RenderMode,
        ingest_filter: MemoryWikiIngestFilter,
    ) -> WikiResult<Self> {
        fs::create_dir_all(&root).map_err(|e| WikiError::io(root.display().to_string(), e))?;
        let meta = root.join(META_DIR);
        fs::create_dir_all(&meta).map_err(|e| WikiError::io(meta.display().to_string(), e))?;
        Ok(Self {
            root,
            render_mode,
            ingest_filter,
            write_lock: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn render_mode(&self) -> RenderMode {
        self.render_mode
    }

    /// Render the canonical body (with `[[topic]]` placeholders) into the
    /// flavor the vault is configured for and write the page atomically.
    ///
    /// `provenance` is appended to whatever provenance the page already
    /// carries — provenance is monotonic; the vault never drops history.
    pub fn write(
        &self,
        topic: &str,
        body_with_placeholders: &str,
        provenance: ProvenanceEntry,
        force: bool,
    ) -> WikiResult<WikiWriteOutcome> {
        validate_topic(topic)?;
        if body_with_placeholders.len() > MAX_BODY_BYTES {
            return Err(WikiError::BodyTooLarge {
                topic: topic.to_string(),
                size: body_with_placeholders.len(),
                cap: MAX_BODY_BYTES,
            });
        }
        let _guard = self.write_lock.lock().expect("vault write lock poisoned");

        let path = self.page_path(topic);
        let mut compile_state = self.load_compile_state()?;
        let existing = read_page_if_present(&path, topic)?;
        let drifted = if let Some(page) = existing.as_ref() {
            let actual = page_state_from_disk(&path, &page.body)?;
            match compile_state.pages.get(topic) {
                Some(prev) => prev.mtime_ns != actual.mtime_ns || prev.sha256 != actual.sha256,
                None => true,
            }
        } else {
            false
        };
        if drifted && !force {
            return Err(WikiError::HandEditConflict {
                topic: topic.to_string(),
            });
        }

        // Decide which body wins. When the caller forces an overwrite over
        // an external edit, we preserve the external body verbatim (only
        // provenance is appended). Otherwise the caller's body is the new
        // truth.
        let chosen_body = if drifted {
            existing
                .as_ref()
                .map(|p| p.body.clone())
                .unwrap_or_else(|| body_with_placeholders.to_string())
        } else {
            self.render_mode.rewrite_links(body_with_placeholders)
        };

        let mut frontmatter_out = match existing {
            Some(WikiPage {
                frontmatter: mut fm,
                ..
            }) => {
                fm.topic = topic.to_string();
                fm.updated = chrono::Utc::now();
                fm.provenance.push(provenance);
                fm
            }
            None => {
                let mut fm = Frontmatter::default_for(topic);
                fm.provenance.push(provenance);
                fm
            }
        };
        let content_sha256 = Frontmatter::hash_body(&chosen_body);
        frontmatter_out.content_sha256 = content_sha256.clone();

        let raw = frontmatter::render(&frontmatter_out, &chosen_body)?;
        atomic_write(&path, raw.as_bytes())?;

        let mtime_ns = mtime_ns_for(&path)?;
        compile_state.pages.insert(
            topic.to_string(),
            PageState {
                mtime_ns,
                sha256: content_sha256.clone(),
            },
        );
        self.save_compile_state(&compile_state)?;
        self.rebuild_index_and_backlinks(&compile_state)?;

        Ok(WikiWriteOutcome {
            topic: topic.to_string(),
            path,
            content_sha256,
            merged_with_external_edit: drifted,
        })
    }

    /// Read a single page. Returns `WikiError::NotFound` if no markdown
    /// file exists for `topic`.
    pub fn get(&self, topic: &str) -> WikiResult<WikiPage> {
        validate_topic(topic)?;
        let path = self.page_path(topic);
        if !path.exists() {
            return Err(WikiError::NotFound(topic.to_string()));
        }
        read_page_if_present(&path, topic)?.ok_or_else(|| WikiError::NotFound(topic.to_string()))
    }

    /// Naive case-insensitive substring search across every page body. v1
    /// scope: works for the "5 pages with topic tags" acceptance test and
    /// keeps the dependency surface small. Vector / FTS5 ranking is a
    /// follow-up tracked under #3329.
    pub fn search(&self, query: &str, limit: usize) -> WikiResult<Vec<SearchHit>> {
        let query_lc = query.trim().to_lowercase();
        if query_lc.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits: Vec<SearchHit> = Vec::new();
        for entry in self.iter_page_files()? {
            let topic = entry.topic;
            let raw = fs::read_to_string(&entry.path)
                .map_err(|e| WikiError::io(entry.path.display().to_string(), e))?;
            let (_yaml, body) = frontmatter::split(&raw);
            let body_lc = body.to_lowercase();
            let topic_lc = topic.to_lowercase();
            let mut score = 0.0_f64;
            if topic_lc.contains(&query_lc) {
                score += 10.0;
            }
            let body_matches = body_lc.matches(&query_lc).count();
            // Sub-linear weighting on body hits so a single long page can't
            // bury short topic-only matches under sheer volume. The +1
            // shift keeps ln() non-negative on the first hit.
            if body_matches > 0 {
                score += (1.0 + body_matches as f64).ln();
            }
            if score <= 0.0 {
                continue;
            }
            let snippet = build_snippet(body, &body_lc, &query_lc);
            hits.push(SearchHit {
                topic,
                snippet,
                score,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.topic.cmp(&b.topic))
        });
        hits.truncate(limit.max(1));
        Ok(hits)
    }

    /// List every backlink (`source` page contains `[[target]]`) the vault
    /// currently tracks. Order is deterministic (target asc, source asc).
    pub fn backlinks(&self) -> WikiResult<Vec<BacklinkEntry>> {
        let path = self.root.join(META_DIR).join(BACKLINKS_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw =
            fs::read_to_string(&path).map_err(|e| WikiError::io(path.display().to_string(), e))?;
        let map: BTreeMap<String, Vec<String>> = serde_json::from_str(&raw).map_err(|_| {
            WikiError::io(
                path.display().to_string(),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "backlinks.json is not valid JSON",
                ),
            )
        })?;
        let mut out = Vec::new();
        for (target, sources) in map {
            for source in sources {
                out.push(BacklinkEntry {
                    source,
                    target: target.clone(),
                });
            }
        }
        Ok(out)
    }

    fn page_path(&self, topic: &str) -> PathBuf {
        self.root.join(format!("{topic}.md"))
    }

    fn compile_state_path(&self) -> PathBuf {
        self.root.join(META_DIR).join(COMPILE_STATE_FILE)
    }

    fn load_compile_state(&self) -> WikiResult<CompileState> {
        let path = self.compile_state_path();
        if !path.exists() {
            return Ok(CompileState::default());
        }
        let raw =
            fs::read_to_string(&path).map_err(|e| WikiError::io(path.display().to_string(), e))?;
        serde_json::from_str(&raw).map_err(|_| {
            WikiError::io(
                path.display().to_string(),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "compile-state.json is not valid JSON",
                ),
            )
        })
    }

    fn save_compile_state(&self, state: &CompileState) -> WikiResult<()> {
        let path = self.compile_state_path();
        let raw = serde_json::to_vec_pretty(state).expect("serialize CompileState");
        atomic_write(&path, &raw)
    }

    fn iter_page_files(&self) -> WikiResult<Vec<PageEntry>> {
        let entries = fs::read_dir(&self.root)
            .map_err(|e| WikiError::io(self.root.display().to_string(), e))?;
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| WikiError::io(self.root.display().to_string(), e))?;
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !name.ends_with(".md") {
                continue;
            }
            let topic = name.strip_suffix(".md").unwrap();
            if topic == RESERVED_TOPIC_INDEX {
                continue;
            }
            out.push(PageEntry {
                topic: topic.to_string(),
                path,
            });
        }
        out.sort_by(|a, b| a.topic.cmp(&b.topic));
        Ok(out)
    }

    fn rebuild_index_and_backlinks(&self, compile_state: &CompileState) -> WikiResult<()> {
        let mut backlinks: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut index_rows: Vec<(String, chrono::DateTime<chrono::Utc>)> = Vec::new();

        for entry in self.iter_page_files()? {
            let raw = fs::read_to_string(&entry.path)
                .map_err(|e| WikiError::io(entry.path.display().to_string(), e))?;
            let (yaml, body) = frontmatter::split(&raw);
            let updated = match yaml.and_then(|y| frontmatter::parse(y, &entry.topic).ok()) {
                Some(fm) => fm.updated,
                None => chrono::Utc::now(),
            };
            index_rows.push((entry.topic.clone(), updated));
            for target in RenderMode::extract_links(body) {
                let bucket = backlinks.entry(target).or_default();
                bucket.push(entry.topic.clone());
            }
        }
        for sources in backlinks.values_mut() {
            sources.sort();
            sources.dedup();
        }
        index_rows.sort_by(|a, b| a.0.cmp(&b.0));

        let mut index_md = String::with_capacity(64 + index_rows.len() * 80);
        index_md.push_str("# Wiki Index\n\n");
        if index_rows.is_empty() {
            index_md.push_str("_(empty)_\n");
        } else {
            for (topic, updated) in &index_rows {
                let link = self.render_mode.link(topic);
                index_md.push_str(&format!("- {} — updated {}\n", link, updated.to_rfc3339()));
            }
        }
        atomic_write(&self.root.join("index.md"), index_md.as_bytes())?;

        let backlinks_path = self.root.join(META_DIR).join(BACKLINKS_FILE);
        let backlinks_raw = serde_json::to_vec_pretty(&backlinks).expect("serialize backlinks map");
        atomic_write(&backlinks_path, &backlinks_raw)?;

        let _ = compile_state;
        Ok(())
    }
}

struct PageEntry {
    topic: String,
    path: PathBuf,
}

fn validate_topic(topic: &str) -> WikiResult<()> {
    if topic.is_empty() {
        return Err(WikiError::InvalidTopic {
            topic: topic.to_string(),
            reason: "topic must not be empty",
        });
    }
    if topic.len() > MAX_TOPIC_LEN {
        return Err(WikiError::InvalidTopic {
            topic: topic.to_string(),
            reason: "topic exceeds 100 characters",
        });
    }
    if topic == RESERVED_TOPIC_INDEX {
        return Err(WikiError::InvalidTopic {
            topic: topic.to_string(),
            reason: "`index` is reserved for the auto-generated index page",
        });
    }
    if topic.starts_with('_') {
        return Err(WikiError::InvalidTopic {
            topic: topic.to_string(),
            reason: "topics starting with `_` are reserved for vault metadata",
        });
    }
    for ch in topic.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            return Err(WikiError::InvalidTopic {
                topic: topic.to_string(),
                reason: "topic must match [a-zA-Z0-9_-]+",
            });
        }
    }
    Ok(())
}

fn read_page_if_present(path: &Path, topic: &str) -> WikiResult<Option<WikiPage>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|e| WikiError::io(path.display().to_string(), e))?;
    let (yaml, body) = frontmatter::split(&raw);
    let frontmatter = match yaml {
        Some(y) => match frontmatter::parse(y, topic) {
            Ok(fm) => fm,
            // The page exists but the YAML block is malformed (e.g. a hand
            // editor saved invalid YAML, or the file was hand-typed without
            // following the schema). Don't fail the read — that would brick
            // every read after the bad save. Synthesise a default header so
            // the body remains accessible; the next successful `wiki_write`
            // re-renders the page with a clean header.
            Err(err) => {
                tracing::warn!(
                    topic = %topic,
                    path = %path.display(),
                    error = %err,
                    "wiki page frontmatter failed to parse; falling back to a synthetic header — \
                     write the page again to repair the YAML block"
                );
                Frontmatter::default_for(topic)
            }
        },
        None => Frontmatter::default_for(topic),
    };
    Ok(Some(WikiPage {
        topic: topic.to_string(),
        frontmatter,
        body: body.to_string(),
    }))
}

fn page_state_from_disk(path: &Path, body: &str) -> WikiResult<PageState> {
    Ok(PageState {
        mtime_ns: mtime_ns_for(path)?,
        sha256: Frontmatter::hash_body(body),
    })
}

fn mtime_ns_for(path: &Path) -> WikiResult<String> {
    let meta = fs::metadata(path).map_err(|e| WikiError::io(path.display().to_string(), e))?;
    let ts = meta
        .modified()
        .map_err(|e| WikiError::io(path.display().to_string(), e))?;
    let dur = ts
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_nanos(0));
    Ok(dur.as_nanos().to_string())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> WikiResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| WikiError::io(parent.display().to_string(), e))?;
    }
    let tmp = path.with_extension("tmp.write");
    fs::write(&tmp, bytes).map_err(|e| WikiError::io(tmp.display().to_string(), e))?;
    fs::rename(&tmp, path).map_err(|e| WikiError::io(path.display().to_string(), e))?;
    Ok(())
}

fn build_snippet(body: &str, body_lc: &str, query_lc: &str) -> String {
    if let Some(pos) = body_lc.find(query_lc) {
        let start = char_floor(body, pos.saturating_sub(60));
        let end = char_ceil(body, (pos + query_lc.len() + 60).min(body.len()));
        let mut snippet = body[start..end].replace('\n', " ");
        if start > 0 {
            snippet.insert(0, '…');
        }
        if end < body.len() {
            snippet.push('…');
        }
        snippet
    } else {
        body.chars().take(120).collect::<String>()
    }
}

fn char_floor(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn char_ceil(s: &str, mut idx: usize) -> usize {
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn fresh_vault(render: RenderMode) -> (WikiVault, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let vault = WikiVault::with_root(
            dir.path().to_path_buf(),
            render,
            MemoryWikiIngestFilter::Tagged,
        )
        .expect("vault");
        (vault, dir)
    }

    fn provenance(agent: &str) -> ProvenanceEntry {
        ProvenanceEntry {
            agent: agent.to_string(),
            session: Some("sess_test".to_string()),
            channel: Some("test".to_string()),
            turn: Some(1),
            at: Utc::now(),
        }
    }

    #[test]
    fn write_and_get_roundtrip() {
        let (vault, _dir) = fresh_vault(RenderMode::Native);
        vault
            .write("widgets", "the widget body", provenance("agent_a"), false)
            .unwrap();
        let page = vault.get("widgets").unwrap();
        assert_eq!(page.topic, "widgets");
        assert_eq!(page.body.trim(), "the widget body");
        assert_eq!(page.frontmatter.provenance.len(), 1);
    }

    #[test]
    fn provenance_appends_on_repeat_writes() {
        let (vault, _dir) = fresh_vault(RenderMode::Native);
        vault.write("a", "v1", provenance("alpha"), false).unwrap();
        vault.write("a", "v2", provenance("beta"), false).unwrap();
        let page = vault.get("a").unwrap();
        assert_eq!(page.frontmatter.provenance.len(), 2);
        assert_eq!(page.frontmatter.provenance[0].agent, "alpha");
        assert_eq!(page.frontmatter.provenance[1].agent, "beta");
    }

    #[test]
    fn handedit_detection_blocks_silent_overwrite() {
        let (vault, dir) = fresh_vault(RenderMode::Native);
        vault.write("topic", "v1", provenance("a"), false).unwrap();
        // Simulate an external edit by rewriting the file directly.
        let path = dir.path().join("topic.md");
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.push_str("\nhand-edited line\n");
        fs::write(&path, raw).unwrap();

        let result = vault.write("topic", "v2", provenance("a"), false);
        assert!(matches!(result, Err(WikiError::HandEditConflict { .. })));

        // Forcing the write preserves the hand-edited body and merely
        // appends a new provenance entry.
        let outcome = vault
            .write("topic", "v2-ignored", provenance("a"), true)
            .unwrap();
        assert!(outcome.merged_with_external_edit);
        let page = vault.get("topic").unwrap();
        assert!(page.body.contains("hand-edited line"));
        assert_eq!(page.frontmatter.provenance.len(), 2);
    }

    #[test]
    fn obsidian_render_emits_wiki_links() {
        let (vault, dir) = fresh_vault(RenderMode::Obsidian);
        vault
            .write("alpha", "see [[beta]] for details", provenance("a"), false)
            .unwrap();
        let raw = fs::read_to_string(dir.path().join("alpha.md")).unwrap();
        assert!(
            raw.contains("[[beta]]"),
            "obsidian render should keep [[link]] syntax: {raw}"
        );
    }

    #[test]
    fn native_render_rewrites_links_to_relative_paths() {
        let (vault, dir) = fresh_vault(RenderMode::Native);
        vault
            .write("alpha", "see [[beta]] please", provenance("a"), false)
            .unwrap();
        let raw = fs::read_to_string(dir.path().join("alpha.md")).unwrap();
        assert!(
            raw.contains("[beta](beta.md)"),
            "native render should produce [topic](topic.md): {raw}"
        );
        assert!(!raw.contains("[[beta]]"));
    }

    #[test]
    fn search_finds_matches_and_orders_by_score() {
        let (vault, _dir) = fresh_vault(RenderMode::Native);
        vault
            .write(
                "alpha",
                "lorem ipsum dolor sit amet",
                provenance("a"),
                false,
            )
            .unwrap();
        vault
            .write(
                "beta",
                "the quick brown fox jumps over the lazy dog",
                provenance("a"),
                false,
            )
            .unwrap();
        vault
            .write("foxhole", "topic about a hole", provenance("a"), false)
            .unwrap();
        let hits = vault.search("fox", 10).unwrap();
        // foxhole's topic matches → +10, beta's body matches → +1.
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].topic, "foxhole");
        assert_eq!(hits[1].topic, "beta");
    }

    #[test]
    fn backlinks_are_built_deterministically() {
        let (vault, _dir) = fresh_vault(RenderMode::Native);
        vault
            .write("a", "links to [[b]] and [[c]]", provenance("x"), false)
            .unwrap();
        vault
            .write("b", "links to [[c]]", provenance("x"), false)
            .unwrap();
        vault.write("c", "leaf", provenance("x"), false).unwrap();
        let mut entries = vault.backlinks().unwrap();
        entries.sort_by(|x, y| {
            x.target
                .cmp(&y.target)
                .then_with(|| x.source.cmp(&y.source))
        });
        assert_eq!(
            entries,
            vec![
                BacklinkEntry {
                    source: "a".into(),
                    target: "b".into()
                },
                BacklinkEntry {
                    source: "a".into(),
                    target: "c".into()
                },
                BacklinkEntry {
                    source: "b".into(),
                    target: "c".into()
                },
            ]
        );
    }

    #[test]
    fn validate_topic_rejects_reserved_and_bad_chars() {
        assert!(matches!(
            validate_topic("index"),
            Err(WikiError::InvalidTopic { .. })
        ));
        assert!(matches!(
            validate_topic("_meta"),
            Err(WikiError::InvalidTopic { .. })
        ));
        assert!(matches!(
            validate_topic("with space"),
            Err(WikiError::InvalidTopic { .. })
        ));
        assert!(matches!(
            validate_topic(""),
            Err(WikiError::InvalidTopic { .. })
        ));
        validate_topic("ok-topic_42").unwrap();
    }

    #[test]
    fn disabled_config_returns_disabled_error() {
        let cfg = MemoryWikiConfig::default();
        // home_dir is unused on the disabled-fast-path; pass anything.
        let err = WikiVault::new(&cfg, Path::new("/tmp")).unwrap_err();
        assert!(matches!(err, WikiError::Disabled));
    }

    #[test]
    fn unimplemented_modes_return_specific_error() {
        let dir = TempDir::new().unwrap();
        let mut cfg = MemoryWikiConfig {
            enabled: true,
            mode: MemoryWikiMode::Bridge,
            vault_path: Some(dir.path().to_path_buf()),
            ..MemoryWikiConfig::default()
        };
        let err = WikiVault::new(&cfg, dir.path()).unwrap_err();
        assert!(matches!(err, WikiError::ModeNotImplemented("bridge")));
        cfg.mode = MemoryWikiMode::UnsafeLocal;
        let err = WikiVault::new(&cfg, dir.path()).unwrap_err();
        assert!(matches!(err, WikiError::ModeNotImplemented("unsafe_local")));
    }

    #[test]
    fn default_vault_path_uses_caller_home_not_env() {
        // When vault_path is unset, the resolved vault root must live
        // under the caller-supplied home_dir, not under the env-derived
        // librefang_home_dir().
        let dir = TempDir::new().unwrap();
        let cfg = MemoryWikiConfig {
            enabled: true,
            ..MemoryWikiConfig::default()
        };
        let resolved = cfg.resolved_vault_path(dir.path());
        assert!(
            resolved.starts_with(dir.path()),
            "resolved vault path {resolved:?} must be under {:?}",
            dir.path()
        );
    }

    #[test]
    fn write_rejects_body_over_one_mib() {
        let (vault, _dir) = fresh_vault(RenderMode::Native);
        let too_big = "x".repeat(MAX_BODY_BYTES + 1);
        let err = vault
            .write("big", &too_big, provenance("a"), false)
            .unwrap_err();
        match err {
            WikiError::BodyTooLarge { topic, size, cap } => {
                assert_eq!(topic, "big");
                assert_eq!(size, MAX_BODY_BYTES + 1);
                assert_eq!(cap, MAX_BODY_BYTES);
            }
            other => panic!("expected BodyTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn malformed_frontmatter_falls_back_instead_of_failing_get() {
        let (vault, dir) = fresh_vault(RenderMode::Native);
        // Land a clean page first so the file exists, then corrupt the
        // YAML block with a hand-edit.
        vault
            .write("topic", "real body", provenance("a"), false)
            .unwrap();
        let path = dir.path().join("topic.md");
        let raw = std::fs::read_to_string(&path).unwrap();
        let body_at = raw.find("\n---\n\n").expect("split marker present");
        let mut corrupted = String::from("---\nthis: is\n: not valid: yaml: at all\n");
        corrupted.push_str(&raw[body_at..]);
        std::fs::write(&path, corrupted).unwrap();
        // `get` must succeed despite the broken header — the body should
        // still come through with a synthetic default frontmatter.
        let page = vault.get("topic").unwrap();
        assert!(page.body.contains("real body"));
        assert_eq!(page.frontmatter.topic, "topic");
        assert!(page.frontmatter.provenance.is_empty());
    }

    #[test]
    fn concurrent_writes_to_same_topic_are_serialised() {
        use std::sync::Arc;
        use std::thread;
        let dir = TempDir::new().unwrap();
        let vault = Arc::new(
            WikiVault::with_root(
                dir.path().to_path_buf(),
                RenderMode::Native,
                MemoryWikiIngestFilter::Tagged,
            )
            .unwrap(),
        );
        let mut handles = Vec::new();
        for i in 0..8 {
            let v = Arc::clone(&vault);
            handles.push(thread::spawn(move || {
                v.write(
                    "shared",
                    &format!("body from thread {i}"),
                    provenance(&format!("agent_{i}")),
                    false,
                )
            }));
        }
        let mut ok_count = 0;
        for h in handles {
            if h.join().unwrap().is_ok() {
                ok_count += 1;
            }
        }
        // Every write either serialises cleanly or is rejected by the
        // hand-edit detector (a previous write within the same race
        // already updated mtime/sha) — neither outcome is data loss.
        assert!(ok_count >= 1, "at least one write should land");
        let page = vault.get("shared").unwrap();
        // Provenance is monotonic: surviving writes appended their entries.
        assert_eq!(page.frontmatter.provenance.len(), ok_count);
    }
}
