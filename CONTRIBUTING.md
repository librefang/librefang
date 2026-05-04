# Contributing to LibreFang

Thank you for your interest in contributing to LibreFang! "Libre" means freedom, and we mean it — this project is built by its community.

**Our promise:** if your contribution positively helps the project, we merge it as-is. If it needs improvement, we provide active, constructive review to help you get it merged. Every contributor matters.

Active contributors are invited to join the LibreFang GitHub org — core participants who consistently contribute get commit access and a voice in project direction.

This guide covers everything you need to get started, from setting up your development environment to submitting pull requests.

## Table of Contents

- [Ways to Contribute](#ways-to-contribute)
- [Contributing to the Registry](#contributing-to-the-registry)
- [Development Environment](#development-environment)
- [Building and Testing](#building-and-testing)
- [Code Style](#code-style)
- [Architecture Overview](#architecture-overview)
- [How to Add a New Agent Template](#how-to-add-a-new-agent-template)
- [How to Add a New Skill](#how-to-add-a-new-skill)
- [How to Add a New Channel Adapter](#how-to-add-a-new-channel-adapter)
- [How to Add a New LLM Provider](#how-to-add-a-new-llm-provider)
- [How to Add a New Tool](#how-to-add-a-new-tool)
- [How to Write Integration Tests](#how-to-write-integration-tests)
- [Release Articles](#release-articles)
- [Pull Request Process](#pull-request-process)
- [Code of Conduct](#code-of-conduct)

---

## Ways to Contribute

**You don't need to know Rust to contribute to LibreFang.** We have contribution paths for every skill level:

### No Rust Required

| What | Skills Needed | Time | Where |
|------|--------------|------|-------|
| Write an agent template | TOML + prompt engineering | 1-2 hours | `agents/` |
| Write a skill (Python) | Python | 2-4 hours | `~/.librefang/skills/` |
| Write a skill (JavaScript) | Node.js | 2-4 hours | `~/.librefang/skills/` |
| Fix typos / improve docs | Markdown | 30 min | `docs/` |
| Translate docs | Markdown + language | 1-2 hours | `docs/i18n/` |
| Report bugs with reproduction steps | Testing | 30 min | GitHub Issues |
| Test on uncommon platforms | Testing | 1 hour | GitHub Issues |

### Basic Rust

| What | Skills Needed | Time | Where |
|------|--------------|------|-------|
| Add a channel adapter | Rust + platform API | half day | `crates/librefang-channels/` |
| Add an LLM provider driver | Rust + provider API | half day | `crates/librefang-runtime/` |
| Add a built-in tool | Rust | 2-4 hours | `crates/librefang-runtime/` |
| Write/improve tests | Rust | 1-2 hours | any crate |

### Advanced Rust

| What | Skills Needed | Time | Where |
|------|--------------|------|-------|
| Kernel features | Deep Rust + architecture | 1+ days | `crates/librefang-kernel/` |
| Security hardening | Rust + security | 1+ days | multiple crates |
| Performance optimization | Rust + profiling | varies | any crate |
| WASM sandbox improvements | Rust + Wasmtime | 1+ days | `crates/librefang-runtime/` |

### Other

| What | Skills Needed | Time | Where |
|------|--------------|------|-------|
| Desktop app features | Rust + Tauri + TypeScript | varies | `crates/librefang-desktop/` |
| JavaScript SDK | TypeScript | varies | `sdk/javascript/` |
| Python SDK | Python | varies | `sdk/python/` |
| WhatsApp gateway | Node.js | varies | `packages/whatsapp-gateway/` |

> **Tip:** Look for issues labeled [`good first issue`](https://github.com/librefang/librefang/labels/good%20first%20issue) — they include the files to modify, how to test, and estimated difficulty.

### Quick Start by Contribution Type

**I want to add an agent template** (no Rust):
```bash
cp -r agents/hello-world agents/my-agent
# Edit agents/my-agent/agent.toml
# Submit a PR
```

**I want to write a Python skill** (no Rust):
```bash
mkdir -p ~/.librefang/skills/my-skill
# See https://docs.librefang.ai/agent/skills for the skill format
```

**I want to fix a bug or add a Rust feature**:
```bash
git clone https://github.com/librefang/librefang.git && cd librefang
cargo build --workspace        # Build
cargo test --workspace         # Test
cargo clippy --workspace --all-targets -- -D warnings  # Lint
```

---

## Contributing to the Registry

The [`librefang-registry`](https://github.com/librefang/librefang-registry) repo is the shared catalog the website browses (at [librefang.ai/skills](https://librefang.ai/skills), `/hands`, etc.) and the CLI pulls from. Contributions are welcome without touching the main Rust codebase.

### What lives in the registry

| Path | Format | What it is |
|------|--------|------------|
| `skills/<id>/SKILL.md` | directory | A prompt-only or WASM skill bundle (markdown + YAML frontmatter) |
| `hands/<id>/HAND.toml` | directory | An autonomous capability unit |
| `agents/<id>/agent.toml` | directory | A pre-built agent template |
| `channels/<id>.toml` | file | A messaging adapter manifest |
| `providers/<id>.toml` | file | An LLM provider adapter manifest |
| `workflows/<id>.toml` | file | A multi-step agent workflow |
| `plugins/<id>/plugin.toml` | directory | A runtime plugin manifest |
| `mcp/<id>.toml` | file | An MCP server manifest |

### Submitting a new entry

1. Fork [`librefang-registry`](https://github.com/librefang/librefang-registry).
2. Add your manifest to the right category directory. Follow the schema of an existing neighbour.
3. Required TOML fields for every entry: `id`, `name`, `description`, `category`, `icon` (one emoji).
4. Add i18n descriptions in `[i18n.zh]`, `[i18n.ja]`, `[i18n.ko]` if you can — the website renders localized descriptions when available.
5. Tag with `tags = ["popular"]` only if you've validated real usage; the site visually promotes popular entries.
6. Open a PR against the registry repo. On merge, the entry is live on librefang.ai within an hour (the Cloudflare Worker at `stats.librefang.ai` runs stale-while-revalidate with a 1-hour fresh window).

### Testing your manifest locally

```bash
# Run the official site against your local registry checkout
cd librefang/web
pnpm dev

# Or install a single skill directly into a running daemon
librefang skill install /path/to/librefang-registry/skills/your-skill
```

### What the website expects

The website's detail pages expect the TOML to be parseable and render the raw contents. No HTML or Markdown is interpreted — readers see the TOML as-is with syntax highlighting. Keep descriptions concise (≤ 280 chars) so they fit in meta tags and social-share cards.

---

## Development Environment

### Option A: GitHub Codespace (Recommended for first-time contributors)

Click the green **"Code"** button on GitHub → **"Codespaces"** → **"Create codespace on main"**. The DevContainer will automatically install Rust, Python, Node.js, and build the project. You'll have a fully working environment in your browser within a few minutes.

### Option B: Local Setup

#### Prerequisites

- **Rust 1.94.1+** (install via [rustup](https://rustup.rs/))
- **Git**
- **Python 3.8+** (optional, for Python runtime and skills)
- A supported LLM API key (Anthropic, OpenAI, Groq, etc.) for end-to-end testing

#### Clone and Build

```bash
git clone https://github.com/librefang/librefang.git
cd librefang
just setup        # one-time per clone — activates git hooks + fetches deps
cargo build
```

`just setup` (which calls `cargo xtask setup`) does three things on a fresh clone:
- Sets `git config core.hooksPath scripts/hooks` so the in-repo `pre-commit`, `pre-push`, and `commit-msg` hooks become active.
- Runs `cargo fetch` to warm up the dependency cache.
- Runs `pnpm install` in the dashboard / web / docs sub-projects.

Hooks are scoped to fast, staged-only checks. CI is the authoritative
gate (clippy, openapi/SDK drift, security audit, full test matrix).

| Hook        | Runs                                                                  | Target time |
|-------------|-----------------------------------------------------------------------|-------------|
| `pre-commit`| `cargo fmt --check` on staged `*.rs` only, CHANGELOG guard, `detect-secrets` (if installed) | < 2s |
| `pre-push`  | Refuses direct push to `main` / `master`. Nothing else.                | < 100ms |
| `commit-msg`| Reject Claude / Anthropic attribution                                  | < 50ms |

Want to lint locally before pushing? Run `just lint` (or
`cargo clippy --workspace --all-targets -- -D warnings`) on demand.
That belongs in your loop when you want it, not gating every push.

Skip the pre-push branch guard with
`LIBREFANG_PREPUSH_SKIP=1 git push` (or `--no-verify`) when the
maintainers have agreed to a release / hotfix push to `main`.

For secret scanning, install `detect-secrets` once (`pipx install detect-secrets`). False positives are managed via `.secrets.baseline`:

```bash
detect-secrets scan --baseline .secrets.baseline   # update findings
detect-secrets audit .secrets.baseline             # mark each as real / false-positive
```

If you also use the `pre-commit` framework (`pipx install pre-commit`), the equivalent staged-only fmt + secret scan is wired in `.pre-commit-config.yaml`. The framework's `pre-push` stage is intentionally not wired — CI is the gate, no need to duplicate locally:

```bash
pre-commit install --install-hooks                 # commit stage
```

You only need to run it once per clone — `git pull` keeps the hooks current automatically because they live in `scripts/hooks/` rather than being copied into `.git/hooks/`.

The first build takes a few minutes because it compiles SQLite (bundled) and Wasmtime. Subsequent builds are incremental.

#### Worktrees for parallel work

For any non-trivial feature, work in a `git worktree` rather than the main clone. This avoids contending with other sessions on the shared `target/` directory and prevents accidental edits on the wrong branch:

```bash
git worktree add /tmp/librefang-<feature> -b <branch-name> origin/main
cd /tmp/librefang-<feature>
```

New worktrees inherit the main clone's `core.hooksPath` setting, so the git hooks are active there too without re-running `just setup`.

### Environment Variables

For running integration tests that hit a real LLM, set at least one provider key:

```bash
export GROQ_API_KEY=gsk_...          # Recommended for fast, free-tier testing
export ANTHROPIC_API_KEY=sk-ant-...  # For Anthropic-specific tests
```

Tests that require a real LLM key will skip gracefully if the env var is absent.

---

## Building and Testing

### Build the Entire Workspace

```bash
cargo build --workspace
```

### Run All Tests

```bash
cargo test --workspace
```

The test suite is currently 2,100+ tests. All must pass before merging.

### Run Tests for a Single Crate

```bash
cargo test -p librefang-kernel
cargo test -p librefang-runtime
cargo test -p librefang-memory
```

### Check for Clippy Warnings

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

The CI pipeline enforces zero clippy warnings.

### Format Code

```bash
cargo fmt --all
```

Always run `cargo fmt` before committing. CI will reject unformatted code.

### Run the Doctor Check

After building, verify your local setup:

```bash
cargo run -- doctor
```

---

## Code Style

- **Formatting**: Use `rustfmt` with default settings. Run `cargo fmt --all` before every commit.
- **Linting**: `cargo clippy --workspace -- -D warnings` must pass with zero warnings.
- **Documentation**: All public types and functions must have doc comments (`///`).
- **Error Handling**: Use `thiserror` for error types. Avoid `unwrap()` in library code; prefer `?` propagation.
- **Naming**:
  - Types: `PascalCase` (e.g., `LibreFangKernel`, `AgentManifest`)
  - Functions/methods: `snake_case`
  - Constants: `SCREAMING_SNAKE_CASE`
  - Crate names: `librefang-{name}` (kebab-case)
- **Dependencies**: Workspace dependencies are declared in the root `Cargo.toml`. Prefer reusing workspace deps over adding new ones. If you need a new dependency, justify it in the PR.
- **Testing**: Every new feature must include tests. Use `tempfile::TempDir` for filesystem isolation and random port binding for network tests.
- **Serde**: All config structs use `#[serde(default)]` for forward compatibility with partial TOML.

---

## Architecture Overview

LibreFang is organized as a Cargo workspace with 14 crates:

| Crate | Role |
|-------|------|
| `librefang-types` | Shared type definitions, taint tracking, manifest signing (Ed25519), model catalog, MCP/A2A config types |
| `librefang-memory` | SQLite-backed memory substrate with vector embeddings, usage tracking, canonical sessions, JSONL mirroring |
| `librefang-runtime` | Agent loop, 3 LLM drivers (Anthropic/Gemini/OpenAI-compat), 53 built-in tools, WASM sandbox, MCP client/server, A2A protocol |
| `librefang-hands` | Hands system (curated autonomous capability packages), 7 bundled hands |
| `librefang-extensions` | Integration registry (25 bundled MCP templates), AES-256-GCM credential vault, OAuth2 PKCE |
| `librefang-kernel` | Assembles all subsystems: workflow engine, RBAC auth, heartbeat monitor, cron scheduler, config hot-reload |
| `librefang-api` | REST/WS/SSE API (Axum 0.8), 76 endpoints, 14-page SPA dashboard, OpenAI-compatible `/v1/chat/completions` |
| `librefang-channels` | 40 channel adapters (Telegram, Discord, Slack, WhatsApp, and 36 more), formatter, rate limiter |
| `librefang-wire` | OFP (LibreFang Protocol): TCP P2P networking with HMAC-SHA256 mutual authentication |
| `librefang-cli` | Clap CLI with daemon auto-detect (HTTP mode vs. in-process fallback), MCP server |
| `librefang-migrate` | Migration engine for importing from OpenClaw (and future frameworks) |
| `librefang-skills` | Skill system: 60 bundled skills, FangHub marketplace, OpenClaw compatibility, prompt injection scanning |
| `librefang-desktop` | Tauri 2.0 native desktop app (WebView + system tray + single-instance + notifications) |
| `xtask` | Build automation tasks |

### Key Architectural Patterns

- **`KernelHandle` trait**: Defined in `librefang-runtime`, implemented on `LibreFangKernel` in `librefang-kernel`. This avoids circular crate dependencies while enabling inter-agent tools.
- **Shared memory**: A fixed UUID (`AgentId(Uuid::from_bytes([0..0, 0x01]))`) provides a cross-agent KV namespace.
- **Daemon detection**: The CLI checks `~/.librefang/daemon.json` and pings the health endpoint. If a daemon is running, commands use HTTP; otherwise, they boot an in-process kernel.
- **Capability-based security**: Every agent operation is checked against the agent's granted capabilities before execution.

---

## How to Add a New Agent Template

Agent templates live in the `agents/` directory. Each template is a folder containing an `agent.toml` manifest.

### Steps

1. Create a new directory under `agents/`:

```
agents/my-agent/agent.toml
```

2. Write the manifest:

```toml
name = "my-agent"
version = "0.1.0"
description = "A brief description of what this agent does."
author = "librefang"
module = "builtin:chat"
tags = ["category"]

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"

[resources]
max_llm_tokens_per_hour = 100000

[capabilities]
tools = ["file_read", "file_list", "web_fetch"]
memory_read = ["*"]
memory_write = ["self.*"]
agent_spawn = false
```

3. Include a system prompt if needed by adding it to the `[model]` section:

```toml
[model]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
system_prompt = """
You are a specialized agent that...
"""
```

4. Test by spawning:

```bash
librefang agent spawn agents/my-agent/agent.toml
```

5. Submit a PR with the new template.

---

## How to Add a New Skill

Skills are reusable capabilities that agents can invoke. They can be written in **Python, JavaScript, or as pure prompt templates** — no Rust required.

### Skill Types

| Type | Language | Description |
|------|----------|-------------|
| `prompt` | None (TOML only) | A prompt template with variables |
| `python` | Python 3.8+ | A Python script with `run()` entry point |
| `javascript` | Node.js 18+ | A JS module with `run()` export |

### Steps (Python example)

1. Create a skill directory:

```
my-skill/
  skill.toml
  main.py
```

2. Write the manifest (`skill.toml`):

```toml
name = "my-skill"
version = "0.1.0"
description = "What this skill does."
author = "your-name"
runtime = "python"
entry = "main.py"
tags = ["utility"]

[input]
url = { type = "string", description = "URL to process", required = true }
```

3. Write the implementation (`main.py`):

```python
def run(input: dict) -> str:
    url = input["url"]
    # Your logic here
    return f"Processed: {url}"
```

4. Test locally:

```bash
librefang skill test ./my-skill --input '{"url": "https://example.com"}'
```

5. Submit as a PR to `skills/community/` or publish to FangHub.

### Steps (Prompt template)

For skills that are just prompt engineering, no code is needed:

```toml
name = "summarize-email"
version = "0.1.0"
description = "Summarize an email thread."
runtime = "promptonly"
tags = ["email", "productivity"]

[input]
thread = { type = "string", description = "The email thread text", required = true }

[prompt]
template = """
Summarize the following email thread in 3 bullet points:

{{thread}}
"""
```

---

## How to Add a New Channel Adapter

Channel adapters live in `crates/librefang-channels/src/`. Each adapter implements the `ChannelAdapter` trait.

### Steps

1. Create a new file: `crates/librefang-channels/src/myplatform.rs`

2. Implement the `ChannelAdapter` trait (defined in `types.rs`):

```rust
use crate::types::{ChannelAdapter, ChannelMessage, ChannelType};
use async_trait::async_trait;

pub struct MyPlatformAdapter {
    // token, client, config fields
}

#[async_trait]
impl ChannelAdapter for MyPlatformAdapter {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("myplatform".to_string())
    }

    async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Start polling/listening for messages
        Ok(())
    }

    async fn send(&self, channel_id: &str, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Send a message back to the platform
        Ok(())
    }

    async fn stop(&mut self) {
        // Clean shutdown
    }
}
```

3. Register the module in `crates/librefang-channels/src/lib.rs`:

```rust
pub mod myplatform;
```

4. Wire it up in the channel bridge (`crates/librefang-api/src/channel_bridge.rs`) so the daemon starts it alongside other adapters.

5. Add configuration support in `librefang-types` config structs (add a `[channels.myplatform]` section).

6. Add CLI setup wizard instructions in `crates/librefang-cli/src/main.rs` under `cmd_channel_setup`.

7. Write tests and submit a PR.

---

## How to Add a New LLM Provider

LLM provider drivers live in `crates/librefang-runtime/src/`. LibreFang uses three driver families that cover most providers:

| Driver | Covers |
|--------|--------|
| `openai_compat` | Any OpenAI-compatible API (Groq, Together, Mistral, local Ollama, etc.) |
| `anthropic` | Anthropic Claude models |
| `gemini` | Google Gemini models |

### If your provider is OpenAI-compatible

Most new providers don't need a new driver — just add an entry to the model catalog in `crates/librefang-types/src/models.rs`:

1. Add the provider constant and its base URL.
2. Add model entries with context window sizes and pricing.
3. Add aliases if desired (e.g., `"fast" -> "groq/llama-3.3-70b"`).
4. Write a test verifying the model resolves correctly.

### If your provider needs a custom driver

1. Create `crates/librefang-runtime/src/my_provider.rs`.
2. Implement the `LlmDriver` trait (see `anthropic.rs` for reference).
3. Register it in the driver factory in `crates/librefang-runtime/src/llm_driver.rs`.
4. Add config types in `crates/librefang-types/src/config.rs`.
5. Write integration tests (they should skip gracefully if the API key env var is absent).

---

## How to Add a New Tool

Built-in tools are defined in `crates/librefang-runtime/src/tool_runner.rs`.

### Steps

1. Add the tool implementation function:

```rust
async fn tool_my_tool(input: &serde_json::Value) -> Result<String, String> {
    let param = input["param"]
        .as_str()
        .ok_or("Missing 'param' field")?;

    // Tool logic here
    Ok(format!("Result: {param}"))
}
```

2. Register it in the `execute_tool` match block:

```rust
"my_tool" => tool_my_tool(input).await,
```

3. Add the tool definition to `builtin_tool_definitions()`:

```rust
ToolDefinition {
    name: "my_tool".to_string(),
    description: "Description shown to the LLM.".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "param": {
                "type": "string",
                "description": "The parameter description"
            }
        },
        "required": ["param"]
    }),
},
```

4. Agents that need the tool must list it in their manifest:

```toml
[capabilities]
tools = ["my_tool"]
```

5. Write tests for the tool function.

6. If the tool requires kernel access (e.g., inter-agent communication), accept `Option<&Arc<dyn KernelHandle>>` and handle the `None` case gracefully.

---

## How to Write Integration Tests

LibreFang has 2,100+ tests covering all crates. Every new feature must include tests. This section explains where tests live, how to structure them, and how to run them.

### Where Tests Live

Tests in LibreFang are **inline** — they live alongside the source code in `#[cfg(test)]` modules at the bottom of each `.rs` file:

```
crates/librefang-kernel/src/metering.rs     # contains #[cfg(test)] mod tests { ... }
crates/librefang-memory/src/substrate.rs    # contains #[cfg(test)] mod tests { ... }
crates/librefang-runtime/src/routing.rs     # contains #[cfg(test)] mod tests { ... }
```

This is the standard Rust convention and keeps tests close to the code they verify.

### Naming Conventions

- Test module: `#[cfg(test)] mod tests { ... }` at the bottom of the file.
- Test functions: `test_<what_is_being_tested>` in `snake_case`.
  - Good: `test_record_and_check_quota_under`, `test_substrate_kv`, `test_routing_table_lookup`
  - Avoid: `test1`, `it_works`, `my_test`

### How to Structure a Test

Follow the **setup / action / assertion** pattern:

1. **Setup** — create the dependencies your code needs (in-memory databases, config structs, etc.).
2. **Action** — call the function or method under test.
3. **Assertion** — verify the result with `assert!`, `assert_eq!`, or pattern matching.

Many crates provide helpers for setup. For example, `MemorySubstrate::open_in_memory(0.1)` creates an in-memory SQLite database, and `MeteringEngine` tests use a shared `setup()` function.

### How to Run Tests

**All tests in the workspace:**

```bash
cargo test --workspace
```

**Tests for a specific crate:**

```bash
cargo test -p librefang-kernel
cargo test -p librefang-memory
cargo test -p librefang-runtime
```

**A single test by name:**

```bash
cargo test -p librefang-kernel test_record_and_check_quota_under
```

**Show output from passing tests (useful for debugging):**

```bash
cargo test -p librefang-memory -- --nocapture
```

### Test Skeleton

#### Synchronous test

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_feature() {
        // Setup
        let config = MyConfig::default();

        // Action
        let result = config.validate();

        // Assertion
        assert!(result.is_ok());
    }
}
```

#### Async test (requires `tokio`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_my_async_feature() {
        // Setup
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();

        // Action
        substrate
            .set(agent_id, "key", serde_json::json!("value"))
            .await
            .unwrap();
        let val = substrate.get(agent_id, "key").await.unwrap();

        // Assertion
        assert_eq!(val, Some(serde_json::json!("value")));
    }
}
```

### Tips

- **Use `#[tokio::test]`** for any test that calls `.await`. Most crates in LibreFang already depend on `tokio` with the `test-util` feature.
- **Use in-memory databases** for isolation. `MemorySubstrate::open_in_memory(0.1)` avoids touching the real filesystem.
- **Use `tempfile::TempDir`** when you need a real directory (e.g., skill loading, file I/O tests). The directory is automatically cleaned up when the `TempDir` value is dropped.
- **Use `Default::default()`** to construct config structs with sensible defaults, then override only the fields relevant to your test.
- **Skip tests that need external services** by checking for environment variables:
  ```rust
  #[tokio::test]
  async fn test_llm_integration() {
      let api_key = match std::env::var("GROQ_API_KEY") {
          Ok(k) => k,
          Err(_) => {
              eprintln!("Skipping: GROQ_API_KEY not set");
              return;
          }
      };
      // ... test with real API
  }
  ```
- **Extract a `setup()` helper** when multiple tests in the same module need the same boilerplate (see `crates/librefang-kernel/src/metering.rs` for an example).
- **Test error cases too** — verify that invalid input returns the expected error, not just that the happy path works.

---

## Release Articles

Each dated release in `CHANGELOG.md` (e.g. `## [2026.4.27]`) should land with a
companion file in `articles/release-<YYYY.M.D>.md`. Two GitHub workflows
consume these files on push to `main`:

- `.github/workflows/devto-publish.yml` — creates / updates the matching
  dev.to post (title-keyed, idempotent).
- `.github/workflows/release-notify.yml` — posts a GitHub Discussion under the
  release tag using the article body.

If the article is missing on a release tag, the dev.to post and the GitHub
Discussion are silently skipped — public release comms quietly stop. The
`articles/` directory drifted out of sync with `CHANGELOG.md` after
2026-03-22 (#3397) for exactly this reason.

To scaffold an article from a CHANGELOG entry:

```bash
bash scripts/changelog-to-article.sh <YYYY.M.D> [<git-tag>]
```

The script slices the matching `## [YYYY.M.D]` section out of `CHANGELOG.md`
and writes `articles/release-<YYYY.M.D>.md` with the front matter shape
expected by `devto-publish.yml`. The optional second argument overrides the
default `v<YYYY.M.D>` placeholder for `canonical_url` — pass the real CalVer
tag (e.g. `v2026.4.27-beta6`) when you have it. Review the file, hand-edit
the lead paragraph if the release deserves a narrative beyond the bullet
list, then commit alongside the CHANGELOG bump.

---

## Pull Request Process

1. **Fork and branch**: Create a feature branch from `main`. Use descriptive names like `feat/add-matrix-adapter` or `fix/session-restore-crash`.

2. **Make your changes**: Follow the code style guidelines above.

3. Test thoroughly:
   - `cargo test --workspace` must pass (all 2,100+ tests).
   - `cargo clippy --workspace --all-targets -- -D warnings` must produce zero warnings.
   - `cargo fmt --all --check` must produce no diff.

4. **Write a clear PR description**: Explain what changed and why. Include before/after examples if applicable.

5. **One concern per PR**: Keep PRs focused. A single PR should address one feature, one bug fix, or one refactor -- not all three.

6. **Review process**: At least one maintainer must approve before merge. Maintainers give an initial response within 7 days. If your PR needs changes, we provide specific, actionable suggestions — we don't leave you guessing. Contributor attribution is always preserved. See `GOVERNANCE.md` for full project policy.

7. **CI must pass**: All automated checks must be green before merge.

### Commit Messages

Use clear, imperative-mood messages:

```
Add Matrix channel adapter with E2EE support
Fix session restore crash on kernel reboot
Refactor capability manager to use DashMap
```

---

## Code of Conduct

This project follows the local [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). By participating, you agree to uphold a welcoming, inclusive, and harassment-free environment for everyone.

Please report unacceptable behavior to the maintainers.

---

## Questions?

- Ask in [GitHub Discussions](https://github.com/librefang/librefang/discussions) for questions or ideas.
- Open a [GitHub Issue](https://github.com/librefang/librefang/issues) for bugs or feature requests.
- Check the [docs/](docs/) directory for detailed guides on specific topics.
- Read [GOVERNANCE.md](GOVERNANCE.md) for decision-making, maintainer expectations, and attribution rules.
