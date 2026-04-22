//! UAR-AGENT-MD specification support for librefang.
//!
//! Provides three things:
//!
//! 1. **`types`** — Rust structs representing a compiled `AgentArtifact` and
//!    the A2A `AgentCard`, mirroring the canonical UAR schema.
//! 2. **`parser`** — lightweight Markdown → `AgentArtifact` compiler that
//!    understands the 15-section `UAR-AGENT-MD` format without requiring a
//!    running UAR instance.
//! 3. **`translator`** — bidirectional conversion between `AgentArtifact` and
//!    librefang's native `AgentManifest`.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use librefang_uar_spec::{parser, translator};
//!
//! let markdown = std::fs::read_to_string("my_agent.uar.md").unwrap();
//! let artifact = parser::parse(&markdown).unwrap();
//! let manifest = translator::artifact_to_manifest(&artifact).unwrap();
//! ```

pub mod error;
pub mod parser;
pub mod translator;
pub mod types;
