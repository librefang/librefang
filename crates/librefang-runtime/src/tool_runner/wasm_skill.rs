//! WASM skill execution — bridges a `SkillRuntime::Wasm` skill to the
//! in-process [`WasmSandbox`](crate::sandbox::WasmSandbox).
//!
//! Why this lives in `librefang-runtime` and not `librefang-skills`: the
//! sandbox (capability gating, fuel/memory/wall-clock metering, the
//! `host_call` ABI) lives in this crate, and the host calls need a
//! [`KernelHandle`]. `librefang-skills` must not depend on
//! `librefang-runtime` (that would be circular), so the skills loader
//! returns `RuntimeNotAvailable` for `Wasm` and the live dispatch path
//! routes here instead.
//!
//! The guest receives the same envelope the subprocess runtimes use —
//! `{"tool": <name>, "input": <input>[, "config": <skill config>]}` — so a
//! skill's tool dispatch is identical regardless of runtime kind. See the
//! sandbox module doc-comment for the required guest ABI (`memory`,
//! `alloc`, `execute`, and the optional `librefang` host imports).

use crate::sandbox::{SandboxConfig, WasmSandbox};
use librefang_kernel_handle::prelude::*;
use librefang_skills::{SkillError, SkillManifest, SkillToolResult};
use librefang_types::capability::Capability;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, warn};

/// Parse a manifest capability string (e.g. `"NetConnect(*)"`, `"ToolAll"`)
/// into a [`Capability`].
///
/// Fail-closed: an unrecognised or malformed string returns `None` so the
/// caller drops it rather than granting an unintended permission. The string
/// form mirrors the enum's `Variant(value)` shape declared in
/// `skill.toml` under `[requirements] capabilities = [...]`.
fn parse_capability(s: &str) -> Option<Capability> {
    use Capability::*;
    let s = s.trim();
    let (name, arg) = match s.split_once('(') {
        Some((name, rest)) => (name.trim(), Some(rest.strip_suffix(')')?.trim())),
        None => (s, None),
    };
    Some(match (name, arg) {
        ("FileRead", Some(a)) => FileRead(a.to_string()),
        ("FileWrite", Some(a)) => FileWrite(a.to_string()),
        ("NetConnect", Some(a)) => NetConnect(a.to_string()),
        ("NetListen", Some(a)) => NetListen(a.parse().ok()?),
        ("ToolInvoke", Some(a)) => ToolInvoke(a.to_string()),
        ("ToolAll", None) => ToolAll,
        ("LlmQuery", Some(a)) => LlmQuery(a.to_string()),
        ("LlmMaxTokens", Some(a)) => LlmMaxTokens(a.parse().ok()?),
        ("AgentSpawn", None) => AgentSpawn,
        ("AgentMessage", Some(a)) => AgentMessage(a.to_string()),
        ("AgentKill", Some(a)) => AgentKill(a.to_string()),
        ("MemoryRead", Some(a)) => MemoryRead(a.to_string()),
        ("MemoryWrite", Some(a)) => MemoryWrite(a.to_string()),
        ("ShellExec", Some(a)) => ShellExec(a.to_string()),
        ("EnvRead", Some(a)) => EnvRead(a.to_string()),
        ("OfpDiscover", None) => OfpDiscover,
        ("OfpConnect", Some(a)) => OfpConnect(a.to_string()),
        ("OfpAdvertise", None) => OfpAdvertise,
        ("EconSpend", Some(a)) => EconSpend(a.parse().ok()?),
        ("EconEarn", None) => EconEarn,
        ("EconTransfer", Some(a)) => EconTransfer(a.to_string()),
        _ => return None,
    })
}

/// Map a skill's declared `[requirements] capabilities` to sandbox grants.
///
/// Unparseable entries are logged and skipped (deny-by-default): a typo in a
/// capability string fails closed to "not granted" rather than silently
/// widening access.
fn resolve_capabilities(manifest: &SkillManifest) -> Vec<Capability> {
    manifest
        .requirements
        .capabilities
        .iter()
        .filter_map(|raw| match parse_capability(raw) {
            Some(cap) => Some(cap),
            None => {
                warn!(
                    capability = raw.as_str(),
                    skill = %manifest.skill.name,
                    "unrecognized capability string; not granting to WASM sandbox"
                );
                None
            }
        })
        .collect()
}

/// Execute a `SkillRuntime::Wasm` skill tool inside the sandbox.
///
/// `skill_dir` is the installed skill's directory; `manifest.runtime.entry`
/// names the `.wasm` (or `.wat`) module relative to it. The module is run
/// with the capabilities the manifest declares and the manifest's
/// `timeout_secs` (falling back to the sandbox default when unset).
///
/// Public so the CLI (`librefang skill test`) can run a WASM skill outside the
/// kernel by passing `kernel = None` — pure-compute skills run; capability-
/// bearing host calls return an error rather than crashing.
pub async fn execute_wasm_skill(
    manifest: &SkillManifest,
    skill_dir: &Path,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<Arc<dyn KernelHandle>>,
    agent_id: &str,
) -> Result<SkillToolResult, SkillError> {
    // SECURITY: identical path-containment guard as the subprocess runtimes —
    // rejects `../` traversal out of the skill directory before any read.
    let module_path =
        librefang_skills::loader::validate_script_path(skill_dir, &manifest.runtime.entry)?;
    let wasm_bytes = tokio::fs::read(&module_path).await.map_err(|e| {
        SkillError::ExecutionFailed(format!(
            "WASM module not readable ({}): {e}",
            module_path.display()
        ))
    })?;

    // Mirror the subprocess envelope so guest tool-dispatch is runtime-agnostic.
    let payload = if manifest.config.is_empty() {
        serde_json::json!({ "tool": tool_name, "input": input })
    } else {
        serde_json::json!({ "tool": tool_name, "input": input, "config": &manifest.config })
    };

    let config = SandboxConfig {
        // Clamp to the same ceiling the subprocess runtimes enforce
        // (MAX_SKILL_TIMEOUT_SECS) so an installed manifest cannot request an
        // unbounded wall-clock budget. Treat 0 as unset (→ sandbox's own 30s
        // default) rather than a 0s instant timeout. None → sandbox default.
        timeout_secs: manifest
            .requirements
            .timeout_secs
            .filter(|&t| t != 0)
            .map(|t| t.min(librefang_skills::loader::MAX_SKILL_TIMEOUT_SECS)),
        capabilities: resolve_capabilities(manifest),
        ..Default::default()
    };

    debug!(
        skill = %manifest.skill.name,
        tool = tool_name,
        caps = config.capabilities.len(),
        "executing WASM skill in sandbox"
    );

    // `execute` builds its own per-invocation engine (epoch isolation requires
    // one engine per guest), so construct the marker directly rather than via
    // `WasmSandbox::new()`, whose only effect here would be a second, discarded
    // engine init. A bad engine config still surfaces — `execute` reports it as
    // `SandboxError::Compilation`.
    let result = WasmSandbox
        .execute(&wasm_bytes, payload, config, kernel, agent_id)
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("WASM execution failed: {e}")))?;

    // The WASM SDK turns a handler `Err` into a *successful* `execute` that
    // returns `{"error": ...}` (mirroring how a non-zero subprocess exit
    // surfaces for the Python/Node runtimes). Detect that envelope so the
    // shared dispatch arm reports it to the agent as an error rather than a
    // success — keeping `is_error` consistent across all skill runtimes.
    let is_error = result
        .output
        .get("error")
        .is_some_and(|v| !v.is_null());

    Ok(SkillToolResult {
        output: result.output,
        is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_capability_covers_arg_and_no_arg_variants() {
        assert_eq!(
            parse_capability("NetConnect(*)"),
            Some(Capability::NetConnect("*".to_string()))
        );
        assert_eq!(
            parse_capability("FileRead(/tmp/*)"),
            Some(Capability::FileRead("/tmp/*".to_string()))
        );
        assert_eq!(parse_capability("ToolAll"), Some(Capability::ToolAll));
        assert_eq!(parse_capability("AgentSpawn"), Some(Capability::AgentSpawn));
        assert_eq!(
            parse_capability("NetListen(8080)"),
            Some(Capability::NetListen(8080))
        );
        assert_eq!(
            parse_capability("EconSpend(1.5)"),
            Some(Capability::EconSpend(1.5))
        );
        // Whitespace tolerance.
        assert_eq!(
            parse_capability("  ShellExec( ls* ) "),
            Some(Capability::ShellExec("ls*".to_string()))
        );
    }

    #[test]
    fn parse_capability_fails_closed_on_garbage() {
        // Unknown variant, malformed parens, and wrong-arity all yield None
        // so the caller never grants an unintended capability.
        assert_eq!(parse_capability("Nonsense(x)"), None);
        assert_eq!(parse_capability("NetConnect("), None);
        assert_eq!(parse_capability("NetListen(not-a-port)"), None);
        assert_eq!(parse_capability("ToolAll(x)"), None);
        assert_eq!(parse_capability("FileRead"), None);
        assert_eq!(parse_capability(""), None);
    }

    /// Minimal echo module: returns the input JSON envelope unchanged. Proves
    /// the runtime wiring (path resolve → read → sandbox → result) end to end
    /// without needing host imports. `Module::new` accepts `.wat` text, so we
    /// can ship the module inline.
    const ECHO_WAT: &str = r#"
        (module
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))
            (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr))
            (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
                (i64.or
                    (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                    (i64.extend_i32_u (local.get $len)))))
    "#;

    fn wasm_manifest(entry: &str) -> SkillManifest {
        let toml_str = format!(
            r#"
[skill]
name = "echo-wasm"

[runtime]
type = "wasm"
entry = "{entry}"
"#
        );
        toml::from_str(&toml_str).expect("manifest parses")
    }

    #[test]
    fn resolve_capabilities_keeps_known_and_drops_unrecognized() {
        // The fail-closed contract at the integration point: a manifest with a
        // mix of valid and garbage capability strings yields exactly the valid
        // grants, in declaration order, with the garbage silently dropped (it
        // is WARN-logged at runtime) — never granted.
        let manifest: SkillManifest = toml::from_str(
            r#"
[skill]
name = "caps-wasm"

[runtime]
type = "wasm"
entry = "skill.wasm"

[requirements]
capabilities = ["NetConnect(*)", "definitely-not-a-cap", "ToolAll", "NetListen(bad)"]
"#,
        )
        .expect("manifest parses");

        assert_eq!(
            resolve_capabilities(&manifest),
            vec![Capability::NetConnect("*".to_string()), Capability::ToolAll]
        );
    }

    #[tokio::test]
    async fn echo_wasm_skill_runs_in_sandbox_and_returns_envelope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skill.wasm"), ECHO_WAT).unwrap();
        let manifest = wasm_manifest("skill.wasm");
        let input = serde_json::json!({"q": "hello"});

        let result =
            execute_wasm_skill(&manifest, dir.path(), "do_echo", &input, None, "test-agent")
                .await
                .expect("wasm skill executes");

        assert!(!result.is_error);
        // Echo returns the full envelope the host fed the guest.
        assert_eq!(result.output["tool"], "do_echo");
        assert_eq!(result.output["input"], input);
    }

    // A module that ignores its input and returns the SDK error envelope
    // `{"error":"boom"}` (16 bytes) preloaded at offset 1024.
    const ERROR_WAT: &str = r#"
        (module
            (memory (export "memory") 1)
            (data (i32.const 1024) "{\"error\":\"boom\"}")
            (global $bump (mut i32) (i32.const 2048))
            (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr))
            (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
                (i64.or
                    (i64.shl (i64.extend_i32_u (i32.const 1024)) (i64.const 32))
                    (i64.extend_i32_u (i32.const 16)))))
    "#;

    #[tokio::test]
    async fn wasm_skill_error_envelope_sets_is_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skill.wasm"), ERROR_WAT).unwrap();
        let manifest = wasm_manifest("skill.wasm");

        let result = execute_wasm_skill(
            &manifest,
            dir.path(),
            "do_fail",
            &serde_json::json!({}),
            None,
            "test-agent",
        )
        .await
        .expect("wasm skill executes (error is in-band, not an Err)");

        // A guest that returns `{"error": ...}` must surface as a tool error,
        // matching how the subprocess runtimes report a non-zero exit.
        assert!(result.is_error, "error envelope must set is_error = true");
        assert_eq!(result.output["error"], "boom");
    }

    #[tokio::test]
    async fn wasm_skill_rejects_path_traversal_entry() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = wasm_manifest("../escape.wasm");
        let err = execute_wasm_skill(
            &manifest,
            dir.path(),
            "t",
            &serde_json::json!({}),
            None,
            "test-agent",
        )
        .await
        .expect_err("traversal must be rejected");
        assert!(
            matches!(err, SkillError::ExecutionFailed(_)),
            "expected ExecutionFailed, got {err:?}"
        );
    }
}
