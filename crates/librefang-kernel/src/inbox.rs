//! File-based input inbox — polls a directory for text files and dispatches
//! them as messages to agents.
//!
//! # File format
//!
//! A plain text file dropped into the inbox directory.  The first line may
//! contain an `agent:<name>` directive that overrides the default target agent.
//! The rest of the file (or the entire file when no directive is present) is
//! sent as the message body.
//!
//! Processed files are moved to `inbox/processed/` to avoid redelivery.

use crate::kernel::LibreFangKernel;
use librefang_types::config::InboxConfig;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Maximum file size we will read (1 MB).
const MAX_FILE_SIZE: u64 = 1_048_576;

/// Status snapshot returned by the `/api/inbox/status` endpoint.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxStatus {
    pub enabled: bool,
    pub directory: String,
    pub poll_interval_secs: u64,
    pub default_agent: Option<String>,
    pub pending_count: usize,
    pub processed_count: usize,
}

/// Resolve the effective inbox directory from config.
pub fn resolve_inbox_dir(config: &InboxConfig, home_dir: &Path) -> PathBuf {
    config
        .directory
        .as_deref()
        .map(expand_home_dir)
        .unwrap_or_else(|| home_dir.join("inbox"))
}

fn expand_home_dir(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }

    PathBuf::from(path)
}

/// Collect current inbox status (sync — reads fs metadata only).
pub fn inbox_status(config: &InboxConfig, home_dir: &Path) -> InboxStatus {
    let dir = resolve_inbox_dir(config, home_dir);
    let processed_dir = dir.join("processed");

    let pending_count = count_text_files(&dir);
    let processed_count = count_text_files(&processed_dir);

    InboxStatus {
        enabled: config.enabled,
        directory: dir.to_string_lossy().into_owned(),
        poll_interval_secs: config.poll_interval_secs,
        default_agent: config.default_agent.clone(),
        pending_count,
        processed_count,
    }
}

/// Start the inbox polling loop as a background tokio task.
///
/// The task runs until the kernel's supervisor signals shutdown.
pub fn start_inbox_watcher(kernel: Arc<LibreFangKernel>) {
    let cfg = kernel.config.load();
    let config = cfg.inbox.clone();
    if !config.enabled {
        debug!("Inbox watcher disabled");
        return;
    }

    let inbox_dir = resolve_inbox_dir(&config, &cfg.home_dir);
    let processed_dir = inbox_dir.join("processed");

    // Ensure directories exist
    if let Err(e) = std::fs::create_dir_all(&inbox_dir) {
        warn!(path = %inbox_dir.display(), error = %e, "Failed to create inbox directory");
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&processed_dir) {
        warn!(path = %processed_dir.display(), error = %e, "Failed to create inbox/processed directory");
        return;
    }

    let poll_interval = std::time::Duration::from_secs(config.poll_interval_secs.max(1));

    info!(
        dir = %inbox_dir.display(),
        interval_secs = config.poll_interval_secs,
        default_agent = ?config.default_agent,
        "Inbox watcher started"
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(poll_interval);
        // Track files we have already queued so a slow send_message doesn't
        // cause double-processing before the file is moved.
        let mut in_flight: HashSet<PathBuf> = HashSet::new();

        loop {
            interval.tick().await;

            if kernel.supervisor.is_shutting_down() {
                info!("Inbox watcher stopping (shutdown)");
                break;
            }

            let entries = match tokio::fs::read_dir(&inbox_dir).await {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "Inbox: failed to read directory");
                    continue;
                }
            };

            let mut entries = entries;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();

                // Skip directories and the processed subdirectory
                if path.is_dir() {
                    continue;
                }

                // Skip files already in-flight
                if in_flight.contains(&path) {
                    continue;
                }

                // Skip files quarantined by a previous failed empty-file move.
                // Match the exact suffix shape `*.quarantined.YYYYMMDD_HHMMSS`
                // (optionally with a `.NNNN` nanosecond tiebreaker) instead
                // of a loose substring, so a user file named e.g.
                // `2024_quarantined.notes.txt` is NOT silently skipped.
                // Operator note: `.quarantined.*` siblings are NEVER cleaned
                // up automatically — long-running daemons may need periodic
                // manual `rm` if the inbox dir keeps producing them.
                if path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(is_quarantine_filename)
                {
                    continue;
                }

                // Skip files that are too large
                let metadata = match tokio::fs::metadata(&path).await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if metadata.len() > MAX_FILE_SIZE {
                    warn!(
                        path = %path.display(),
                        size = metadata.len(),
                        "Inbox: skipping file (exceeds 1 MB limit)"
                    );
                    continue;
                }

                // Skip non-text extensions (binary guard)
                if !is_text_file(&path) {
                    debug!(path = %path.display(), "Inbox: skipping non-text file");
                    continue;
                }

                // Read file content
                let content = match tokio::fs::read_to_string(&path).await {
                    Ok(c) => c,
                    Err(e) => {
                        debug!(path = %path.display(), error = %e, "Inbox: skipping unreadable file");
                        continue;
                    }
                };

                if content.trim().is_empty() {
                    // Move empty files to processed without sending.
                    // #3751 — never silently delete the user's file.  If the
                    // move fails (read-only processed dir, disk full, EACCES),
                    // try to rename it in place with a `.quarantined` suffix
                    // so subsequent polls skip it without spinning.  If even
                    // that rename fails, park the path in `in_flight` so we
                    // skip it for the rest of this process lifetime.
                    if let Err(e) = move_to_processed(&path, &processed_dir).await {
                        warn!(
                            path = %path.display(),
                            error = %e,
                            "Inbox: failed to move empty file to processed dir, attempting quarantine rename"
                        );
                        if let Err(e2) = quarantine_in_place(&path).await {
                            warn!(
                                path = %path.display(),
                                error = %e2,
                                "Inbox: quarantine rename also failed; suppressing rescan via in_flight"
                            );
                            in_flight.insert(path.clone());
                        }
                    }
                    continue;
                }

                // Parse agent directive from first line
                let (target_agent, message) = parse_inbox_file(&content, &config);

                let agent_name = match target_agent {
                    Some(name) => name,
                    None => {
                        warn!(
                            path = %path.display(),
                            "Inbox: no target agent (no agent: directive and no default_agent configured)"
                        );
                        continue;
                    }
                };

                // Resolve agent by name
                let agent_entry = kernel.registry.find_by_name(&agent_name);
                let agent_id = match agent_entry {
                    Some(entry) => entry.id,
                    None => {
                        warn!(
                            path = %path.display(),
                            agent = %agent_name,
                            "Inbox: target agent not found in registry"
                        );
                        continue;
                    }
                };

                // Mark as in-flight and dispatch
                in_flight.insert(path.clone());

                let kernel_clone = Arc::clone(&kernel);
                let processed_dir_clone = processed_dir.clone();
                let path_clone = path.clone();
                let file_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                tokio::spawn(async move {
                    let inbox_prompt = format!("[INBOX FILE: {file_name}]\n{message}");

                    info!(
                        agent = %agent_name,
                        file = %file_name,
                        "Inbox: dispatching file to agent"
                    );

                    match kernel_clone.send_message(agent_id, &inbox_prompt).await {
                        Ok(result) => {
                            info!(
                                agent = %agent_name,
                                file = %file_name,
                                response_len = result.response.len(),
                                "Inbox: message delivered"
                            );
                        }
                        Err(e) => {
                            warn!(
                                agent = %agent_name,
                                file = %file_name,
                                error = %e,
                                "Inbox: failed to deliver message"
                            );
                        }
                    }

                    // Move to processed regardless of send result (avoid infinite retry)
                    if let Err(e) = move_to_processed(&path_clone, &processed_dir_clone).await {
                        warn!(
                            path = %path_clone.display(),
                            error = %e,
                            "Inbox: failed to move file to processed"
                        );
                    }
                });
            }

            // Clean up in-flight set: remove entries whose files no longer exist
            // in the inbox directory (they've been moved to processed).
            in_flight.retain(|p| p.exists());
        }
    });
}

/// Parse an inbox file, extracting the optional `agent:` directive and the
/// message body.  Returns `(target_agent_name, message_text)`.
fn parse_inbox_file(content: &str, config: &InboxConfig) -> (Option<String>, String) {
    let mut lines = content.lines();
    if let Some(first_line) = lines.next() {
        let trimmed = first_line.trim();
        if let Some(agent_name) = trimmed
            .strip_prefix("agent:")
            .or_else(|| trimmed.strip_prefix("Agent:"))
            .or_else(|| trimmed.strip_prefix("AGENT:"))
        {
            let agent_name = agent_name.trim().to_string();
            let rest: String = lines.collect::<Vec<_>>().join("\n");
            let message = rest.trim().to_string();
            return (Some(agent_name), message);
        }
    }

    // No directive — use default agent
    (config.default_agent.clone(), content.to_string())
}

/// Move a file to the processed directory, appending a timestamp to avoid
/// collisions.
async fn move_to_processed(src: &Path, processed_dir: &Path) -> std::io::Result<()> {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    let ext = src
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let dest = processed_dir.join(format!("{stem}_{ts}{ext}"));

    tokio::fs::rename(src, &dest).await?;
    debug!(
        from = %src.display(),
        to = %dest.display(),
        "Inbox: moved file to processed"
    );
    Ok(())
}

/// Rename a file in place by appending `.quarantined.<timestamp>` so the inbox
/// poller stops rescanning it without destroying the user's data.
///
/// Used as a fallback when `move_to_processed` fails — a same-directory rename
/// usually succeeds even when the `processed/` subdir is broken.
async fn quarantine_in_place(src: &Path) -> std::io::Result<()> {
    let file_name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "inbox_file".to_string());
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let dest_base = src.with_file_name(format!("{file_name}.quarantined.{ts}"));
    // Collision is unlikely but possible if poll_interval < 1s.  Try the
    // nanosecond-suffix variant; if that also exists, give up and let the
    // caller fall back to the in_flight blocklist rather than silently
    // overwriting a pre-existing quarantine file.
    let dest = if !dest_base.exists() {
        dest_base
    } else {
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dest_nanos = src.with_file_name(format!("{file_name}.quarantined.{ts}.{nanos}"));
        if dest_nanos.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("quarantine target already exists: {}", dest_nanos.display()),
            ));
        }
        dest_nanos
    };
    tokio::fs::rename(src, &dest).await?;
    warn!(
        from = %src.display(),
        to = %dest.display(),
        "Inbox: quarantined file in place to break spin loop"
    );
    Ok(())
}

/// Tight match for the exact suffix shape `quarantine_in_place` produces:
/// `<original>.quarantined.YYYYMMDD_HHMMSS` optionally followed by
/// `.NNNNNNNNNNNNNNNNNNN` (nanosecond tiebreaker on second-collision). This
/// narrower form avoids skipping user files that happen to contain the
/// substring `.quarantined.` for unrelated reasons.
fn is_quarantine_filename(name: &str) -> bool {
    let Some((_, after)) = name.rsplit_once(".quarantined.") else {
        return false;
    };
    // First segment must be 15 chars in `YYYYMMDD_HHMMSS` shape.
    let mut iter = after.splitn(2, '.');
    let ts = iter.next().unwrap_or("");
    if ts.len() != 15 {
        return false;
    }
    let bytes = ts.as_bytes();
    if !(bytes[0..8].iter().all(u8::is_ascii_digit)
        && bytes[8] == b'_'
        && bytes[9..15].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    // Optional trailing `.NNN...` nanos suffix, if present must be all digits.
    match iter.next() {
        None => true,
        Some(nanos) => !nanos.is_empty() && nanos.bytes().all(|b| b.is_ascii_digit()),
    }
}

/// Heuristic to identify text files by extension.
fn is_text_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "txt"
                | "md"
                | "text"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "xml"
                | "html"
                | "htm"
                | "log"
                | "cfg"
                | "ini"
                | "sh"
                | "bash"
                | "py"
                | "rs"
                | "js"
                | "ts"
                | "rb"
                | "go"
                | "java"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "sql"
                | "prompt"
        ),
        // No extension — assume text
        None => true,
    }
}

/// Count text files in a directory (non-recursive).
fn count_text_files(dir: &Path) -> usize {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let path = e.path();
                path.is_file() && is_text_file(&path)
            })
            .count(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_inbox_file_with_agent_directive() {
        let config = InboxConfig {
            default_agent: Some("fallback".to_string()),
            ..Default::default()
        };

        let content = "agent:researcher\nPlease summarize this document.";
        let (agent, msg) = parse_inbox_file(content, &config);
        assert_eq!(agent.as_deref(), Some("researcher"));
        assert_eq!(msg, "Please summarize this document.");
    }

    #[test]
    fn test_parse_inbox_file_case_insensitive_prefix() {
        let config = InboxConfig::default();
        let content = "Agent: my-agent\nHello world";
        let (agent, msg) = parse_inbox_file(content, &config);
        assert_eq!(agent.as_deref(), Some("my-agent"));
        assert_eq!(msg, "Hello world");
    }

    #[test]
    fn test_parse_inbox_file_no_directive_uses_default() {
        let config = InboxConfig {
            default_agent: Some("default-bot".to_string()),
            ..Default::default()
        };

        let content = "Just a regular message\nwith multiple lines";
        let (agent, msg) = parse_inbox_file(content, &config);
        assert_eq!(agent.as_deref(), Some("default-bot"));
        assert_eq!(msg, content);
    }

    #[test]
    fn test_parse_inbox_file_no_directive_no_default() {
        let config = InboxConfig::default();
        let content = "Some message text";
        let (agent, _msg) = parse_inbox_file(content, &config);
        assert!(agent.is_none());
    }

    #[test]
    fn test_is_text_file() {
        assert!(is_text_file(Path::new("hello.txt")));
        assert!(is_text_file(Path::new("script.py")));
        assert!(is_text_file(Path::new("data.json")));
        assert!(is_text_file(Path::new("readme.md")));
        assert!(is_text_file(Path::new("noext")));
        assert!(!is_text_file(Path::new("image.png")));
        assert!(!is_text_file(Path::new("binary.exe")));
        assert!(!is_text_file(Path::new("archive.zip")));
    }

    #[test]
    fn test_count_text_files_nonexistent_dir() {
        assert_eq!(count_text_files(Path::new("/nonexistent/dir")), 0);
    }

    #[test]
    fn test_count_text_files_with_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("b.md"), "world").unwrap();
        std::fs::write(tmp.path().join("c.png"), "binary").unwrap();
        assert_eq!(count_text_files(tmp.path()), 2);
    }

    #[test]
    fn test_resolve_inbox_dir_default() {
        let config = InboxConfig::default();
        let home = PathBuf::from("/home/user/.librefang");
        assert_eq!(resolve_inbox_dir(&config, &home), home.join("inbox"));
    }

    #[test]
    fn test_resolve_inbox_dir_custom() {
        let config = InboxConfig {
            directory: Some("/custom/inbox".to_string()),
            ..Default::default()
        };
        let home = PathBuf::from("/home/user/.librefang");
        assert_eq!(
            resolve_inbox_dir(&config, &home),
            PathBuf::from("/custom/inbox")
        );
    }

    #[test]
    fn test_resolve_inbox_dir_expands_tilde() {
        let config = InboxConfig {
            directory: Some("~/.librefang/inbox".to_string()),
            ..Default::default()
        };
        let home = PathBuf::from("/home/user/.librefang");
        let expected = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".librefang")
            .join("inbox");
        assert_eq!(resolve_inbox_dir(&config, &home), expected);
    }

    #[tokio::test]
    async fn test_quarantine_in_place_renames_file() {
        // #3751 — quarantine fallback must rename rather than delete.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("empty.txt");
        std::fs::write(&src, "").unwrap();

        quarantine_in_place(&src).await.unwrap();

        // Original path is gone.
        assert!(!src.exists(), "src must have been renamed away");

        // A sibling with `.quarantined.` in the name now exists.
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.iter().any(|n| n.contains(".quarantined.")),
            "expected a .quarantined.* sibling, got {entries:?}"
        );
    }

    #[test]
    fn test_is_quarantine_filename_matches_only_real_quarantine_shape() {
        // Real quarantine names from quarantine_in_place — must match.
        assert!(is_quarantine_filename(
            "msg.txt.quarantined.20260101_120000"
        ));
        assert!(is_quarantine_filename(
            "msg.txt.quarantined.20260101_120000.123456789"
        ));
        // Bare files — must NOT match.
        assert!(!is_quarantine_filename("msg.txt"));
        assert!(!is_quarantine_filename("notes.md"));
        // User files that happen to contain the substring — must NOT match
        // (this is the false-positive bug the loose `.contains(...)` had).
        assert!(!is_quarantine_filename("2024_quarantined.notes.txt"));
        assert!(!is_quarantine_filename("a.quarantined.b"));
        assert!(!is_quarantine_filename("a.quarantined.20260101_12000")); // 14 chars, wrong length
        assert!(!is_quarantine_filename("a.quarantined.20260101-120000")); // wrong separator
        assert!(!is_quarantine_filename("a.quarantined.20260101_120000.abc")); // non-numeric nanos
    }

    #[test]
    fn test_inbox_status_default() {
        let config = InboxConfig::default();
        let home = PathBuf::from("/nonexistent");
        let status = inbox_status(&config, &home);
        assert!(!status.enabled);
        assert_eq!(status.pending_count, 0);
        assert_eq!(status.processed_count, 0);
    }
}
