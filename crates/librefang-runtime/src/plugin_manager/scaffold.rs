//! Plugin scaffolding: creates the directory layout, default manifest, and
//! per-language hook templates for a new plugin. Split out of plugin_manager
//! to keep the long template-string constants from inflating the parent module.

use super::*;

/// Create a scaffold for a new plugin. `runtime` defaults to `"python"`;
/// pass `"v"` / `"node"` / `"go"` / `"deno"` / `"native"` to generate a
/// template for that language instead.
pub fn scaffold_plugin(
    name: &str,
    description: &str,
    runtime: Option<&str>,
) -> Result<PathBuf, String> {
    validate_plugin_name(name)?;
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;
    let plugin_dir = plugins.join(name);

    if plugin_dir.exists() {
        return Err(format!("Plugin '{name}' already exists"));
    }

    let hooks_dir = plugin_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Failed to create plugin directory: {e}"))?;

    // Normalize the runtime tag via PluginRuntime so aliases (py/js/golang/...)
    // resolve the same way the hook dispatcher will at runtime.
    let runtime_kind = crate::plugin_runtime::PluginRuntime::from_tag(runtime);
    let runtime_tag = runtime_kind.label();

    // Each runtime declares its own hook filenames + template body so the
    // manifest + files stay in sync.
    let files = hook_templates(runtime_kind.clone());
    let (ingest_file, ingest_body) = files.ingest;
    let (after_file, after_body) = files.after_turn;
    let (assemble_file, assemble_body) = files.assemble;
    let (compact_file, compact_body) = files.compact;
    let (bootstrap_file, bootstrap_body) = files.bootstrap;
    let (prepare_file, prepare_body) = files.prepare_subagent;
    let (merge_file, merge_body) = files.merge_subagent;

    // Write plugin.toml as a hand-crafted string so we can include comments
    // that guide users toward the new hook slots.
    let runtime_line = if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
        String::new()
    } else {
        format!("runtime = \"{runtime_tag}\"\n")
    };
    let requirements_line = if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python)
    {
        "requirements = \"requirements.txt\"\n".to_string()
    } else {
        String::new()
    };
    let manifest_toml = format!(
        r#"name = "{name}"
version = "0.1.0"
description = "{description}"
# librefang_min_version = "2026.4.0"   # refuse to load on older daemons
{runtime_line}
# hook_timeout_secs = 30   # per-invocation timeout; bootstrap gets 2× this value
# max_retries       = 0    # retry hook on failure (0 = no retry)
# retry_delay_ms    = 500  # wait between retries
# on_hook_failure   = "warn"   # "warn" | "abort" | "skip"

[hooks]
# --- Active hooks ---
ingest    = "hooks/{ingest_file}"
after_turn = "hooks/{after_file}"

# ingest_filter = "remember"  # only run ingest when message contains this string

# --- Optional hooks (uncomment to activate; template files already written) ---
# bootstrap        = "hooks/{bootstrap_file}"   # runs once at startup (2× timeout)
# assemble         = "hooks/{assemble_file}"    # control what the LLM sees (powerful)
# compact          = "hooks/{compact_file}"     # custom context compression
# prepare_subagent = "hooks/{prepare_file}"     # called before sub-agent spawns
# merge_subagent   = "hooks/{merge_file}"       # called after sub-agent completes

# [env]
# MY_SERVICE_URL = "http://localhost:6333"
# MY_API_KEY     = "${{MY_API_KEY}}"   # expanded from daemon environment at runtime
{requirements_line}"#,
        name = name,
        description = description,
        ingest_file = ingest_file,
        after_file = after_file,
        bootstrap_file = bootstrap_file,
        assemble_file = assemble_file,
        compact_file = compact_file,
        prepare_file = prepare_file,
        merge_file = merge_file,
        runtime_line = runtime_line,
        requirements_line = requirements_line,
    );
    std::fs::write(plugin_dir.join("plugin.toml"), manifest_toml)
        .map_err(|e| format!("Failed to write plugin.toml: {e}"))?;

    let ingest_path = hooks_dir.join(ingest_file);
    let after_path = hooks_dir.join(after_file);
    let assemble_path = hooks_dir.join(assemble_file);
    let compact_path = hooks_dir.join(compact_file);
    let bootstrap_path = hooks_dir.join(bootstrap_file);
    let prepare_path = hooks_dir.join(prepare_file);
    let merge_path = hooks_dir.join(merge_file);
    std::fs::write(&ingest_path, ingest_body)
        .map_err(|e| format!("Failed to write {ingest_file}: {e}"))?;
    std::fs::write(&after_path, after_body)
        .map_err(|e| format!("Failed to write {after_file}: {e}"))?;
    std::fs::write(&assemble_path, assemble_body)
        .map_err(|e| format!("Failed to write {assemble_file}: {e}"))?;
    std::fs::write(&compact_path, compact_body)
        .map_err(|e| format!("Failed to write {compact_file}: {e}"))?;
    std::fs::write(&bootstrap_path, bootstrap_body)
        .map_err(|e| format!("Failed to write {bootstrap_file}: {e}"))?;
    // prepare_subagent and merge_subagent may share the same template body;
    // write them to distinct files so users can customise them independently.
    std::fs::write(&prepare_path, prepare_body)
        .map_err(|e| format!("Failed to write {prepare_file}: {e}"))?;
    std::fs::write(&merge_path, merge_body)
        .map_err(|e| format!("Failed to write {merge_file}: {e}"))?;

    // Native plugins exec the file directly, so the scaffolded shell wrapper
    // needs the executable bit. No-op on Windows (which uses extension-based
    // execution) and on other runtimes (interpreter handles execution).
    if runtime_kind.requires_executable_bit() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [
                &ingest_path,
                &after_path,
                &assemble_path,
                &compact_path,
                &bootstrap_path,
                &prepare_path,
                &merge_path,
            ] {
                if let Ok(meta) = std::fs::metadata(path) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(path, perms);
                }
            }
        }
    }

    // Python plugins get requirements.txt; other runtimes manage deps
    // their own way (go.mod, package.json, v.mod, ...).
    if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
        std::fs::write(
            plugin_dir.join("requirements.txt"),
            "# Python dependencies\n",
        )
        .map_err(|e| format!("Failed to write requirements.txt: {e}"))?;
    }

    info!(
        plugin = name,
        runtime = runtime_tag.as_ref(),
        "Scaffolded new plugin"
    );
    Ok(plugin_dir)
}

/// All hook file names and template bodies for a given runtime.
struct HookFiles {
    /// `(filename, template_body)` pairs for each hook.
    ingest: (&'static str, &'static str),
    after_turn: (&'static str, &'static str),
    assemble: (&'static str, &'static str),
    compact: (&'static str, &'static str),
    /// One-shot startup hook (connect to vector DB, warm cache, etc.)
    bootstrap: (&'static str, &'static str),
    /// Called before a sub-agent spawns.
    prepare_subagent: (&'static str, &'static str),
    /// Called after a sub-agent completes.
    merge_subagent: (&'static str, &'static str),
}

/// Return scaffolded hook filenames + body content for a given runtime.
///
/// Each hook gets a working template showing the stdin/stdout protocol.
/// Python, Node, Go, and Deno get full implementations with token-budget
/// logic; other runtimes get minimal no-op stubs with protocol comments.
fn hook_templates(runtime: crate::plugin_runtime::PluginRuntime) -> HookFiles {
    use crate::plugin_runtime::PluginRuntime as R;
    match runtime {
        R::Python => HookFiles {
            ingest: ("ingest.py", PY_INGEST),
            after_turn: ("after_turn.py", PY_AFTER_TURN),
            assemble: ("assemble.py", PY_ASSEMBLE),
            compact: ("compact.py", PY_COMPACT),
            bootstrap: ("bootstrap.py", PY_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.py", PY_PREPARE_SUBAGENT),
            merge_subagent: ("merge_subagent.py", PY_MERGE_SUBAGENT),
        },
        R::Node => HookFiles {
            ingest: ("ingest.js", NODE_INGEST),
            after_turn: ("after_turn.js", NODE_AFTER_TURN),
            assemble: ("assemble.js", NODE_ASSEMBLE),
            compact: ("compact.js", NODE_COMPACT),
            bootstrap: ("bootstrap.js", NODE_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.js", STUB_BOOTSTRAP_NODE),
            merge_subagent: ("merge_subagent.js", STUB_BOOTSTRAP_NODE),
        },
        R::Deno => HookFiles {
            ingest: ("ingest.ts", DENO_INGEST),
            after_turn: ("after_turn.ts", DENO_AFTER_TURN),
            assemble: ("assemble.ts", DENO_ASSEMBLE),
            compact: ("compact.ts", DENO_COMPACT),
            bootstrap: ("bootstrap.ts", DENO_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.ts", STUB_LIFECYCLE_DENO),
            merge_subagent: ("merge_subagent.ts", STUB_LIFECYCLE_DENO),
        },
        R::Go => HookFiles {
            ingest: ("ingest.go", GO_INGEST),
            after_turn: ("after_turn.go", GO_AFTER_TURN),
            assemble: ("assemble.go", GO_ASSEMBLE),
            compact: ("compact.go", GO_COMPACT),
            bootstrap: ("bootstrap.go", GO_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.go", STUB_LIFECYCLE_GO),
            merge_subagent: ("merge_subagent.go", STUB_LIFECYCLE_GO),
        },
        R::V => HookFiles {
            ingest: ("ingest.v", V_INGEST),
            after_turn: ("after_turn.v", V_AFTER_TURN),
            assemble: ("assemble.v", STUB_ASSEMBLE_V),
            compact: ("compact.v", STUB_COMPACT_V),
            bootstrap: ("bootstrap.v", STUB_LIFECYCLE_V),
            prepare_subagent: ("prepare_subagent.v", STUB_LIFECYCLE_V),
            merge_subagent: ("merge_subagent.v", STUB_LIFECYCLE_V),
        },
        R::Ruby => HookFiles {
            ingest: ("ingest.rb", RUBY_INGEST),
            after_turn: ("after_turn.rb", RUBY_AFTER_TURN),
            assemble: ("assemble.rb", STUB_ASSEMBLE_RUBY),
            compact: ("compact.rb", STUB_COMPACT_RUBY),
            bootstrap: ("bootstrap.rb", STUB_LIFECYCLE_RUBY),
            prepare_subagent: ("prepare_subagent.rb", STUB_LIFECYCLE_RUBY),
            merge_subagent: ("merge_subagent.rb", STUB_LIFECYCLE_RUBY),
        },
        R::Bash => HookFiles {
            ingest: ("ingest.sh", BASH_INGEST),
            after_turn: ("after_turn.sh", BASH_AFTER_TURN),
            assemble: ("assemble.sh", STUB_ASSEMBLE_BASH),
            compact: ("compact.sh", STUB_COMPACT_BASH),
            bootstrap: ("bootstrap.sh", STUB_LIFECYCLE_BASH),
            prepare_subagent: ("prepare_subagent.sh", STUB_LIFECYCLE_BASH),
            merge_subagent: ("merge_subagent.sh", STUB_LIFECYCLE_BASH),
        },
        R::Bun => HookFiles {
            ingest: ("ingest.ts", BUN_INGEST),
            after_turn: ("after_turn.ts", BUN_AFTER_TURN),
            assemble: ("assemble.ts", STUB_ASSEMBLE_BUN),
            compact: ("compact.ts", STUB_COMPACT_BUN),
            bootstrap: ("bootstrap.ts", STUB_LIFECYCLE_BUN),
            prepare_subagent: ("prepare_subagent.ts", STUB_LIFECYCLE_BUN),
            merge_subagent: ("merge_subagent.ts", STUB_LIFECYCLE_BUN),
        },
        R::Php => HookFiles {
            ingest: ("ingest.php", PHP_INGEST),
            after_turn: ("after_turn.php", PHP_AFTER_TURN),
            assemble: ("assemble.php", STUB_ASSEMBLE_PHP),
            compact: ("compact.php", STUB_COMPACT_PHP),
            bootstrap: ("bootstrap.php", STUB_LIFECYCLE_PHP),
            prepare_subagent: ("prepare_subagent.php", STUB_LIFECYCLE_PHP),
            merge_subagent: ("merge_subagent.php", STUB_LIFECYCLE_PHP),
        },
        R::Lua => HookFiles {
            ingest: ("ingest.lua", LUA_INGEST),
            after_turn: ("after_turn.lua", LUA_AFTER_TURN),
            assemble: ("assemble.lua", STUB_ASSEMBLE_LUA),
            compact: ("compact.lua", STUB_COMPACT_LUA),
            bootstrap: ("bootstrap.lua", STUB_LIFECYCLE_LUA),
            prepare_subagent: ("prepare_subagent.lua", STUB_LIFECYCLE_LUA),
            merge_subagent: ("merge_subagent.lua", STUB_LIFECYCLE_LUA),
        },
        R::Native => HookFiles {
            // Shell wrapper — users replace with a real pre-compiled binary.
            ingest: ("ingest", NATIVE_INGEST),
            after_turn: ("after_turn", NATIVE_AFTER_TURN),
            assemble: ("assemble", STUB_ASSEMBLE_NATIVE),
            compact: ("compact", STUB_COMPACT_NATIVE),
            bootstrap: ("bootstrap", STUB_LIFECYCLE_NATIVE),
            prepare_subagent: ("prepare_subagent", STUB_LIFECYCLE_NATIVE),
            merge_subagent: ("merge_subagent", STUB_LIFECYCLE_NATIVE),
        },
        R::Wasm => HookFiles {
            // Wasm hooks run inline via wasmtime — no template files needed.
            // Scaffold stubs so the directory structure is consistent.
            ingest: ("ingest.wasm", NATIVE_INGEST),
            after_turn: ("after_turn.wasm", NATIVE_AFTER_TURN),
            assemble: ("assemble.wasm", STUB_ASSEMBLE_NATIVE),
            compact: ("compact.wasm", STUB_COMPACT_NATIVE),
            bootstrap: ("bootstrap.wasm", STUB_LIFECYCLE_NATIVE),
            prepare_subagent: ("prepare_subagent.wasm", STUB_LIFECYCLE_NATIVE),
            merge_subagent: ("merge_subagent.wasm", STUB_LIFECYCLE_NATIVE),
        },
        // Custom launchers: fall back to the native (shell-wrapper) templates.
        // Users will replace these with scripts suitable for their launcher.
        R::Custom(_) => HookFiles {
            ingest: ("ingest", NATIVE_INGEST),
            after_turn: ("after_turn", NATIVE_AFTER_TURN),
            assemble: ("assemble", STUB_ASSEMBLE_NATIVE),
            compact: ("compact", STUB_COMPACT_NATIVE),
            bootstrap: ("bootstrap", STUB_LIFECYCLE_NATIVE),
            prepare_subagent: ("prepare_subagent", STUB_LIFECYCLE_NATIVE),
            merge_subagent: ("merge_subagent", STUB_LIFECYCLE_NATIVE),
        },
    }
}

// --- Python templates (the original, kept verbatim for backwards compat) ---

const PY_INGEST: &str = r#"#!/usr/bin/env python3
"""Context engine ingest hook.

Receives via stdin:
    {
      "type": "ingest",
      "agent_id": "...",
      "message": "user message text",
      "peer_id": "platform-user-id-or-null"
    }

Should print to stdout:
    {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}

Tip: scope your recall to peer_id when present to prevent cross-user leaks.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    message = request["message"]
    peer_id = request.get("peer_id")  # None when called directly via API

    # TODO: Implement your custom recall logic here.
    # Example: query a vector database, search a knowledge base, etc.
    memories = []

    print(json.dumps({"type": "ingest_result", "memories": memories}))

if __name__ == "__main__":
    main()
"#;

const PY_AFTER_TURN: &str = r#"#!/usr/bin/env python3
"""Context engine after_turn hook.

Receives via stdin:
    {
      "type": "after_turn",
      "agent_id": "...",
      "messages": [{"role": "user"|"assistant", "content": "...", "pinned": false}, ...]
    }

Note: message content is truncated to 500 chars per message for performance.

Should print to stdout:
    {"type": "ok"}
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    messages = request["messages"]

    # TODO: Implement your post-turn logic here.
    # Example: update indexes, persist state, log analytics, etc.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_ASSEMBLE: &str = r#"#!/usr/bin/env python3
"""Context engine assemble hook — controls what the LLM sees.

This is the most powerful hook. Called before every LLM request.

Receives via stdin:
    {
      "type": "assemble",
      "system_prompt": "...",
      "messages": [
        {"role": "user"|"assistant"|"tool", "content": <text or blocks>, "pinned": false},
        ...
      ],
      "context_window_tokens": 200000
    }

Messages use the full LibreFang message format — content can be a plain string
or a list of blocks (text, tool_use, tool_result, image, thinking).

Should print to stdout:
    {"type": "assemble_result", "messages": [...]}

Return a trimmed/reordered subset of messages that fits the token budget.
If you return an empty list or fail, LibreFang falls back to its default
overflow recovery (trim oldest, then compact).
"""
import json
import sys

def estimate_tokens(text: str) -> int:
    """Rough token estimate: ~4 chars per token."""
    return max(1, len(text) // 4)

def message_text(msg: dict) -> str:
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            b.get("text", b.get("content", ""))
            for b in content
            if isinstance(b, dict)
        )
    return ""

def main():
    request = json.loads(sys.stdin.read())
    messages = request["messages"]
    context_window_tokens = request["context_window_tokens"]

    # Reserve tokens for system prompt and response headroom
    budget = context_window_tokens - 4000

    # Keep messages newest-first until we exceed the budget, then stop
    kept = []
    used = 0
    for msg in reversed(messages):
        tokens = estimate_tokens(message_text(msg))
        if used + tokens > budget:
            break
        kept.append(msg)
        used += tokens

    kept.reverse()
    print(json.dumps({"type": "assemble_result", "messages": kept}))

if __name__ == "__main__":
    main()
"#;

const PY_COMPACT: &str = r#"#!/usr/bin/env python3
"""Context engine compact hook — custom context compression.

Called when the context window is under pressure.

Receives via stdin:
    {
      "type": "compact",
      "agent_id": "...",
      "messages": [...],   # full message list (same format as assemble)
      "model": "llama-3.3-70b-versatile",
      "context_window_tokens": 200000
    }

Should print to stdout:
    {"type": "compact_result", "messages": [...]}

Return a compacted version of the message list. If you fail or return
an empty list, LibreFang falls back to its built-in LLM-based compaction.
"""
import json
import sys

def message_text(msg: dict) -> str:
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            b.get("text", b.get("content", ""))
            for b in content
            if isinstance(b, dict)
        )
    return ""

def main():
    request = json.loads(sys.stdin.read())
    messages = request["messages"]

    # Simple strategy: keep the first (system/context) message and the last 10
    pinned = [m for m in messages if m.get("pinned")]
    rest = [m for m in messages if not m.get("pinned")]

    summary_text = "... (older messages summarized) ..."
    summary_msg = {"role": "assistant", "content": summary_text, "pinned": False}

    if len(rest) > 10:
        compacted = pinned + [summary_msg] + rest[-10:]
    else:
        compacted = pinned + rest

    print(json.dumps({"type": "compact_result", "messages": compacted}))

if __name__ == "__main__":
    main()
"#;

// --- Python lifecycle hooks (bootstrap / prepare_subagent / merge_subagent) ---

const PY_BOOTSTRAP: &str = r#"#!/usr/bin/env python3
"""Context engine bootstrap hook — runs ONCE when the engine initialises.

Use this to connect to external services (vector databases, caches, HTTP APIs)
and warm any state your other hooks will read at runtime.

Receives via stdin:
    {
      "type": "bootstrap",
      "context_window_tokens": 200000,
      "stable_prefix_mode": false,
      "max_recall_results": 10
    }

Should print to stdout:
    {"type": "ok"}

Failures here are non-fatal — the engine continues without your bootstrap work,
but the missing connection may cause later hooks to fail silently.

Note: bootstrap gets DOUBLE the configured hook_timeout_secs.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    context_window_tokens = request.get("context_window_tokens", 200000)
    stable_prefix_mode = request.get("stable_prefix_mode", False)

    # TODO: Connect to your data store here.
    # Example: initialise a SQLite connection, ping a vector DB, etc.
    #
    # import sqlite3
    # db = sqlite3.connect(os.path.expanduser("~/.librefang/my-plugin.db"))
    # db.execute("CREATE TABLE IF NOT EXISTS memories (...)")
    # db.commit()
    # db.close()
    #
    # Any errors raised here are caught and logged as warnings.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_PREPARE_SUBAGENT: &str = r#"#!/usr/bin/env python3
"""Context engine prepare_subagent hook.

Called just before a sub-agent is spawned. Use this to isolate memory scope,
snapshot parent state, or set up any resources the child agent needs.

Receives via stdin:
    {
      "type": "prepare_subagent",
      "parent_id": "uuid-of-parent-agent",
      "child_id":  "uuid-of-child-agent"
    }

Should print to stdout:
    {"type": "ok"}

Non-fatal: failures are logged as warnings and the sub-agent still spawns.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    parent_id = request["parent_id"]
    child_id = request["child_id"]

    # TODO: Snapshot or fork per-agent state here.
    # Example: copy parent memories to child scope in your data store.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_MERGE_SUBAGENT: &str = r#"#!/usr/bin/env python3
"""Context engine merge_subagent hook.

Called after a sub-agent completes. Use this to merge the child agent's
findings or memories back into the parent context.

Receives via stdin:
    {
      "type": "merge_subagent",
      "parent_id": "uuid-of-parent-agent",
      "child_id":  "uuid-of-child-agent"
    }

Should print to stdout:
    {"type": "ok"}

Non-fatal: failures are logged as warnings; the parent agent continues normally.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    parent_id = request["parent_id"]
    child_id = request["child_id"]

    # TODO: Merge child agent state into the parent here.
    # Example: copy child memories back to parent scope in your data store.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

// --- Node templates (assemble + compact) ---

const NODE_ASSEMBLE: &str = r#"#!/usr/bin/env node
// Context engine assemble hook (Node.js).
// Controls what the LLM sees — called before every LLM request.
//
// Receives on stdin:
//   {
//     "type": "assemble",
//     "system_prompt": "...",
//     "messages": [{"role":"user"|"assistant", "content": ..., "pinned": false}, ...],
//     "context_window_tokens": 200000
//   }
// content can be a plain string or an array of blocks (tool_use, tool_result, image, thinking).
//
// Emits on stdout:
//   {"type": "assemble_result", "messages": [...]}
//
// Return an empty list or fail to trigger fallback to LibreFang's default trimming.

"use strict";

function estimateTokens(msg) {
  const text = typeof msg.content === "string"
    ? msg.content
    : (Array.isArray(msg.content)
        ? msg.content.map(b => b.text || b.content || "").join(" ")
        : "");
  return Math.max(1, Math.ceil(text.length / 4));
}

let buf = "";
process.stdin.on("data", chunk => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const messages = req.messages;
  const budget = req.context_window_tokens - 4000; // headroom for system + response

  // Keep newest messages that fit within the token budget.
  let used = 0;
  const kept = [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const tokens = estimateTokens(messages[i]);
    if (used + tokens > budget) break;
    kept.unshift(messages[i]);
    used += tokens;
  }

  process.stdout.write(JSON.stringify({ type: "assemble_result", messages: kept }) + "\n");
});
"#;

const NODE_COMPACT: &str = r#"#!/usr/bin/env node
// Context engine compact hook (Node.js).
// Custom context compression — called under context pressure.
//
// Receives on stdin:
//   {
//     "type": "compact",
//     "agent_id": "...",
//     "messages": [...],
//     "model": "...",
//     "context_window_tokens": 200000
//   }
//
// Emits on stdout:
//   {"type": "compact_result", "messages": [...]}
//
// Return an empty list or fail to trigger fallback to LLM-based compaction.

"use strict";

let buf = "";
process.stdin.on("data", chunk => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const messages = req.messages;

  const pinned = messages.filter(m => m.pinned);
  const rest   = messages.filter(m => !m.pinned);

  // Keep last 10 non-pinned messages; summarise the rest with a placeholder.
  let compacted;
  if (rest.length > 10) {
    const summary = { role: "assistant", content: "... (older messages summarised) ...", pinned: false };
    compacted = [...pinned, summary, ...rest.slice(-10)];
  } else {
    compacted = [...pinned, ...rest];
  }

  process.stdout.write(JSON.stringify({ type: "compact_result", messages: compacted }) + "\n");
});
"#;

// --- Deno / TypeScript templates (assemble + compact) ---

const DENO_ASSEMBLE: &str = r#"// Context engine assemble hook (Deno / TypeScript).
// Controls what the LLM sees — called before every LLM request.
//
// Run via: deno run --allow-read assemble.ts

type ContentBlock = { type: string; text?: string; content?: string; [k: string]: unknown };
type Message = { role: string; content: string | ContentBlock[]; pinned: boolean };

function estimateTokens(msg: Message): number {
  const text = typeof msg.content === "string"
    ? msg.content
    : msg.content.map((b: ContentBlock) => b.text ?? b.content ?? "").join(" ");
  return Math.max(1, Math.ceil(text.length / 4));
}

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as { type: string; messages: Message[]; context_window_tokens: number };
const budget = req.context_window_tokens - 4000;

let used = 0;
const kept: Message[] = [];
for (let i = req.messages.length - 1; i >= 0; i--) {
  const tokens = estimateTokens(req.messages[i]);
  if (used + tokens > budget) break;
  kept.unshift(req.messages[i]);
  used += tokens;
}

console.log(JSON.stringify({ type: "assemble_result", messages: kept }));
"#;

const DENO_COMPACT: &str = r#"// Context engine compact hook (Deno / TypeScript).
// Custom context compression — called under context pressure.
//
// Run via: deno run --allow-read compact.ts

type Message = { role: string; content: unknown; pinned: boolean };

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as { type: string; messages: Message[] };
const messages = req.messages;

const pinned = messages.filter((m: Message) => m.pinned);
const rest   = messages.filter((m: Message) => !m.pinned);

const summary: Message = { role: "assistant", content: "... (older messages summarised) ...", pinned: false };
const compacted = rest.length > 10
  ? [...pinned, summary, ...rest.slice(-10)]
  : [...pinned, ...rest];

console.log(JSON.stringify({ type: "compact_result", messages: compacted }));
"#;

// --- Go templates (assemble + compact) ---

const GO_ASSEMBLE: &str = r#"// Context engine assemble hook (Go).
// Controls what the LLM sees — called before every LLM request.
//
// Run with: go run assemble.go
package main

import (
	"encoding/json"
	"io"
	"os"
)

type Message struct {
	Role    string `json:"role"`
	Content any    `json:"content"`
	Pinned  bool   `json:"pinned"`
}

type AssembleRequest struct {
	Type                string    `json:"type"`
	SystemPrompt        string    `json:"system_prompt"`
	Messages            []Message `json:"messages"`
	ContextWindowTokens int       `json:"context_window_tokens"`
}

type AssembleResult struct {
	Type     string    `json:"type"`
	Messages []Message `json:"messages"`
}

func estimateTokens(m Message) int {
	text := ""
	switch v := m.Content.(type) {
	case string:
		text = v
	}
	tokens := len(text) / 4
	if tokens < 1 {
		tokens = 1
	}
	return tokens
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req AssembleRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}

	budget := req.ContextWindowTokens - 4000
	used := 0
	kept := []Message{}
	for i := len(req.Messages) - 1; i >= 0; i-- {
		tokens := estimateTokens(req.Messages[i])
		if used+tokens > budget {
			break
		}
		kept = append([]Message{req.Messages[i]}, kept...)
		used += tokens
	}

	out, _ := json.Marshal(AssembleResult{Type: "assemble_result", Messages: kept})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

const GO_COMPACT: &str = r#"// Context engine compact hook (Go).
// Custom context compression — called under context pressure.
//
// Run with: go run compact.go
package main

import (
	"encoding/json"
	"io"
	"os"
)

type Message struct {
	Role    string `json:"role"`
	Content any    `json:"content"`
	Pinned  bool   `json:"pinned"`
}

type CompactRequest struct {
	Type                string    `json:"type"`
	AgentID             string    `json:"agent_id"`
	Messages            []Message `json:"messages"`
	Model               string    `json:"model"`
	ContextWindowTokens int       `json:"context_window_tokens"`
}

type CompactResult struct {
	Type     string    `json:"type"`
	Messages []Message `json:"messages"`
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req CompactRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}

	var pinned, rest []Message
	for _, m := range req.Messages {
		if m.Pinned {
			pinned = append(pinned, m)
		} else {
			rest = append(rest, m)
		}
	}

	compacted := append(pinned, rest...)
	if len(rest) > 10 {
		summary := Message{
			Role:    "assistant",
			Content: "... (older messages summarised) ...",
			Pinned:  false,
		}
		compacted = append(pinned, summary)
		compacted = append(compacted, rest[len(rest)-10:]...)
	}

	out, _ := json.Marshal(CompactResult{Type: "compact_result", Messages: compacted})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

// --- Node / Deno / Go bootstrap templates ---

const NODE_BOOTSTRAP: &str = r#"#!/usr/bin/env node
// Context engine bootstrap hook (Node.js).
// Runs ONCE at engine startup — connect to external services here.
// Receives: { type, context_window_tokens, stable_prefix_mode, max_recall_results }
// Returns:  { type: "ok" }
'use strict';
const { stdin } = process;
let raw = '';
stdin.setEncoding('utf8');
stdin.on('data', chunk => { raw += chunk; });
stdin.on('end', () => {
  // const req = JSON.parse(raw);
  // TODO: initialise your data store, warm caches, etc.
  process.stdout.write(JSON.stringify({ type: 'ok' }) + '\n');
});
"#;

const DENO_BOOTSTRAP: &str = r#"// Context engine bootstrap hook (Deno / TypeScript).
// Runs ONCE at engine startup — connect to external services here.
// Receives: { type, context_window_tokens, stable_prefix_mode, max_recall_results }
// Returns:  { type: "ok" }
const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
// const req = JSON.parse(raw);
// TODO: initialise your data store, warm caches, etc.
console.log(JSON.stringify({ type: 'ok' }));
"#;

const GO_BOOTSTRAP: &str = r#"// Context engine bootstrap hook (Go).
// Runs ONCE at engine startup — connect to external services here.
// go run bootstrap.go
package main

import (
	"encoding/json"
	"fmt"
	"os"
)

type BootstrapRequest struct {
	Type               string `json:"type"`
	ContextWindowTokens int   `json:"context_window_tokens"`
	StablePrefixMode   bool   `json:"stable_prefix_mode"`
	MaxRecallResults   int    `json:"max_recall_results"`
}

func main() {
	var req BootstrapRequest
	if err := json.NewDecoder(os.Stdin).Decode(&req); err != nil {
		fmt.Fprintln(os.Stderr, "bootstrap: invalid JSON on stdin:", err)
		os.Exit(1)
	}

	// TODO: connect to your database, warm caches, etc.

	fmt.Println(`{"type":"ok"}`)
}
"#;

// --- Minimal lifecycle stubs for other runtimes ---
// bootstrap / prepare_subagent / merge_subagent all use the same "ok" response.
// These stubs print {"type":"ok"} and exit — sufficient to acknowledge the hook.

const STUB_BOOTSTRAP_NODE: &str = r#"#!/usr/bin/env node
// Lifecycle hook stub (Node.js) — bootstrap / prepare_subagent / merge_subagent.
// Replace body with your logic; response must be {"type":"ok"}.
'use strict';
let raw = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', c => { raw += c; });
process.stdin.on('end', () => {
  // const req = JSON.parse(raw);
  process.stdout.write(JSON.stringify({ type: 'ok' }) + '\n');
});
"#;

const STUB_LIFECYCLE_DENO: &str = r#"// Lifecycle hook stub (Deno / TypeScript).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
await Deno.readAll(Deno.stdin); // consume stdin
console.log(JSON.stringify({ type: 'ok' }));
"#;

const STUB_LIFECYCLE_GO: &str = r#"// Lifecycle hook stub (Go).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
// go run <hook>.go
package main

import (
	"fmt"
	"io"
	"os"
)

func main() {
	io.ReadAll(os.Stdin) // consume stdin
	fmt.Println(`{"type":"ok"}`)
}
"#;

const STUB_LIFECYCLE_V: &str = r#"// Lifecycle hook stub (V).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
import os

fn main() {
    os.get_raw_stdin()  // consume stdin
    println('{"type":"ok"}')
}
"#;

const STUB_LIFECYCLE_RUBY: &str = r#"# Lifecycle hook stub (Ruby).
# bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
require 'json'
$stdin.read  # consume stdin
puts JSON.generate({ type: 'ok' })
"#;

const STUB_LIFECYCLE_BASH: &str = r#"#!/usr/bin/env bash
# Lifecycle hook stub (Bash).
# bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
cat /dev/stdin > /dev/null   # consume stdin
printf '{"type":"ok"}\n'
"#;

const STUB_LIFECYCLE_BUN: &str = r#"// Lifecycle hook stub (Bun / TypeScript).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
await Bun.stdin.text(); // consume stdin
console.log(JSON.stringify({ type: 'ok' }));
"#;

const STUB_LIFECYCLE_PHP: &str = r#"<?php
// Lifecycle hook stub (PHP).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
file_get_contents('php://stdin'); // consume stdin
echo json_encode(['type' => 'ok']) . "\n";
"#;

const STUB_LIFECYCLE_LUA: &str = r#"-- Lifecycle hook stub (Lua).
-- bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
io.read("*a")  -- consume stdin
print('{"type":"ok"}')
"#;

const STUB_LIFECYCLE_NATIVE: &str = r#"#!/bin/sh
# Lifecycle hook stub (native/shell wrapper).
# bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
cat > /dev/null  # consume stdin
printf '{"type":"ok"}\n'
"#;

// --- Minimal stubs for other runtimes (assemble + compact) ---
// These fall back gracefully — returning an empty messages list causes
// LibreFang to use its default overflow recovery / LLM compaction.

const STUB_ASSEMBLE_V: &str = r#"// Context engine assemble hook stub (V).
// See docs/agent/plugins for the full protocol.
// Returning empty messages triggers LibreFang's default context trimming.
module main
import os
import json

fn main() {
    _ := os.get_raw_stdin().bytestr()
    // TODO: implement assemble logic or delete this file to use default trimming.
    println(json.encode({ 'type': 'assemble_result', 'messages': [] }))
}
"#;

const STUB_COMPACT_V: &str = r#"// Context engine compact hook stub (V).
module main
import os
import json

fn main() {
    _ := os.get_raw_stdin().bytestr()
    // TODO: implement compact logic or delete this file to use LLM compaction.
    println(json.encode({ 'type': 'compact_result', 'messages': [] }))
}
"#;

const STUB_ASSEMBLE_RUBY: &str = r#"# Context engine assemble hook stub (Ruby).
# See docs/agent/plugins for the full protocol.
require "json"
_req = JSON.parse($stdin.read)
# TODO: implement assemble logic, or delete this file to use default trimming.
puts JSON.generate({ "type" => "assemble_result", "messages" => [] })
"#;

const STUB_COMPACT_RUBY: &str = r#"# Context engine compact hook stub (Ruby).
require "json"
_req = JSON.parse($stdin.read)
# TODO: implement compact logic, or delete this file to use LLM compaction.
puts JSON.generate({ "type" => "compact_result", "messages" => [] })
"#;

const STUB_ASSEMBLE_BASH: &str = r#"#!/usr/bin/env bash
# Context engine assemble hook stub (Bash).
# See docs/agent/plugins for the full protocol.
# For non-trivial logic, pipe stdin through `jq` or call a helper binary.
set -euo pipefail
_input=$(cat)
# TODO: implement assemble logic, or delete this file to use default trimming.
printf '{"type":"assemble_result","messages":[]}\n'
"#;

const STUB_COMPACT_BASH: &str = r#"#!/usr/bin/env bash
# Context engine compact hook stub (Bash).
set -euo pipefail
_input=$(cat)
# TODO: implement compact logic, or delete this file to use LLM compaction.
printf '{"type":"compact_result","messages":[]}\n'
"#;

const STUB_ASSEMBLE_BUN: &str = r#"// Context engine assemble hook stub (Bun / TypeScript).
// See docs/agent/plugins for the full protocol.
const _req = JSON.parse(await Bun.stdin.text());
// TODO: implement assemble logic, or delete this file to use default trimming.
console.log(JSON.stringify({ type: "assemble_result", messages: [] }));
"#;

const STUB_COMPACT_BUN: &str = r#"// Context engine compact hook stub (Bun / TypeScript).
const _req = JSON.parse(await Bun.stdin.text());
// TODO: implement compact logic, or delete this file to use LLM compaction.
console.log(JSON.stringify({ type: "compact_result", messages: [] }));
"#;

const STUB_ASSEMBLE_PHP: &str = r#"<?php
// Context engine assemble hook stub (PHP).
// See docs/agent/plugins for the full protocol.
$_req = json_decode(file_get_contents('php://stdin'), true);
// TODO: implement assemble logic, or delete this file to use default trimming.
echo json_encode(['type' => 'assemble_result', 'messages' => []]) . "\n";
"#;

const STUB_COMPACT_PHP: &str = r#"<?php
// Context engine compact hook stub (PHP).
$_req = json_decode(file_get_contents('php://stdin'), true);
// TODO: implement compact logic, or delete this file to use LLM compaction.
echo json_encode(['type' => 'compact_result', 'messages' => []]) . "\n";
"#;

const STUB_ASSEMBLE_LUA: &str = r#"-- Context engine assemble hook stub (Lua).
-- See docs/agent/plugins for the full protocol.
local json = require("json")  -- install lua-cjson or dkjson
local _req = json.decode(io.read("*a"))
-- TODO: implement assemble logic, or delete this file to use default trimming.
print(json.encode({ type = "assemble_result", messages = {} }))
"#;

const STUB_COMPACT_LUA: &str = r#"-- Context engine compact hook stub (Lua).
local json = require("json")
local _req = json.decode(io.read("*a"))
-- TODO: implement compact logic, or delete this file to use LLM compaction.
print(json.encode({ type = "compact_result", messages = {} }))
"#;

const STUB_ASSEMBLE_NATIVE: &str = r#"#!/bin/sh
# Context engine assemble hook stub (native shell wrapper).
# Replace this script with a pre-compiled binary that speaks the JSON protocol.
# Returning empty messages triggers LibreFang's default context trimming.
read -r _input
printf '{"type":"assemble_result","messages":[]}\n'
"#;

const STUB_COMPACT_NATIVE: &str = r#"#!/bin/sh
# Context engine compact hook stub (native shell wrapper).
# Replace with a pre-compiled binary that speaks the JSON protocol.
read -r _input
printf '{"type":"compact_result","messages":[]}\n'
"#;

// --- V language templates ---

const V_INGEST: &str = r#"// Context engine ingest hook (V).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "user message text"}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}
//
// Run with: `v run ingest.v` (or pre-compile: `v ingest.v`)
module main

import os
import json

struct IngestRequest {
	@type     string @[json: 'type']
	agent_id  string
	message   string
}

struct Memory {
	content string
}

struct IngestResult {
	@type    string   @[json: 'type']
	memories []Memory
}

fn main() {
	input := os.get_raw_stdin().bytestr()
	req := json.decode(IngestRequest, input) or {
		eprintln('ingest: invalid JSON on stdin: ${err}')
		exit(1)
	}
	_ := req.agent_id
	_ := req.message

	// TODO: Implement your custom recall logic here.
	result := IngestResult{
		@type: 'ingest_result'
		memories: []
	}
	println(json.encode(result))
}
"#;

const V_AFTER_TURN: &str = r#"// Context engine after_turn hook (V).
//
// Receives on stdin:
//   {"type": "after_turn", "agent_id": "...", "messages": [...]}
// Emits on stdout:
//   {"type": "ok"}
module main

import os
import json

struct AfterTurnRequest {
	@type    string @[json: 'type']
	agent_id string
}

struct Ok {
	@type string @[json: 'type']
}

fn main() {
	input := os.get_raw_stdin().bytestr()
	_ := json.decode(AfterTurnRequest, input) or {
		eprintln('after_turn: invalid JSON on stdin: ${err}')
		exit(1)
	}

	// TODO: persist state, update indexes, log analytics, ...

	println(json.encode(Ok{ @type: 'ok' }))
}
"#;

// --- Node templates ---

const NODE_INGEST: &str = r#"#!/usr/bin/env node
// Context engine ingest hook (Node.js).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "user message text"}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}

"use strict";

let buf = "";
process.stdin.on("data", (chunk) => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const agentId = req.agent_id;
  const message = req.message;

  // TODO: Implement your custom recall logic here.
  const memories = [];

  process.stdout.write(JSON.stringify({ type: "ingest_result", memories }) + "\n");
});
"#;

const NODE_AFTER_TURN: &str = r#"#!/usr/bin/env node
// Context engine after_turn hook (Node.js).

"use strict";

let buf = "";
process.stdin.on("data", (chunk) => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const _agentId = req.agent_id;
  const _messages = req.messages;

  // TODO: persist state, update indexes, log analytics, ...

  process.stdout.write(JSON.stringify({ type: "ok" }) + "\n");
});
"#;

// --- Deno / TypeScript templates ---

const DENO_INGEST: &str = r#"// Context engine ingest hook (Deno / TypeScript).
//
// Run via `deno run --allow-read ingest.ts`.

interface IngestRequest { type: "ingest"; agent_id: string; message: string; }
interface Memory { content: string; }
interface IngestResult { type: "ingest_result"; memories: Memory[]; }

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as IngestRequest;
void req.agent_id; void req.message;

// TODO: Implement your custom recall logic here.
const result: IngestResult = { type: "ingest_result", memories: [] };
console.log(JSON.stringify(result));
"#;

const DENO_AFTER_TURN: &str = r#"// Context engine after_turn hook (Deno / TypeScript).

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
void JSON.parse(raw);

// TODO: persist state, update indexes, log analytics, ...

console.log(JSON.stringify({ type: "ok" }));
"#;

// --- Go templates ---

const GO_INGEST: &str = r#"// Context engine ingest hook (Go).
//
// Run with: `go run ingest.go`
package main

import (
	"encoding/json"
	"io"
	"os"
)

type IngestRequest struct {
	Type    string `json:"type"`
	AgentID string `json:"agent_id"`
	Message string `json:"message"`
}

type Memory struct {
	Content string `json:"content"`
}

type IngestResult struct {
	Type     string   `json:"type"`
	Memories []Memory `json:"memories"`
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req IngestRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}
	_ = req.AgentID
	_ = req.Message

	// TODO: Implement your custom recall logic here.
	out, _ := json.Marshal(IngestResult{Type: "ingest_result", Memories: []Memory{}})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

const GO_AFTER_TURN: &str = r#"// Context engine after_turn hook (Go).
package main

import (
	"encoding/json"
	"io"
	"os"
)

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req map[string]any
	_ = json.Unmarshal(raw, &req)

	// TODO: persist state, update indexes, log analytics, ...

	out, _ := json.Marshal(map[string]string{"type": "ok"})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

// --- Native (bring-your-own-binary) templates ---

const NATIVE_INGEST: &str = r#"#!/bin/sh
# Native plugin ingest hook.
#
# Replace this shell wrapper with your own pre-compiled binary
# (V / Rust / Go / Zig / C++ — anything that speaks the JSON
# stdin/stdout protocol).
#
# Receives on stdin:
#   {"type": "ingest", "agent_id": "...", "message": "..."}
# Emits on stdout:
#   {"type": "ingest_result", "memories": [...]}
#
# chmod +x hooks/ingest to make this executable.

read -r _input
printf '{"type":"ingest_result","memories":[]}\n'
"#;

const NATIVE_AFTER_TURN: &str = r#"#!/bin/sh
# Native plugin after_turn hook — replace with your binary.
read -r _input
printf '{"type":"ok"}\n'
"#;

// --- Ruby templates ---

const RUBY_INGEST: &str = r#"# Context engine ingest hook (Ruby).
#
# Receives on stdin:
#   {"type": "ingest", "agent_id": "...", "message": "..."}
# Emits on stdout:
#   {"type": "ingest_result", "memories": [{"content": "..."}]}
require "json"

req = JSON.parse($stdin.read)
_agent_id = req["agent_id"]
_message  = req["message"]

# TODO: Implement your custom recall logic here.
memories = []

puts JSON.generate({ "type" => "ingest_result", "memories" => memories })
"#;

const RUBY_AFTER_TURN: &str = r#"# Context engine after_turn hook (Ruby).
require "json"

req = JSON.parse($stdin.read)
_agent_id = req["agent_id"]
_messages = req["messages"]

# TODO: Implement your post-turn logic here.

puts JSON.generate({ "type" => "ok" })
"#;

// --- Bash templates ---

const BASH_INGEST: &str = r#"#!/usr/bin/env bash
# Context engine ingest hook (Bash).
#
# Receives on stdin:
#   {"type":"ingest","agent_id":"...","message":"..."}
# Emits on stdout:
#   {"type":"ingest_result","memories":[]}
#
# For non-trivial logic, pipe stdin through `jq` or call out to a helper binary.
set -euo pipefail

_input=$(cat)
# TODO: parse "$_input" and build your recall result.
printf '{"type":"ingest_result","memories":[]}\n'
"#;

const BASH_AFTER_TURN: &str = r#"#!/usr/bin/env bash
# Context engine after_turn hook (Bash).
set -euo pipefail

_input=$(cat)
# TODO: persist state, update indexes, etc.
printf '{"type":"ok"}\n'
"#;

// --- Bun templates (TypeScript via Bun) ---

const BUN_INGEST: &str = r#"// Context engine ingest hook (Bun / TypeScript).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "..."}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "..."}]}
//
// Run with: `bun run ingest.ts`

interface IngestRequest {
  type: "ingest";
  agent_id: string;
  message: string;
}

interface Memory { content: string }

const input = await Bun.stdin.text();
const req = JSON.parse(input) as IngestRequest;
void req.agent_id;
void req.message;

// TODO: Implement your custom recall logic here.
const memories: Memory[] = [];

console.log(JSON.stringify({ type: "ingest_result", memories }));
"#;

const BUN_AFTER_TURN: &str = r#"// Context engine after_turn hook (Bun / TypeScript).
const input = await Bun.stdin.text();
const _req = JSON.parse(input);

// TODO: Implement your post-turn logic here.

console.log(JSON.stringify({ type: "ok" }));
"#;

// --- PHP templates ---

const PHP_INGEST: &str = r#"<?php
// Context engine ingest hook (PHP).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "..."}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "..."}]}

$raw = stream_get_contents(STDIN);
$req = json_decode($raw, true);
$_agentId = $req["agent_id"] ?? null;
$_message = $req["message"] ?? null;

// TODO: Implement your custom recall logic here.
$memories = [];

echo json_encode(["type" => "ingest_result", "memories" => $memories]), "\n";
"#;

const PHP_AFTER_TURN: &str = r#"<?php
// Context engine after_turn hook (PHP).
$raw = stream_get_contents(STDIN);
$_req = json_decode($raw, true);

// TODO: Implement your post-turn logic here.

echo json_encode(["type" => "ok"]), "\n";
"#;

// --- Lua templates ---

const LUA_INGEST: &str = r#"-- Context engine ingest hook (Lua).
--
-- Receives on stdin:
--   {"type": "ingest", "agent_id": "...", "message": "..."}
-- Emits on stdout:
--   {"type": "ingest_result", "memories": [{"content": "..."}]}
--
-- Requires a JSON library on LUA_PATH (`luarocks install dkjson`).
local json = require("dkjson")

local raw = io.read("*a")
local req = json.decode(raw)
local _agent_id = req.agent_id
local _message  = req.message

-- TODO: Implement your custom recall logic here.
local memories = {}

io.write(json.encode({ type = "ingest_result", memories = memories }), "\n")
"#;

const LUA_AFTER_TURN: &str = r#"-- Context engine after_turn hook (Lua).
local json = require("dkjson")

local raw = io.read("*a")
local _req = json.decode(raw)

-- TODO: Implement your post-turn logic here.

io.write(json.encode({ type = "ok" }), "\n")
"#;
