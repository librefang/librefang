//! Acceptance tests for issue #3329 — Memory wiki: durable knowledge vault.
//!
//! These exercise the public `WikiVault` surface end-to-end against a
//! real on-disk filesystem (tempdir). They mirror the seven-bullet
//! acceptance list in the issue description; each `#[test]` cites the
//! bullet it covers in its function-level doc comment.

use std::fs;

use chrono::Utc;
use librefang_memory_wiki::{
    BacklinkEntry, MemoryWikiConfig, MemoryWikiIngestFilter, MemoryWikiMode, MemoryWikiRenderMode,
    ProvenanceEntry, RenderMode, WikiVault,
};
use tempfile::TempDir;

fn provenance(agent: &str, turn: u64) -> ProvenanceEntry {
    ProvenanceEntry {
        agent: agent.to_string(),
        session: Some("sess_acceptance".to_string()),
        channel: Some("test-harness".to_string()),
        turn: Some(turn),
        at: Utc::now(),
    }
}

fn vault_in(dir: &TempDir, render: RenderMode) -> WikiVault {
    WikiVault::with_root(
        dir.path().to_path_buf(),
        render,
        MemoryWikiIngestFilter::Tagged,
    )
    .expect("vault construction")
}

/// Acceptance #1: scaffold compiles + `enabled = false` is the default and
/// has zero side effects (vault is not constructed).
#[test]
fn default_config_is_disabled_and_construction_short_circuits() {
    let cfg = MemoryWikiConfig::default();
    assert!(!cfg.enabled, "default must be off");
    let err = WikiVault::new(&cfg).expect_err("disabled vault must not construct");
    assert!(matches!(err, librefang_memory_wiki::WikiError::Disabled));
}

/// Acceptance #2: isolated mode end-to-end —
/// `wiki_write` lands a page on disk, `wiki_get` returns it,
/// `wiki_search` finds it.
#[test]
fn isolated_mode_round_trip() {
    let dir = TempDir::new().unwrap();
    let vault = vault_in(&dir, RenderMode::Native);

    let outcome = vault
        .write(
            "project-conventions",
            "We always run `cargo fmt` before commit.",
            provenance("agent_alpha", 1),
            false,
        )
        .unwrap();
    assert!(dir.path().join("project-conventions.md").is_file());
    assert!(!outcome.merged_with_external_edit);

    let page = vault.get("project-conventions").unwrap();
    assert_eq!(page.topic, "project-conventions");
    assert!(page.body.contains("cargo fmt"));

    let hits = vault.search("cargo fmt", 5).unwrap();
    assert!(hits.iter().any(|h| h.topic == "project-conventions"));
}

/// Acceptance #3: provenance frontmatter is populated on every write —
/// each call appends; the vault never drops history.
#[test]
fn provenance_is_populated_on_every_write() {
    let dir = TempDir::new().unwrap();
    let vault = vault_in(&dir, RenderMode::Native);

    vault
        .write("widgets", "v1", provenance("agent_alpha", 1), false)
        .unwrap();
    vault
        .write("widgets", "v2", provenance("agent_beta", 2), false)
        .unwrap();
    vault
        .write("widgets", "v3", provenance("agent_alpha", 3), false)
        .unwrap();

    let page = vault.get("widgets").unwrap();
    assert_eq!(page.frontmatter.provenance.len(), 3);
    let agents: Vec<_> = page
        .frontmatter
        .provenance
        .iter()
        .map(|p| p.agent.as_str())
        .collect();
    assert_eq!(agents, vec!["agent_alpha", "agent_beta", "agent_alpha"]);

    // Frontmatter survives a round-trip through the on-disk YAML.
    let raw = fs::read_to_string(dir.path().join("widgets.md")).unwrap();
    assert!(raw.contains("provenance:"));
    assert!(raw.contains("agent_alpha"));
    assert!(raw.contains("agent_beta"));
    assert!(raw.contains("content_sha256:"));
}

/// Acceptance #4: hand-edit detection — an external edit is preserved
/// rather than silently overwritten.
#[test]
fn external_hand_edit_is_preserved_under_force() {
    let dir = TempDir::new().unwrap();
    let vault = vault_in(&dir, RenderMode::Native);

    vault
        .write("notes", "first version", provenance("agent_a", 1), false)
        .unwrap();

    // Simulate the user opening notes.md in their editor and adding a line.
    let path = dir.path().join("notes.md");
    let mut raw = fs::read_to_string(&path).unwrap();
    raw.push_str("\nhand-typed important caveat\n");
    fs::write(&path, raw).unwrap();

    // A subsequent write without `force` is refused.
    let conflict = vault
        .write("notes", "second version", provenance("agent_a", 2), false)
        .unwrap_err();
    assert!(matches!(
        conflict,
        librefang_memory_wiki::WikiError::HandEditConflict { .. }
    ));

    // With `force`, the body the user typed survives — only provenance is
    // appended. The vault never drops the user's edit.
    let outcome = vault
        .write(
            "notes",
            "this body is intentionally ignored",
            provenance("agent_a", 3),
            true,
        )
        .unwrap();
    assert!(outcome.merged_with_external_edit);
    let page = vault.get("notes").unwrap();
    assert!(page.body.contains("hand-typed important caveat"));
    assert!(!page.body.contains("this body is intentionally ignored"));
    assert_eq!(page.frontmatter.provenance.len(), 2);
}

/// Acceptance #5: obsidian render mode produces `[[wiki-link]]` syntax.
#[test]
fn obsidian_mode_emits_wiki_link_syntax() {
    let dir = TempDir::new().unwrap();
    let vault = vault_in(&dir, RenderMode::Obsidian);
    vault
        .write(
            "graph",
            "see [[adjacent]] and [[other]] for related work",
            provenance("agent_a", 1),
            false,
        )
        .unwrap();

    let raw = fs::read_to_string(dir.path().join("graph.md")).unwrap();
    assert!(
        raw.contains("[[adjacent]]"),
        "obsidian must keep [[]] syntax: {raw}"
    );
    assert!(raw.contains("[[other]]"));
    // Native rewrite must NOT have happened.
    assert!(!raw.contains("[adjacent](adjacent.md)"));
}

/// Acceptance #5 (counterpart): native mode rewrites `[[topic]]` placeholders
/// into plain markdown links so the file opens cleanly in any markdown
/// viewer.
#[test]
fn native_mode_emits_relative_markdown_links() {
    let dir = TempDir::new().unwrap();
    let vault = vault_in(&dir, RenderMode::Native);
    vault
        .write(
            "graph",
            "see [[adjacent]] for related work",
            provenance("agent_a", 1),
            false,
        )
        .unwrap();

    let raw = fs::read_to_string(dir.path().join("graph.md")).unwrap();
    assert!(
        raw.contains("[adjacent](adjacent.md)"),
        "native must rewrite to [topic](topic.md): {raw}"
    );
    assert!(!raw.contains("[[adjacent]]"));
}

/// Acceptance #7: 5 writes with topic tags produce 5 wiki pages with
/// correct backlinks. (The issue text says "5 memory_store calls" — v1
/// ingests via explicit `wiki_write` rather than subscribing to memory
/// events; the topology assertion is the same regardless of source.)
#[test]
fn five_pages_with_links_produce_five_files_and_correct_backlinks() {
    let dir = TempDir::new().unwrap();
    let vault = vault_in(&dir, RenderMode::Native);

    vault
        .write(
            "alpha",
            "alpha references [[beta]] and [[gamma]]",
            provenance("a", 1),
            false,
        )
        .unwrap();
    vault
        .write(
            "beta",
            "beta references [[gamma]] and [[delta]]",
            provenance("a", 2),
            false,
        )
        .unwrap();
    vault
        .write(
            "gamma",
            "gamma references [[delta]]",
            provenance("a", 3),
            false,
        )
        .unwrap();
    vault
        .write(
            "delta",
            "delta references [[epsilon]]",
            provenance("a", 4),
            false,
        )
        .unwrap();
    vault
        .write(
            "epsilon",
            "leaf with no outbound links",
            provenance("a", 5),
            false,
        )
        .unwrap();

    // 5 page files (plus auto-generated index.md).
    let mut topics: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_suffix(".md").map(str::to_string)
        })
        .collect();
    topics.sort();
    assert_eq!(
        topics,
        vec!["alpha", "beta", "delta", "epsilon", "gamma", "index"]
    );

    let mut entries = vault.backlinks().unwrap();
    entries.sort_by(|a, b| {
        a.target
            .cmp(&b.target)
            .then_with(|| a.source.cmp(&b.source))
    });
    let expected = vec![
        // beta is referenced by alpha
        BacklinkEntry {
            source: "alpha".into(),
            target: "beta".into(),
        },
        // delta is referenced by beta and gamma
        BacklinkEntry {
            source: "beta".into(),
            target: "delta".into(),
        },
        BacklinkEntry {
            source: "gamma".into(),
            target: "delta".into(),
        },
        // epsilon is referenced by delta
        BacklinkEntry {
            source: "delta".into(),
            target: "epsilon".into(),
        },
        // gamma is referenced by alpha and beta
        BacklinkEntry {
            source: "alpha".into(),
            target: "gamma".into(),
        },
        BacklinkEntry {
            source: "beta".into(),
            target: "gamma".into(),
        },
    ];
    let mut expected_sorted = expected;
    expected_sorted.sort_by(|a, b| {
        a.target
            .cmp(&b.target)
            .then_with(|| a.source.cmp(&b.source))
    });
    assert_eq!(entries, expected_sorted);

    // Index page lists every topic (and only the topics — no `_meta`).
    let index = fs::read_to_string(dir.path().join("index.md")).unwrap();
    for topic in ["alpha", "beta", "delta", "epsilon", "gamma"] {
        assert!(
            index.contains(topic),
            "index.md should mention `{topic}`: {index}"
        );
    }
    assert!(!index.contains("_meta"));
}

/// `MemoryWikiRenderMode` and `RenderMode` agree at the conversion seam,
/// so config-time choices flow into the vault unchanged.
#[test]
fn render_mode_conversion_round_trip() {
    assert_eq!(
        RenderMode::from(MemoryWikiRenderMode::Native),
        RenderMode::Native
    );
    assert_eq!(
        RenderMode::from(MemoryWikiRenderMode::Obsidian),
        RenderMode::Obsidian
    );
}

/// Bridge / unsafe_local modes are reserved; v1 must surface a specific
/// error so an operator misconfiguration is loud rather than silent.
#[test]
fn reserved_modes_return_specific_error() {
    let dir = TempDir::new().unwrap();
    let cfg = MemoryWikiConfig {
        enabled: true,
        mode: MemoryWikiMode::Bridge,
        vault_path: Some(dir.path().to_path_buf()),
        render_mode: MemoryWikiRenderMode::Native,
        ingest_filter: MemoryWikiIngestFilter::Tagged,
    };
    let err = WikiVault::new(&cfg).unwrap_err();
    assert!(matches!(
        err,
        librefang_memory_wiki::WikiError::ModeNotImplemented("bridge")
    ));
}
