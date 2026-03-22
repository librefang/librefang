//! Subagent context types for workflow session management with context inheritance.
//!
//! When a workflow executes steps across multiple agents, each subagent can
//! receive context from previous steps via [`SubagentContext`]. This enables
//! downstream agents to make informed decisions based on upstream results
//! without requiring full session sharing.

use serde::{Deserialize, Serialize};

/// Maximum length (in bytes) for a single step output preview.
/// Longer outputs are truncated to this limit to avoid bloating prompts.
const MAX_OUTPUT_PREVIEW_BYTES: usize = 500;

/// Context passed to a subagent during workflow execution.
///
/// Contains a summary of the parent workflow's progress so far,
/// enabling the subagent to make context-aware decisions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentContext {
    /// Name of the parent agent that orchestrates this workflow (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_name: Option<String>,

    /// Free-form summary of the parent session (e.g., goal description).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_summary: Option<String>,

    /// Name of the workflow being executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,

    /// Zero-based index of the current step in the workflow.
    #[serde(default)]
    pub step_index: usize,

    /// Outputs from previous steps: `(step_name, output_preview)`.
    /// Output previews are truncated to [`MAX_OUTPUT_PREVIEW_BYTES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_outputs: Vec<(String, String)>,
}

impl SubagentContext {
    /// Build a context preamble string suitable for prepending to a prompt.
    ///
    /// Returns `None` if there is no meaningful context to inject (i.e. this
    /// is the first step with no previous outputs and no session summary).
    pub fn format_preamble(&self) -> Option<String> {
        // Nothing to inject for the very first step with no summary
        if self.previous_outputs.is_empty() && self.parent_session_summary.is_none() {
            return None;
        }

        let mut parts = Vec::new();
        parts.push("[Parent workflow context]".to_string());

        if let Some(ref wf) = self.workflow_name {
            parts.push(format!("Workflow: {wf}"));
        }

        if let Some(ref summary) = self.parent_session_summary {
            parts.push(format!("Session summary: {summary}"));
        }

        if !self.previous_outputs.is_empty() {
            parts.push("Previous steps completed:".to_string());
            for (name, preview) in &self.previous_outputs {
                parts.push(format!("- {name}: {preview}"));
            }
        }

        parts.push(String::new()); // trailing blank line before actual prompt
        Some(parts.join("\n"))
    }

    /// Create a truncated preview of a step output, safe for prompt injection.
    pub fn truncate_output_preview(output: &str) -> String {
        let trimmed = output.trim();
        if trimmed.len() <= MAX_OUTPUT_PREVIEW_BYTES {
            return trimmed.to_string();
        }
        let truncated = crate::truncate_str(trimmed, MAX_OUTPUT_PREVIEW_BYTES);
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_context_returns_none() {
        let ctx = SubagentContext::default();
        assert!(ctx.format_preamble().is_none());
    }

    #[test]
    fn first_step_with_summary_returns_preamble() {
        let ctx = SubagentContext {
            parent_session_summary: Some("Analyze Q4 data".to_string()),
            workflow_name: Some("quarterly-report".to_string()),
            ..Default::default()
        };
        let preamble = ctx.format_preamble().unwrap();
        assert!(preamble.contains("[Parent workflow context]"));
        assert!(preamble.contains("Workflow: quarterly-report"));
        assert!(preamble.contains("Session summary: Analyze Q4 data"));
        assert!(!preamble.contains("Previous steps"));
    }

    #[test]
    fn context_with_previous_outputs() {
        let ctx = SubagentContext {
            workflow_name: Some("test-pipeline".to_string()),
            step_index: 2,
            previous_outputs: vec![
                ("analyze".to_string(), "Found 3 issues".to_string()),
                ("triage".to_string(), "Priority: high".to_string()),
            ],
            ..Default::default()
        };
        let preamble = ctx.format_preamble().unwrap();
        assert!(preamble.contains("Previous steps completed:"));
        assert!(preamble.contains("- analyze: Found 3 issues"));
        assert!(preamble.contains("- triage: Priority: high"));
    }

    #[test]
    fn truncate_output_preview_short() {
        let short = "Hello world";
        assert_eq!(SubagentContext::truncate_output_preview(short), "Hello world");
    }

    #[test]
    fn truncate_output_preview_long() {
        let long = "a".repeat(1000);
        let preview = SubagentContext::truncate_output_preview(&long);
        assert!(preview.len() <= MAX_OUTPUT_PREVIEW_BYTES + 3); // +3 for "..."
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn truncate_output_preview_trims_whitespace() {
        let padded = "  hello  ";
        assert_eq!(SubagentContext::truncate_output_preview(padded), "hello");
    }

    #[test]
    fn serde_round_trip() {
        let ctx = SubagentContext {
            parent_agent_name: Some("orchestrator".to_string()),
            parent_session_summary: Some("Summarize data".to_string()),
            workflow_name: Some("pipeline".to_string()),
            step_index: 1,
            previous_outputs: vec![("step-1".to_string(), "done".to_string())],
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let back: SubagentContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back.step_index, 1);
        assert_eq!(back.previous_outputs.len(), 1);
        assert_eq!(back.workflow_name, Some("pipeline".to_string()));
    }

    #[test]
    fn context_disabled_produces_no_preamble() {
        // When inherit_parent_context is false, the caller simply doesn't
        // build a SubagentContext. Verify default produces None.
        let ctx = SubagentContext {
            step_index: 3,
            ..Default::default()
        };
        assert!(ctx.format_preamble().is_none());
    }
}
