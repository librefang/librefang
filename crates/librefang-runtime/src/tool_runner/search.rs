//! Workspace-wide regex code search tool.

use super::error::{ToolError, ToolResult};
use super::fs::resolve_file_path_ext;
use regex::RegexBuilder;
use std::path::{Path, PathBuf};

/// Directories never worth searching — VCS metadata, build output, vendored deps.
const SKIP_DIRS: &[&str] = &[".git", ".hg", ".svn", "target", "node_modules"];
/// Files larger than this are skipped (generated / vendored / binary blobs).
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Default cap on returned match lines.
const DEFAULT_MAX_RESULTS: usize = 100;
/// Hard ceiling on `max_results` so a caller can't ask for an unbounded dump.
const HARD_MAX_RESULTS: usize = 1000;
/// Stop after scanning this many files so a pathological tree can't hang a turn.
const MAX_FILES_SCANNED: usize = 20_000;
/// Per-line character cap in output so one minified line can't blow the budget.
const MAX_LINE_CHARS: usize = 400;

/// Returns `relpath:line: content` rows sorted by path then line (#3298).
pub(super) async fn tool_code_search(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> ToolResult {
    let query = input["query"]
        .as_str()
        .ok_or(ToolError::MissingParameter("query"))?;
    if query.is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "query",
            reason: "query must not be empty".to_string(),
        });
    }
    let raw_path = input["path"].as_str().unwrap_or(".");
    let root =
        resolve_file_path_ext(raw_path, workspace_root, additional_roots).map_err(|reason| {
            ToolError::InvalidParameter {
                name: "path",
                reason,
            }
        })?;
    let case_sensitive = input["case_sensitive"].as_bool().unwrap_or(false);
    let max_results = (input["max_results"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_MAX_RESULTS))
    .clamp(1, HARD_MAX_RESULTS);

    let re = RegexBuilder::new(query)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| ToolError::InvalidParameter {
            name: "query",
            reason: format!("invalid regex: {e}"),
        })?;

    // Sort entries per directory so the truncation boundary is deterministic (#3298).
    let mut matches: Vec<(String, usize, String)> = Vec::new();
    let mut files_scanned: usize = 0;
    let mut truncated = false;
    let mut stack: Vec<PathBuf> = vec![root.clone()];

    'walk: while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            // Unreadable directory: skip it rather than failing the whole search.
            Err(_) => continue,
        };
        let mut entries: Vec<PathBuf> = Vec::new();
        while let Ok(Some(e)) = rd.next_entry().await {
            entries.push(e.path());
        }
        entries.sort();

        let mut subdirs: Vec<PathBuf> = Vec::new();
        for path in entries {
            // symlink_metadata so we never follow a symlink out of the tree.
            let meta = match tokio::fs::symlink_metadata(&path).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || SKIP_DIRS.contains(&name) {
                    continue;
                }
                subdirs.push(path);
                continue;
            }
            if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                continue;
            }

            files_scanned += 1;
            if files_scanned > MAX_FILES_SCANNED {
                truncated = true;
                break 'walk;
            }

            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            if bytes.contains(&0u8) {
                continue; // binary
            }
            let text = String::from_utf8_lossy(&bytes);
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();
            for (idx, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    matches.push((rel.clone(), idx + 1, clip_line(line)));
                    if matches.len() >= max_results {
                        truncated = true;
                        break 'walk;
                    }
                }
            }
        }
        // Push in reverse so the next pops visit subdirs in sorted order.
        for d in subdirs.into_iter().rev() {
            stack.push(d);
        }
    }

    if matches.is_empty() {
        return Ok(format!("No matches for /{query}/ under '{raw_path}'."));
    }
    matches.sort();
    let mut out = String::with_capacity(matches.len() * 48);
    for (rel, line, content) in &matches {
        out.push_str(&format!("{rel}:{line}: {content}\n"));
    }
    if truncated {
        out.push_str(&format!(
            "--- truncated at {} matches; narrow `path` or refine the query ---\n",
            matches.len()
        ));
    }
    Ok(out)
}

/// Trim a matched line and cap it at [`MAX_LINE_CHARS`] characters, counting by
/// `char` so the cut never lands inside a multi-byte UTF-8 sequence (which a
/// byte-wise `String::truncate` would panic on).
fn clip_line(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() > MAX_LINE_CHARS {
        let mut s: String = t.chars().take(MAX_LINE_CHARS).collect();
        s.push('…');
        s
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn missing_query_is_missing_parameter() {
        let r = tool_code_search(&json!({}), None, &[]).await;
        assert!(
            matches!(r, Err(ToolError::MissingParameter("query"))),
            "{r:?}"
        );
    }

    #[tokio::test]
    async fn invalid_regex_is_invalid_parameter() {
        let tmp = TempDir::new().unwrap();
        let r = tool_code_search(&json!({"query": "(unclosed"}), Some(tmp.path()), &[]).await;
        assert!(
            matches!(r, Err(ToolError::InvalidParameter { name: "query", .. })),
            "{r:?}"
        );
    }

    #[tokio::test]
    async fn finds_matches_with_line_numbers() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "let x = alpha();\n").unwrap();
        let out = tool_code_search(&json!({"query": "alpha"}), Some(tmp.path()), &[])
            .await
            .unwrap();
        assert!(out.contains("a.rs:1: fn alpha() {}"), "got: {out}");
        assert!(out.contains("b.rs:1: let x = alpha();"), "got: {out}");
        // beta line must not appear.
        assert!(!out.contains("beta"), "got: {out}");
    }

    #[tokio::test]
    async fn is_case_insensitive_by_default_and_respects_flag() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "Needle\n").unwrap();
        let ci = tool_code_search(&json!({"query": "needle"}), Some(tmp.path()), &[])
            .await
            .unwrap();
        assert!(
            ci.contains("a.txt:1:"),
            "default should be case-insensitive: {ci}"
        );
        let cs = tool_code_search(
            &json!({"query": "needle", "case_sensitive": true}),
            Some(tmp.path()),
            &[],
        )
        .await
        .unwrap();
        assert!(
            cs.contains("No matches"),
            "case_sensitive should miss: {cs}"
        );
    }

    #[tokio::test]
    async fn skips_git_dir_and_binary_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join(".git").join("config"), "needle in git\n").unwrap();
        std::fs::write(tmp.path().join("bin.dat"), [b'n', b'e', 0u8, b'e', b'd']).unwrap();
        std::fs::write(tmp.path().join("ok.txt"), "needle here\n").unwrap();
        let out = tool_code_search(&json!({"query": "needle"}), Some(tmp.path()), &[])
            .await
            .unwrap();
        assert!(out.contains("ok.txt:1:"), "got: {out}");
        assert!(!out.contains(".git"), "must skip .git: {out}");
        assert!(!out.contains("bin.dat"), "must skip binary: {out}");
    }

    #[tokio::test]
    async fn no_matches_returns_friendly_message() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello\n").unwrap();
        let out = tool_code_search(&json!({"query": "zzz_absent"}), Some(tmp.path()), &[])
            .await
            .unwrap();
        assert!(out.contains("No matches"), "got: {out}");
    }
}
