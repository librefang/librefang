//! Lightweight UAR-AGENT-MD Markdown → [`AgentArtifact`] parser.
//!
//! Understands the canonical 15-section format defined by the UAR
//! specification without requiring a running UAR instance or the full
//! 8-stage compilation pipeline.
//!
//! # Supported input formats
//! - `.uar.md` — UAR-AGENT-MD specification document
//! - `.agent.json` — pre-compiled descriptor (round-trip via [`parse_json`])
//!
//! # Parsing strategy
//! The parser walks the Markdown line-by-line, treating `## <Heading>` lines
//! as section boundaries and extracting `key: value` pairs within each
//! section (indented YAML-like syntax used by UAR-AGENT-MD). This deliberately
//! avoids a full YAML/TOML parser to remain dependency-light.

use crate::error::{Result, UarSpecError};
use crate::types::{
    A2ASection, AgentArtifact, BudgetsSection, CapabilitiesSection, ConversationMemory,
    ExecutionSection, IdentitySection, McpServerRef, McpServersSection, MemorySection,
    MetadataSection, PersistentMemory, SkillRef, SkillsSection, ToolRef, ToolsSection,
};
use std::collections::HashMap;

/// Parse a UAR-AGENT-MD Markdown string into an [`AgentArtifact`].
///
/// Returns [`UarSpecError::MissingAgentHeading`] if the document does not
/// begin with a `# Agent: <name>` heading, and
/// [`UarSpecError::MissingRequiredSection`] for `## Metadata` or
/// `## Identity` sections that are absent.
pub fn parse(markdown: &str) -> Result<AgentArtifact> {
    let doc = Document::from_str(markdown)?;

    let metadata = parse_metadata(&doc)?;
    let identity = parse_identity(&doc)?;
    let capabilities = parse_capabilities(&doc);
    let skills = parse_skills(&doc);
    let tools = parse_tools(&doc);
    let mcp_servers = parse_mcp_servers(&doc);
    let memory = parse_memory(&doc);
    let a2a = parse_a2a(&doc);
    let budgets = parse_budgets(&doc);
    let execution = parse_execution(&doc);

    Ok(AgentArtifact {
        schema: "uar-agent-descriptor/v1".to_owned(),
        agent_id: identity.name.clone(),
        version: metadata.version.clone(),
        metadata,
        identity,
        capabilities,
        skills,
        tools,
        mcp_servers,
        memory,
        a2a,
        budgets,
        execution,
        extensions: HashMap::new(),
    })
}

/// Deserialize a pre-compiled `.agent.json` descriptor into an [`AgentArtifact`].
pub fn parse_json(json: &str) -> Result<AgentArtifact> {
    Ok(serde_json::from_str(json)?)
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal document model
// ─────────────────────────────────────────────────────────────────────────────

/// A parsed view of the Markdown document, split into named sections.
struct Document {
    /// Agent name extracted from `# Agent: <name>`.
    pub agent_name: String,
    /// Sections keyed by lowercased heading (e.g. `"metadata"`, `"identity"`).
    pub sections: HashMap<String, Vec<String>>,
}

impl Document {
    fn from_str(markdown: &str) -> Result<Self> {
        let mut agent_name = None;
        let mut sections: HashMap<String, Vec<String>> = HashMap::new();
        let mut current_section: Option<String> = None;

        for line in markdown.lines() {
            // H1 heading — agent name
            if let Some(rest) = line.strip_prefix("# Agent:") {
                agent_name = Some(rest.trim().to_owned());
                current_section = None;
                continue;
            }
            // H2 heading — section boundary
            if let Some(rest) = line.strip_prefix("## ") {
                let key = rest.trim().to_lowercase();
                current_section = Some(key.clone());
                sections.entry(key).or_default();
                continue;
            }
            // Accumulate body lines into the current section
            if let Some(ref key) = current_section {
                sections
                    .entry(key.clone())
                    .or_default()
                    .push(line.to_owned());
            }
        }

        Ok(Self {
            agent_name: agent_name.ok_or(UarSpecError::MissingAgentHeading)?,
            sections,
        })
    }

    /// Return the body lines of a section, or an empty slice.
    fn section(&self, name: &str) -> &[String] {
        self.sections.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Extract `key: value` pairs from a section.
    fn kv(&self, section: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for line in self.section(section) {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim().to_lowercase();
                let val = v.trim().to_owned();
                if !key.is_empty() && !val.is_empty() {
                    map.insert(key, val);
                }
            }
        }
        map
    }

    /// Collect lines that start with `-` or `*` as a list.
    fn list(&self, section: &str) -> Vec<String> {
        self.section(section)
            .iter()
            .filter_map(|l| {
                let t = l.trim();
                t.strip_prefix("- ")
                    .or_else(|| t.strip_prefix("* "))
                    .map(str::to_owned)
            })
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Section parsers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_metadata(doc: &Document) -> Result<MetadataSection> {
    let kv = doc.kv("metadata");
    if kv.is_empty() && !doc.sections.contains_key("metadata") {
        return Err(UarSpecError::MissingRequiredSection {
            section: "Metadata",
        });
    }
    let version = kv
        .get("version")
        .cloned()
        .unwrap_or_else(|| "0.1.0".to_owned());

    let tags = kv
        .get("tags")
        .map(|t| t.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();

    Ok(MetadataSection {
        version,
        description: kv.get("description").cloned(),
        author: kv.get("author").cloned(),
        license: kv.get("license").cloned(),
        tags,
        created: kv.get("created").cloned(),
        updated: kv.get("updated").cloned(),
    })
}

fn parse_identity(doc: &Document) -> Result<IdentitySection> {
    let kv = doc.kv("identity");
    if kv.is_empty() && !doc.sections.contains_key("identity") {
        return Err(UarSpecError::MissingRequiredSection {
            section: "Identity",
        });
    }

    // `name` falls back to the document-level agent name if not in the section.
    let name = kv
        .get("name")
        .cloned()
        .unwrap_or_else(|| doc.agent_name.clone());
    let role = kv.get("role").cloned().unwrap_or_default();

    // Persona may be a multi-line paragraph after the `persona:` key; collect
    // remaining non-kv lines as a fallback.
    let persona = kv.get("persona").cloned().unwrap_or_else(|| {
        doc.section("identity")
            .iter()
            .filter(|l| !l.contains(':') && !l.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    });

    Ok(IdentitySection {
        name,
        role,
        persona,
        system_prompt: kv.get("system_prompt").cloned(),
        greeting: kv.get("greeting").cloned(),
    })
}

fn parse_capabilities(doc: &Document) -> CapabilitiesSection {
    let kv = doc.kv("capabilities");
    let bool_flag = |key: &str| -> bool {
        kv.get(key)
            .map(|v| matches!(v.to_lowercase().as_str(), "true" | "yes" | "1"))
            .unwrap_or(false)
    };
    CapabilitiesSection {
        streaming: bool_flag("streaming"),
        file_upload: bool_flag("file_upload"),
        image_generation: bool_flag("image_generation"),
        code_execution: bool_flag("code_execution"),
        web_browsing: bool_flag("web_browsing"),
        model: kv.get("model").cloned(),
        extensions: HashMap::new(),
    }
}

fn parse_skills(doc: &Document) -> SkillsSection {
    let skills = doc
        .list("skills")
        .into_iter()
        .map(|id| SkillRef {
            id,
            version: None,
            required: false,
            config: HashMap::new(),
        })
        .collect();
    SkillsSection { skills }
}

fn parse_tools(doc: &Document) -> ToolsSection {
    // Look for `allow:` / `deny:` kv first, then fall through to list items.
    let kv = doc.kv("tools");
    let allow = kv
        .get("allow")
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();
    let deny = kv
        .get("deny")
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();
    let tools = doc
        .list("tools")
        .into_iter()
        .filter(|name| !name.starts_with("allow:") && !name.starts_with("deny:"))
        .map(|name| ToolRef {
            name,
            server: None,
            required: false,
            config: HashMap::new(),
        })
        .collect();
    ToolsSection { tools, allow, deny }
}

fn parse_mcp_servers(doc: &Document) -> McpServersSection {
    // Each MCP server entry is a `- id: url` list item or a `### id` sub-block.
    // Support the simple `- id url` format for now.
    let servers = doc
        .list("mcp servers")
        .into_iter()
        .chain(doc.list("mcp_servers"))
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let id = parts.next()?.to_owned();
            let url = parts.next().unwrap_or("").to_owned();
            if id.is_empty() {
                return None;
            }
            Some(McpServerRef {
                id,
                url,
                transport: None,
                tools: Vec::new(),
                env: HashMap::new(),
            })
        })
        .collect();
    McpServersSection { servers }
}

fn parse_memory(doc: &Document) -> MemorySection {
    let kv_conv = doc.kv("memory model");
    let kv_mem = doc.kv("memory");
    let kv = if !kv_conv.is_empty() { kv_conv } else { kv_mem };

    let bool_flag = |key: &str| -> bool {
        kv.get(key)
            .map(|v| matches!(v.to_lowercase().as_str(), "true" | "yes" | "1"))
            .unwrap_or(false)
    };

    let conversation_enabled = kv
        .get("conversation")
        .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "no" | "0"))
        .unwrap_or(true);

    let max_turns = kv.get("max_turns").and_then(|v| v.parse::<u32>().ok());

    let persistent_enabled = bool_flag("persistent");
    let persistent = persistent_enabled.then(|| PersistentMemory {
        enabled: true,
        backend: kv.get("backend").cloned(),
    });

    MemorySection {
        conversation: ConversationMemory {
            enabled: conversation_enabled,
            max_turns,
            summarization: bool_flag("summarization"),
        },
        persistent,
    }
}

fn parse_a2a(doc: &Document) -> A2ASection {
    // Simple: list items as endpoint IDs.
    let endpoints = doc
        .list("a2a contracts")
        .into_iter()
        .chain(doc.list("a2a"))
        .map(|id| crate::types::A2AEndpoint {
            id,
            method: None,
            description: None,
            input_schema: None,
            output_schema: None,
        })
        .collect();
    A2ASection {
        endpoints,
        dependencies: Vec::new(),
    }
}

fn parse_budgets(doc: &Document) -> BudgetsSection {
    let kv = {
        let a = doc.kv("budgets & constraints");
        if !a.is_empty() {
            a
        } else {
            doc.kv("budgets")
        }
    };
    let parse_u64 = |key: &str| kv.get(key).and_then(|v| v.parse::<u64>().ok());
    let parse_u32 = |key: &str| kv.get(key).and_then(|v| v.parse::<u32>().ok());
    let parse_f64 = |key: &str| kv.get(key).and_then(|v| v.parse::<f64>().ok());
    BudgetsSection {
        max_tokens_per_turn: parse_u64("max_tokens_per_turn"),
        max_tokens_per_session: parse_u64("max_tokens_per_session"),
        max_tool_calls_per_turn: parse_u32("max_tool_calls_per_turn"),
        max_cost_per_session_usd: parse_f64("max_cost_per_session_usd"),
        timeout_seconds: parse_u64("timeout_seconds"),
    }
}

fn parse_execution(doc: &Document) -> ExecutionSection {
    let kv = {
        let a = doc.kv("execution model");
        if !a.is_empty() {
            a
        } else {
            doc.kv("execution")
        }
    };
    ExecutionSection {
        mode: kv.get("mode").cloned(),
        max_iterations: kv.get("max_iterations").and_then(|v| v.parse::<u32>().ok()),
        stop_conditions: kv
            .get("stop_conditions")
            .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
            .unwrap_or_default(),
        fallback_behavior: kv.get("fallback_behavior").cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_DOC: &str = r#"
# Agent: TestBot

## Metadata
version: 1.0.0
description: A simple test agent.
author: test
tags: test, demo

## Identity
name: TestBot
role: A helpful assistant
persona: I am a friendly assistant that helps with questions.
model: openai/gpt-4o-mini

## Capabilities
streaming: true
web_browsing: false
"#;

    #[test]
    fn parse_minimal_document() {
        let artifact = parse(MINIMAL_DOC).expect("parse should succeed");
        assert_eq!(artifact.agent_id, "TestBot");
        assert_eq!(artifact.version, "1.0.0");
        assert_eq!(artifact.identity.role, "A helpful assistant");
        assert!(artifact.capabilities.streaming);
        assert!(!artifact.capabilities.web_browsing);
        assert_eq!(artifact.metadata.tags, vec!["test", "demo"]);
    }

    #[test]
    fn missing_agent_heading_returns_error() {
        let doc = "## Metadata\nversion: 1.0.0\n";
        assert!(matches!(parse(doc), Err(UarSpecError::MissingAgentHeading)));
    }

    #[test]
    fn roundtrip_json() {
        let artifact = parse(MINIMAL_DOC).unwrap();
        let json = serde_json::to_string(&artifact).unwrap();
        let back = parse_json(&json).unwrap();
        assert_eq!(artifact.agent_id, back.agent_id);
    }
}
