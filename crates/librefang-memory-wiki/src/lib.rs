//! Durable markdown knowledge vault for the LibreFang Agent OS.
//!
//! Companion to `librefang-memory` (the SQLite + vector substrate). Where the
//! memory substrate is good at "find me the K nearest snippets", this vault
//! is good at "give me a navigable knowledge base I can also open in Obsidian
//! and edit by hand". Every page carries provenance frontmatter (which agent
//! / session captured the claim, when, on whose channel) so a reader can
//! always audit where a fact came from.
//!
//! The vault is **off by default**. Operators opt in via:
//!
//! ```toml
//! [memory_wiki]
//! enabled = true
//! mode = "isolated"                           # isolated | bridge | unsafe_local
//! vault_path = "~/.librefang/wiki/main"
//! render_mode = "native"                      # native | obsidian
//! ingest_filter = "tagged"                    # tagged | all
//! ```
//!
//! See `docs/architecture/memory-wiki.md` and the original RFC at issue #3329.
//!
//! ## v1 scope
//!
//! - `isolated` mode: own vault, own writes; no dependency on the active
//!   memory plugin.
//! - Three builtin tools: `wiki_get`, `wiki_search`, `wiki_write`.
//! - `native` and `obsidian` render modes.
//! - Hand-edit safety: external edits (mtime newer than the last compiler
//!   run) are preserved; the vault re-parses the file before merging new
//!   provenance entries.
//!
//! ## Out of scope for v1 (tracked under #3329 follow-ups)
//!
//! - `bridge` mode: read shared artifacts from the memory substrate via the
//!   public seams. The trait surface is the same (`WikiVault`), and the
//!   `MemoryWikiMode` enum already carries the variant; the read path is
//!   stubbed with a `not_yet_implemented` error.
//! - `unsafe_local` mode: same-machine escape hatch for an existing Obsidian
//!   vault. Same trait, same stub.
//! - Memory-event subscription (`memory_store` durable filter). v1 ingests
//!   only via explicit `wiki_write` calls. The hook contract is left to
//!   #3326's `before_prompt_build` infrastructure.
//! - LLM-assisted topic extraction. v1 requires explicit `topic` tags.
//! - `memory_search` cross-corpus parameter (`corpus = all|kv|wiki`). The
//!   builtin lives in `librefang-runtime`; extending it touches the runtime
//!   tool surface and should land as a follow-up so the wiki crate stays
//!   independently usable.

pub mod error;
pub mod frontmatter;
pub mod render;
pub mod vault;

pub use error::{WikiError, WikiResult};
pub use frontmatter::{Frontmatter, ProvenanceEntry};
pub use librefang_types::config::{MemoryWikiIngestFilter, MemoryWikiRenderMode};
pub use render::RenderMode;
pub use vault::{
    BacklinkEntry, MemoryWikiConfig, MemoryWikiMode, SearchHit, WikiPage, WikiVault,
    WikiWriteOutcome,
};
