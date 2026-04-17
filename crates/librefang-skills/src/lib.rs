//! Skill system for LibreFang.
//!
//! Skills are pluggable tool bundles that extend agent capabilities.
//! They can be:
//! - TOML + Python scripts
//! - TOML + WASM modules
//! - TOML + Node.js modules (OpenClaw compatibility)
//! - Remote skills from FangHub registry

pub mod clawhub;
pub mod evolution;
pub(crate) mod http_client;
pub mod loader;
pub mod marketplace;
pub mod openclaw_compat;
pub mod publish;
pub mod registry;
pub mod skillhub;
pub mod verify;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Errors from the skill system.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Skill not found: {0}")]
    NotFound(String),
    #[error("Invalid skill manifest: {0}")]
    InvalidManifest(String),
    #[error("Skill already installed: {0}")]
    AlreadyInstalled(String),
    #[error("Runtime not available: {0}")]
    RuntimeNotAvailable(String),
    #[error("Skill execution failed: {0}")]
    ExecutionFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Rate limited by ClawHub — please wait a moment and try again: {0}")]
    RateLimited(String),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("YAML parse error: {0}")]
    YamlParse(String),
    #[error("Security blocked: {0}")]
    SecurityBlocked(String),
}

/// The runtime type for a skill.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillRuntime {
    /// Python script executed in subprocess.
    Python,
    /// WASM module executed in sandbox.
    Wasm,
    /// Node.js module (OpenClaw compatibility).
    Node,
    /// Shell/Bash script executed in subprocess.
    Shell,
    /// Built-in (compiled into the binary).
    Builtin,
    /// Prompt-only skill: injects context into the LLM system prompt.
    /// No executable code — the Markdown body teaches the LLM.
    #[default]
    PromptOnly,
}

/// Provenance tracking for skill origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SkillSource {
    /// Built into LibreFang or manually installed.
    Native,
    /// User-created workspace or local skill.
    Local,
    /// Converted from OpenClaw format.
    OpenClaw,
    /// Downloaded from ClawHub marketplace.
    ClawHub { slug: String, version: String },
    /// Downloaded from Skillhub marketplace.
    Skillhub { slug: String, version: String },
}

/// A tool provided by a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolDef {
    /// Tool name (must be unique).
    pub name: String,
    /// Description shown to LLM.
    pub description: String,
    /// JSON Schema for the tool input.
    pub input_schema: serde_json::Value,
}

/// Requirements declared by a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillRequirements {
    /// Built-in tools this skill needs access to.
    pub tools: Vec<String>,
    /// Capabilities this skill needs from the host.
    pub capabilities: Vec<String>,
}

/// A skill manifest (parsed from skill.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Skill metadata.
    pub skill: SkillMeta,
    /// Runtime configuration (defaults to PromptOnly if omitted).
    #[serde(default)]
    pub runtime: SkillRuntimeConfig,
    /// Tools provided by this skill.
    #[serde(default)]
    pub tools: SkillTools,
    /// Requirements from the host.
    #[serde(default)]
    pub requirements: SkillRequirements,
    /// Markdown body for prompt-only skills (injected into LLM system prompt).
    #[serde(default)]
    pub prompt_context: Option<String>,
    /// Provenance tracking — where this skill came from.
    #[serde(default)]
    pub source: Option<SkillSource>,
    /// Arbitrary user-defined configuration keys.
    ///
    /// Skill authors place custom config under a `[config]` table:
    ///
    /// ```toml
    /// [skill]
    /// name = "my-skill"
    ///
    /// [config]
    /// apiKey = "sk-..."
    /// custom_endpoint = "https://api.example.com"
    /// max_retries = 3
    /// ```
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

/// Skill metadata section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    /// Unique skill name.
    pub name: String,
    /// Semantic version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Author.
    #[serde(default)]
    pub author: String,
    /// License.
    #[serde(default)]
    pub license: String,
    /// Tags for discovery.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// Runtime configuration section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillRuntimeConfig {
    /// Runtime type.
    #[serde(rename = "type", default)]
    pub runtime_type: SkillRuntime,
    /// Entry point file (relative to skill directory).
    #[serde(default)]
    pub entry: String,
}

/// Tools section (wraps provided tools).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillTools {
    /// Tools provided by this skill.
    pub provided: Vec<SkillToolDef>,
}

/// An installed skill in the registry.
#[derive(Debug, Clone)]
pub struct InstalledSkill {
    /// Skill manifest.
    pub manifest: SkillManifest,
    /// Path to skill directory.
    pub path: PathBuf,
    /// Whether this skill is enabled.
    pub enabled: bool,
}

/// Result of executing a skill tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolResult {
    /// Output content.
    pub output: serde_json::Value,
    /// Whether execution was an error.
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_manifest_parse() {
        let toml_str = r#"
[skill]
name = "web-summarizer"
version = "0.1.0"
description = "Summarizes any web page into bullet points"
author = "librefang-community"
license = "MIT"
tags = ["web", "summarizer", "research"]

[runtime]
type = "python"
entry = "src/main.py"

[[tools.provided]]
name = "summarize_url"
description = "Fetch a URL and return a concise bullet-point summary"
input_schema = { type = "object", properties = { url = { type = "string" } }, required = ["url"] }

[requirements]
tools = ["web_fetch"]
capabilities = ["NetConnect(*)"]
"#;

        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "web-summarizer");
        assert_eq!(manifest.runtime.runtime_type, SkillRuntime::Python);
        assert_eq!(manifest.tools.provided.len(), 1);
        assert_eq!(manifest.tools.provided[0].name, "summarize_url");
        assert_eq!(manifest.requirements.tools, vec!["web_fetch"]);
    }

    #[test]
    fn test_skill_runtime_serde() {
        let json = serde_json::to_string(&SkillRuntime::Python).unwrap();
        assert_eq!(json, "\"python\"");

        let rt: SkillRuntime = serde_json::from_str("\"wasm\"").unwrap();
        assert_eq!(rt, SkillRuntime::Wasm);

        let rt: SkillRuntime = serde_json::from_str("\"shell\"").unwrap();
        assert_eq!(rt, SkillRuntime::Shell);

        let json = serde_json::to_string(&SkillRuntime::Shell).unwrap();
        assert_eq!(json, "\"shell\"");

        let rt: SkillRuntime = serde_json::from_str("\"promptonly\"").unwrap();
        assert_eq!(rt, SkillRuntime::PromptOnly);
    }

    #[test]
    fn test_skill_source_serde() {
        let src = SkillSource::ClawHub {
            slug: "github-helper".to_string(),
            version: "1.0.0".to_string(),
        };
        let json = serde_json::to_string(&src).unwrap();
        let back: SkillSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, src);

        let native = SkillSource::Native;
        let json = serde_json::to_string(&native).unwrap();
        let back: SkillSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SkillSource::Native);
    }

    #[test]
    fn test_skill_manifest_parse_shell() {
        let toml_str = r#"
[skill]
name = "disk-cleanup"
version = "0.1.0"
description = "Clean up temporary files"
author = "librefang-community"
license = "MIT"
tags = ["disk", "cleanup", "shell"]

[runtime]
type = "shell"
entry = "cleanup.sh"

[[tools.provided]]
name = "cleanup_tmp"
description = "Remove temporary files older than 7 days"
input_schema = { type = "object", properties = { days = { type = "number" } } }
"#;

        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "disk-cleanup");
        assert_eq!(manifest.runtime.runtime_type, SkillRuntime::Shell);
        assert_eq!(manifest.runtime.entry, "cleanup.sh");
        assert_eq!(manifest.tools.provided.len(), 1);
        assert_eq!(manifest.tools.provided[0].name, "cleanup_tmp");
    }

    #[test]
    fn test_skill_manifest_extra_config_keys() {
        let toml_str = r#"
[skill]
name = "my-custom-skill"
version = "1.0.0"
description = "A skill with custom config"

[runtime]
type = "python"
entry = "main.py"

[config]
apiKey = "sk-test-123"
custom_endpoint = "https://api.example.com"
max_retries = 3
nested_config = { timeout = 30, retries = 5 }
"#;

        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "my-custom-skill");
        assert_eq!(manifest.config.len(), 4);
        assert_eq!(
            manifest.config.get("apiKey").and_then(|v| v.as_str()),
            Some("sk-test-123")
        );
        assert_eq!(
            manifest
                .config
                .get("custom_endpoint")
                .and_then(|v| v.as_str()),
            Some("https://api.example.com")
        );
        assert_eq!(
            manifest.config.get("max_retries").and_then(|v| v.as_i64()),
            Some(3)
        );
        assert!(manifest.config.get("nested_config").unwrap().is_object());
    }

    #[test]
    fn test_skill_manifest_no_extra_keys() {
        let toml_str = r#"
[skill]
name = "plain-skill"
version = "0.1.0"
description = "No extra config"
"#;

        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "plain-skill");
        assert!(manifest.config.is_empty());
    }

    #[test]
    fn test_skill_manifest_extra_roundtrip() {
        let toml_str = r#"
[skill]
name = "roundtrip-skill"
version = "1.0.0"
description = "Test serialization roundtrip"

[config]
custom_key = "custom_value"
"#;

        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.config.len(), 1);

        // Serialize back and verify the extra key is preserved
        let serialized = toml::to_string(&manifest).unwrap();
        let reparsed: SkillManifest = toml::from_str(&serialized).unwrap();
        assert_eq!(
            reparsed.config.get("custom_key").and_then(|v| v.as_str()),
            Some("custom_value")
        );
    }
}
