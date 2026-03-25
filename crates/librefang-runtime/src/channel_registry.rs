//! Load channel metadata from `~/.librefang/channels/*.toml` (synced from the
//! librefang-registry). Provides structured data (name, description, icon,
//! i18n, docs URL) that the API can serve to the Dashboard.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Metadata for a single communication channel, parsed from a registry TOML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub i18n: HashMap<String, ChannelMetaI18n>,
    #[serde(default)]
    pub metadata: Option<ChannelMetaExtras>,
}

/// Localised description for a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMetaI18n {
    #[serde(default)]
    pub description: String,
}

/// Optional extra links (project URL, documentation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMetaExtras {
    pub url: Option<String>,
    pub docs: Option<String>,
}

/// Read all `*.toml` files from `channels_dir` and parse each into a [`ChannelMeta`].
///
/// Files that fail to parse are logged and skipped (never fatal).
pub fn load_channel_metadata(channels_dir: &Path) -> Vec<ChannelMeta> {
    let entries = match std::fs::read_dir(channels_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".toml") || !path.is_file() {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read channel file {}: {e}", path.display());
                continue;
            }
        };

        match toml::from_str::<ChannelMeta>(&content) {
            Ok(meta) => result.push(meta),
            Err(e) => {
                tracing::warn!("Failed to parse channel file {}: {e}", path.display());
            }
        }
    }

    // Stable sort by id for deterministic API output.
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_channel_metadata_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_channel_metadata(tmp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_channel_metadata_nonexistent_dir() {
        let result = load_channel_metadata(Path::new("/tmp/nonexistent_channel_dir_12345"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_channel_metadata_parses_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
id = "slack"
name = "Slack"
description = "Slack workspace integration"
category = "messaging"
tags = ["chat", "enterprise"]
icon = "slack-icon.svg"
protocol = "websocket"

[i18n.zh]
description = "Slack 工作区集成"

[metadata]
url = "https://slack.com"
docs = "https://api.slack.com/docs"
"#;
        std::fs::write(tmp.path().join("slack.toml"), toml_content).unwrap();

        // Also write a non-toml file that should be ignored
        std::fs::write(tmp.path().join("readme.txt"), "ignore me").unwrap();

        let result = load_channel_metadata(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "slack");
        assert_eq!(result[0].name, "Slack");
        assert_eq!(result[0].category, "messaging");
        assert_eq!(result[0].tags, vec!["chat", "enterprise"]);
        assert_eq!(result[0].icon, "slack-icon.svg");
        assert_eq!(result[0].protocol, "websocket");
        assert_eq!(
            result[0].i18n.get("zh").unwrap().description,
            "Slack 工作区集成"
        );
        let meta = result[0].metadata.as_ref().unwrap();
        assert_eq!(meta.url.as_deref(), Some("https://slack.com"));
        assert_eq!(meta.docs.as_deref(), Some("https://api.slack.com/docs"));
    }

    #[test]
    fn test_load_channel_metadata_skips_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        // Valid file
        std::fs::write(
            tmp.path().join("valid.toml"),
            "id = \"valid\"\nname = \"Valid\"",
        )
        .unwrap();
        // Invalid TOML (missing required fields — name is not present)
        std::fs::write(tmp.path().join("bad.toml"), "not_valid = [[[").unwrap();

        let result = load_channel_metadata(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "valid");
    }

    #[test]
    fn test_load_channel_metadata_sorted_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("z_channel.toml"),
            "id = \"zebra\"\nname = \"Zebra\"",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("a_channel.toml"),
            "id = \"alpha\"\nname = \"Alpha\"",
        )
        .unwrap();

        let result = load_channel_metadata(tmp.path());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "alpha");
        assert_eq!(result[1].id, "zebra");
    }
}
