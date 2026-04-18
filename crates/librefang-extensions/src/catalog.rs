//! MCP Catalog — read-only set of MCP server templates cached on disk.
//!
//! Templates live at `~/.librefang/mcp/catalog/*.toml` and are refreshed
//! from the upstream `librefang-registry` by
//! [`librefang_runtime::registry_sync`]. The catalog is purely read-only —
//! the user's installed MCP servers live in `config.toml` under
//! `[[mcp_servers]]` with an optional `template_id` pointing back into the
//! catalog.

use crate::{McpCatalogEntry, McpCategory};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// The MCP catalog — in-memory view of all template files under
/// `~/.librefang/mcp/catalog/`.
pub struct McpCatalog {
    /// All known templates, keyed by id.
    entries: HashMap<String, McpCatalogEntry>,
    /// Directory this catalog was loaded from (for reload).
    catalog_dir: PathBuf,
}

impl McpCatalog {
    /// Create a new empty catalog rooted at `home_dir/mcp/catalog/`.
    pub fn new(home_dir: &Path) -> Self {
        Self {
            entries: HashMap::new(),
            catalog_dir: home_dir.join("mcp").join("catalog"),
        }
    }

    /// Load all template files from `home_dir/mcp/catalog/`. Returns count
    /// loaded.
    ///
    /// Accepts `home_dir` explicitly (rather than using the stored path) so
    /// callers can share a single home dir across tests.
    pub fn load(&mut self, home_dir: &std::path::Path) -> usize {
        // Refresh the stored path in case the home_dir differs from
        // construction time (tests sometimes rebind LIBREFANG_HOME).
        self.catalog_dir = home_dir.join("mcp").join("catalog");

        // Full reload semantics: drop everything first so entries deleted
        // or renamed on disk don't linger in the in-memory map.
        self.entries.clear();

        let mut count = 0usize;
        if let Ok(entries) = std::fs::read_dir(&self.catalog_dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Two layouts are valid upstream:
                //   (A) `<id>.toml` — flat file, id from filename minus ext.
                //   (B) `<id>/MCP.toml` — directory-backed (for multi-file
                //       MCP packages), id from directory name.
                // Mirrors `web/scripts/fetch-registry.ts` + the detail-page
                // resolver so catalog loading, live API, and UI agree.
                let (id, manifest_path) = if path.is_file() {
                    match path.file_name().and_then(|n| n.to_str()) {
                        Some(n) if n.ends_with(".toml") => {
                            (n.trim_end_matches(".toml").to_string(), path.clone())
                        }
                        _ => continue,
                    }
                } else if path.is_dir() {
                    let manifest = path.join("MCP.toml");
                    if !manifest.is_file() {
                        continue;
                    }
                    match path.file_name().and_then(|n| n.to_str()) {
                        Some(n) => (n.to_string(), manifest),
                        None => continue,
                    }
                } else {
                    continue;
                };

                let content = match std::fs::read_to_string(&manifest_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                match toml::from_str::<McpCatalogEntry>(&content) {
                    Ok(entry) => {
                        self.entries.insert(id, entry);
                        count += 1;
                    }
                    Err(e) => {
                        warn!("Failed to parse MCP catalog entry '{}': {}", id, e);
                    }
                }
            }
        }

        if count > 0 {
            debug!("Loaded {count} MCP catalog entry/entries");
        }
        count
    }

    /// Get a catalog entry by ID.
    pub fn get(&self, id: &str) -> Option<&McpCatalogEntry> {
        self.entries.get(id)
    }

    /// List all entries, sorted by id.
    pub fn list(&self) -> Vec<&McpCatalogEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
    }

    /// List entries by category.
    pub fn list_by_category(&self, category: &McpCategory) -> Vec<&McpCatalogEntry> {
        self.entries
            .values()
            .filter(|t| &t.category == category)
            .collect()
    }

    /// Search entries by query (matches id, name, description, tags).
    pub fn search(&self, query: &str) -> Vec<&McpCatalogEntry> {
        let q = query.to_lowercase();
        self.entries
            .values()
            .filter(|t| {
                t.id.to_lowercase().contains(&q)
                    || t.name.to_lowercase().contains(&q)
                    || t.description.to_lowercase().contains(&q)
                    || t.tags.iter().any(|tag| tag.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Number of catalog entries currently loaded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Directory this catalog reads from.
    pub fn catalog_dir(&self) -> &Path {
        &self.catalog_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure registry content is available for tests.
    /// resolve_home_dir_for_tests() handles sync internally via OnceLock.
    fn ensure_registry() {
        let _ = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
    }

    #[test]
    fn catalog_load() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut cat = McpCatalog::new(&home);
        let count = cat.load(&home);
        assert!(
            count >= 20,
            "Expected at least 20 MCP catalog entries, got {count}"
        );
        assert_eq!(cat.len(), count);
    }

    #[test]
    fn catalog_get() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut cat = McpCatalog::new(&home);
        cat.load(&home);
        let gh = cat.get("github").unwrap();
        assert_eq!(gh.name, "GitHub");
        assert_eq!(gh.category, McpCategory::DevTools);
    }

    #[test]
    fn catalog_search() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut cat = McpCatalog::new(&home);
        cat.load(&home);
        let results = cat.search("search");
        assert!(results.len() >= 2); // brave-search, exa-search
    }

    #[test]
    fn catalog_list_by_category() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut cat = McpCatalog::new(&home);
        cat.load(&home);
        let devtools = cat.list_by_category(&McpCategory::DevTools);
        assert!(
            devtools.len() >= 6,
            "expected at least 6 DevTools entries, got {}",
            devtools.len()
        );
    }
}
