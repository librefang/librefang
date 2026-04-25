//! Map a tool name (and optional definition) to a [`ToolApprovalClass`].
//!
//! This is a passive helper — it only computes a class, it does not gate
//! execution. The approval ladder will consume it in a follow-up PR.
//!
//! Resolution order:
//! 1. If the supplied [`ToolDefinition::input_schema`] carries an
//!    `x-tool-class` extension key (a JSON-Schema vendor extension) whose
//!    value is a known snake_case identifier, that wins. We also accept the
//!    same key nested under a top-level `metadata` object so future tool
//!    schemas can group bookkeeping fields without breaking the classifier.
//! 2. Otherwise we pattern-match the tool name against a hand-curated list.
//! 3. Anything else falls through to [`ToolApprovalClass::Unknown`].
//!
//! This module also exposes [`ParallelSafety`] and [`parallel_safety`], a
//! projection from `ToolApprovalClass` used by the agent loop's batch
//! dispatcher to decide which calls in a single assistant turn can run
//! concurrently. It's defined here (next to `classify_tool`) because the
//! two computations always travel together.

use librefang_types::tool::ToolDefinition;
use librefang_types::tool_class::ToolApprovalClass;
use serde::{Deserialize, Serialize};

/// Classify a tool by name, honoring an explicit `x-tool-class` annotation
/// inside the definition's `input_schema` when present.
pub fn classify_tool(name: &str, definition: Option<&ToolDefinition>) -> ToolApprovalClass {
    if let Some(def) = definition {
        if let Some(explicit) = explicit_class_from_schema(&def.input_schema) {
            return explicit;
        }
    }
    classify_by_name(name)
}

fn classify_by_name(name: &str) -> ToolApprovalClass {
    match name {
        "file_read" | "glob" | "grep" | "ls" | "cat" => ToolApprovalClass::ReadonlyScoped,
        "web_search" | "web_fetch" => ToolApprovalClass::ReadonlySearch,
        "file_write" | "file_edit" | "apply_patch" => ToolApprovalClass::Mutating,
        "shell_exec" | "python_exec" | "exec" => ToolApprovalClass::ExecCapable,
        "config_set" | "agent_spawn" | "agent_kill" | "kernel_reload" => {
            ToolApprovalClass::ControlPlane
        }
        "approval_request" | "totp_request" => ToolApprovalClass::Interactive,
        _ => ToolApprovalClass::Unknown,
    }
}

/// Parallel-execution safety class for a tool call.
///
/// This is a projection of [`ToolApprovalClass`] specialised for the agent
/// loop's batch dispatcher. The dispatcher uses it to decide which calls
/// in a single assistant turn can run concurrently and which must serialise.
///
/// Variants are ordered from most-permissive to least-permissive: any
/// future scheduler that wants a single ordering can take `as_u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelSafety {
    /// No observable side effects on shared state. Safe to run alongside
    /// any peer in the same batch.
    ReadOnly,
    /// Mutates shared state but the mutation is scoped to a path (or
    /// virtual namespace) that can be projected from the call's input.
    /// Safe to run with peers whose scope does not overlap.
    WriteScoped,
    /// Mutates shared state with no clean scope projection. Must run as
    /// the only call in its bucket — peers run before or after, never
    /// concurrently with it.
    WriteShared,
    /// Requires user interaction or has cross-cutting effects (approval
    /// flows, control-plane mutations). Forces the entire batch to
    /// serialise, since concurrent peers could observe partial state.
    Exclusive,
}

impl ParallelSafety {
    /// Snake-case identifier matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WriteScoped => "write_scoped",
            Self::WriteShared => "write_shared",
            Self::Exclusive => "exclusive",
        }
    }

    /// Parse a snake_case identifier back into a safety class.
    ///
    /// Used by [`parallel_safety`] to honour explicit annotations on a
    /// tool's input schema (e.g. `metadata.parallel_safety = "read_only"`).
    pub fn from_snake_case(s: &str) -> Option<Self> {
        match s {
            "read_only" => Some(Self::ReadOnly),
            "write_scoped" => Some(Self::WriteScoped),
            "write_shared" => Some(Self::WriteShared),
            "exclusive" => Some(Self::Exclusive),
            _ => None,
        }
    }

    /// Project a [`ToolApprovalClass`] onto a parallel-safety class.
    ///
    /// Mapping rationale:
    /// - Read-only classes (scoped + search) are fully parallelisable.
    /// - `Mutating` is path-scoped (`file_write`, `apply_patch`, …) and
    ///   can run with peers when paths don't overlap; the dispatcher
    ///   does the overlap check.
    /// - `ExecCapable` (`shell_exec`, `python_exec`, …) has unbounded
    ///   side effects: a shell command's working directory and FS
    ///   reach cannot be inferred from args, so we serialise it.
    /// - `Interactive` and `ControlPlane` are exclusive — interactive
    ///   tools need user attention, control-plane mutations affect
    ///   every other call's environment.
    /// - `Unknown` is conservatively `WriteShared` (never blocks the
    ///   batch entirely, but never runs alongside anything either).
    pub const fn from_approval_class(class: ToolApprovalClass) -> Self {
        match class {
            ToolApprovalClass::ReadonlyScoped | ToolApprovalClass::ReadonlySearch => Self::ReadOnly,
            ToolApprovalClass::Mutating => Self::WriteScoped,
            ToolApprovalClass::ExecCapable => Self::WriteShared,
            ToolApprovalClass::Interactive | ToolApprovalClass::ControlPlane => Self::Exclusive,
            ToolApprovalClass::Unknown => Self::WriteShared,
        }
    }
}

/// Compute the parallel-safety class for a tool call.
///
/// Resolution order:
/// 1. Explicit `metadata.parallel_safety` (or top-level
///    `x-parallel-safety`) on the tool's input schema, if it parses to a
///    known [`ParallelSafety`] variant. This is the escape hatch for MCP
///    servers and plugin authors who know more than name-level heuristics.
/// 2. Otherwise, project the [`ToolApprovalClass`] computed by
///    [`classify_tool`] via [`ParallelSafety::from_approval_class`].
pub fn parallel_safety(name: &str, definition: Option<&ToolDefinition>) -> ParallelSafety {
    if let Some(def) = definition {
        if let Some(explicit) = explicit_parallel_safety_from_schema(&def.input_schema) {
            return explicit;
        }
    }
    ParallelSafety::from_approval_class(classify_tool(name, definition))
}

/// Look for an explicit parallel-safety annotation in a tool's input schema.
///
/// Accepts either:
/// - top-level `"x-parallel-safety": "<snake_case>"`, or
/// - `"metadata": { "parallel_safety": "<snake_case>" }`.
///
/// Unknown values fall through (caller will use the projection path), so
/// a typo in the annotation never poisons the result.
fn explicit_parallel_safety_from_schema(schema: &serde_json::Value) -> Option<ParallelSafety> {
    let obj = schema.as_object()?;

    if let Some(s) = obj.get("x-parallel-safety").and_then(|v| v.as_str()) {
        if let Some(c) = ParallelSafety::from_snake_case(s) {
            return Some(c);
        }
    }

    if let Some(meta) = obj.get("metadata").and_then(|v| v.as_object()) {
        if let Some(s) = meta.get("parallel_safety").and_then(|v| v.as_str()) {
            if let Some(c) = ParallelSafety::from_snake_case(s) {
                return Some(c);
            }
        }
    }

    None
}

/// Look for an explicit class annotation in a tool's input schema.
///
/// Accepts either:
/// - top-level `"x-tool-class": "<snake_case>"`, or
/// - `"metadata": { "tool_class": "<snake_case>" }`
fn explicit_class_from_schema(schema: &serde_json::Value) -> Option<ToolApprovalClass> {
    let obj = schema.as_object()?;

    if let Some(s) = obj.get("x-tool-class").and_then(|v| v.as_str()) {
        if let Some(c) = ToolApprovalClass::from_snake_case(s) {
            return Some(c);
        }
    }

    if let Some(meta) = obj.get("metadata").and_then(|v| v.as_object()) {
        if let Some(s) = meta.get("tool_class").and_then(|v| v.as_str()) {
            if let Some(c) = ToolApprovalClass::from_snake_case(s) {
                return Some(c);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_with_schema(schema: serde_json::Value) -> ToolDefinition {
        ToolDefinition {
            name: "file_read".to_string(),
            description: "test".to_string(),
            input_schema: schema,
        }
    }

    #[test]
    fn readonly_scoped_names() {
        for n in ["file_read", "glob", "grep", "ls", "cat"] {
            assert_eq!(
                classify_tool(n, None),
                ToolApprovalClass::ReadonlyScoped,
                "{n} should be ReadonlyScoped"
            );
        }
    }

    #[test]
    fn readonly_search_names() {
        for n in ["web_search", "web_fetch"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::ReadonlySearch);
        }
    }

    #[test]
    fn mutating_names() {
        for n in ["file_write", "file_edit", "apply_patch"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::Mutating);
        }
    }

    #[test]
    fn exec_capable_names() {
        for n in ["shell_exec", "python_exec", "exec"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::ExecCapable);
        }
    }

    #[test]
    fn control_plane_names() {
        for n in ["config_set", "agent_spawn", "agent_kill", "kernel_reload"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::ControlPlane);
        }
    }

    #[test]
    fn interactive_names() {
        for n in ["approval_request", "totp_request"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::Interactive);
        }
    }

    #[test]
    fn unknown_falls_through() {
        assert_eq!(
            classify_tool("brand_new_tool", None),
            ToolApprovalClass::Unknown
        );
    }

    #[test]
    fn classify_file_read_without_definition() {
        assert_eq!(
            classify_tool("file_read", None),
            ToolApprovalClass::ReadonlyScoped
        );
    }

    #[test]
    fn explicit_metadata_overrides_name_heuristic() {
        // file_read would normally be ReadonlyScoped, but the explicit
        // annotation must win.
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "metadata": { "tool_class": "exec_capable" }
        }));
        assert_eq!(
            classify_tool("file_read", Some(&def)),
            ToolApprovalClass::ExecCapable
        );
    }

    #[test]
    fn explicit_x_tool_class_overrides_name_heuristic() {
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "x-tool-class": "control_plane"
        }));
        assert_eq!(
            classify_tool("file_read", Some(&def)),
            ToolApprovalClass::ControlPlane
        );
    }

    #[test]
    fn unknown_explicit_value_falls_back_to_name() {
        // Unrecognized annotation must not poison the result — fall back
        // to the name-based heuristic.
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "x-tool-class": "totally_made_up"
        }));
        assert_eq!(
            classify_tool("file_read", Some(&def)),
            ToolApprovalClass::ReadonlyScoped
        );
    }

    #[test]
    fn definition_without_annotation_uses_name() {
        let def = def_with_schema(serde_json::json!({"type": "object"}));
        assert_eq!(
            classify_tool("shell_exec", Some(&def)),
            ToolApprovalClass::ExecCapable
        );
    }

    #[test]
    fn severity_rank_ordering_spot_check() {
        // Mirrors the spec: ReadonlyScoped < ExecCapable < Interactive < Unknown.
        let scoped = classify_tool("file_read", None).severity_rank();
        let exec = classify_tool("shell_exec", None).severity_rank();
        let interactive = classify_tool("totp_request", None).severity_rank();
        let unknown = classify_tool("???", None).severity_rank();
        assert!(scoped < exec);
        assert!(exec < interactive);
        assert!(interactive < unknown);
    }

    #[test]
    fn serde_round_trip_readonly_scoped() {
        let json = serde_json::to_string(&ToolApprovalClass::ReadonlyScoped).unwrap();
        assert_eq!(json, "\"readonly_scoped\"");
        let back: ToolApprovalClass = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ToolApprovalClass::ReadonlyScoped);
    }

    // ---- ParallelSafety projection tests ----

    /// All read-only classes project to `ReadOnly` so reads can fan out
    /// freely within a batch.
    #[test]
    fn parallel_safety_reads_are_readonly() {
        for n in [
            "file_read",
            "glob",
            "grep",
            "ls",
            "cat",
            "web_search",
            "web_fetch",
        ] {
            assert_eq!(
                parallel_safety(n, None),
                ParallelSafety::ReadOnly,
                "{n} should be ReadOnly"
            );
        }
    }

    /// Path-scoped writers project to `WriteScoped` — the dispatcher will
    /// run them alongside peers when their target paths don't overlap.
    #[test]
    fn parallel_safety_writers_are_write_scoped() {
        for n in ["file_write", "file_edit", "apply_patch"] {
            assert_eq!(
                parallel_safety(n, None),
                ParallelSafety::WriteScoped,
                "{n} should be WriteScoped"
            );
        }
    }

    /// Exec-capable tools are `WriteShared`: a shell command's reach
    /// cannot be inferred from its args, so it must own its bucket.
    #[test]
    fn parallel_safety_shell_is_write_shared() {
        for n in ["shell_exec", "python_exec", "exec"] {
            assert_eq!(
                parallel_safety(n, None),
                ParallelSafety::WriteShared,
                "{n} should be WriteShared"
            );
        }
    }

    /// Interactive and control-plane tools force the whole batch to
    /// serialise — concurrent peers could observe partial state.
    #[test]
    fn parallel_safety_interactive_and_control_plane_are_exclusive() {
        for n in [
            "approval_request",
            "totp_request",
            "config_set",
            "agent_spawn",
            "agent_kill",
            "kernel_reload",
        ] {
            assert_eq!(
                parallel_safety(n, None),
                ParallelSafety::Exclusive,
                "{n} should be Exclusive"
            );
        }
    }

    /// Unclassified tools default to `WriteShared` — they never run
    /// alongside peers, but they don't force the entire batch to
    /// serialise either. Conservative without being punitive.
    #[test]
    fn parallel_safety_unknown_defaults_to_write_shared() {
        assert_eq!(
            parallel_safety("brand_new_tool", None),
            ParallelSafety::WriteShared,
        );
    }

    /// Explicit `metadata.parallel_safety` overrides the name-based
    /// projection. This is the escape hatch for MCP / plugin authors.
    #[test]
    fn parallel_safety_metadata_override_wins() {
        // `shell_exec` would normally be WriteShared, but a deliberately
        // sandboxed wrapper can opt in to ReadOnly.
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "metadata": { "parallel_safety": "read_only" }
        }));
        assert_eq!(
            parallel_safety("shell_exec", Some(&def)),
            ParallelSafety::ReadOnly,
        );
    }

    /// Top-level `x-parallel-safety` works the same way as the nested
    /// metadata form.
    #[test]
    fn parallel_safety_x_extension_override_wins() {
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "x-parallel-safety": "exclusive"
        }));
        assert_eq!(
            parallel_safety("file_read", Some(&def)),
            ParallelSafety::Exclusive,
        );
    }

    /// An unrecognised override value must fall through to the projection
    /// path rather than poisoning the result.
    #[test]
    fn parallel_safety_unknown_override_falls_back_to_projection() {
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "x-parallel-safety": "totally_made_up"
        }));
        assert_eq!(
            parallel_safety("file_read", Some(&def)),
            ParallelSafety::ReadOnly,
        );
    }

    /// `from_approval_class` is `const fn` so callers (including future
    /// match-arm tables) can use it in const contexts. Spot-check the
    /// projection at every variant so the table stays exhaustive.
    #[test]
    fn from_approval_class_covers_every_variant() {
        use ToolApprovalClass::*;
        assert_eq!(
            ParallelSafety::from_approval_class(ReadonlyScoped),
            ParallelSafety::ReadOnly
        );
        assert_eq!(
            ParallelSafety::from_approval_class(ReadonlySearch),
            ParallelSafety::ReadOnly
        );
        assert_eq!(
            ParallelSafety::from_approval_class(Mutating),
            ParallelSafety::WriteScoped
        );
        assert_eq!(
            ParallelSafety::from_approval_class(ExecCapable),
            ParallelSafety::WriteShared
        );
        assert_eq!(
            ParallelSafety::from_approval_class(ControlPlane),
            ParallelSafety::Exclusive
        );
        assert_eq!(
            ParallelSafety::from_approval_class(Interactive),
            ParallelSafety::Exclusive
        );
        assert_eq!(
            ParallelSafety::from_approval_class(Unknown),
            ParallelSafety::WriteShared
        );
    }

    #[test]
    fn parallel_safety_serde_round_trip() {
        for v in [
            ParallelSafety::ReadOnly,
            ParallelSafety::WriteScoped,
            ParallelSafety::WriteShared,
            ParallelSafety::Exclusive,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let back: ParallelSafety = serde_json::from_str(&json).unwrap();
            assert_eq!(back, v);
            // as_str and the serde wire form must agree.
            assert_eq!(json, format!("\"{}\"", v.as_str()));
        }
    }
}
