# Manifest-first control plane: MCP allowlist vs server registry

This note records the **current** architecture, not a proposal. It exists
because the split between `AgentManifest.mcp_servers` and
`KernelConfig.mcp_servers` looks redundant on first reading; the two
fields hold different things and live at different layers.

## The three values

| Where | Type | What it holds |
|---|---|---|
| `AgentManifest.mcp_servers` | `Vec<String>` (server **names**) | Per-agent **allowlist**. Empty = "all configured servers". Lives in `agent.toml`. |
| `KernelConfig.mcp_servers` | `Vec<McpServerConfigEntry>` (full configs) | Global **registry** of installed servers — transport, env, OAuth, taint policy, headers. Lives in `~/.librefang/config.toml`. |
| `kernel.mcp.effective_mcp_servers` | `RwLock<Vec<McpServerConfigEntry>>` | Hot-reloadable runtime mirror of `KernelConfig.mcp_servers`. Bumped together with `mcp_generation` so cached prompt summaries invalidate atomically. |

`crates/librefang-types/src/agent.rs:803` — manifest field.
`crates/librefang-kernel/src/kernel/subsystems/mcp.rs:63` — runtime mirror.
`crates/librefang-kernel/src/kernel/mcp_setup.rs:324` — `reload_mcp_servers`
copies `cfg.mcp_servers` into `effective_mcp_servers` on hot-reload.

## Resolution

The registry mirror is **not** pre-intersected with any agent's allowlist.
Intersection happens at the prompt boundary, per agent, per turn:

```
render_mcp_summary(tool_names, configured_servers, mcp_allowlist)
```

— `crates/librefang-kernel/src/kernel/prompt_context.rs:281`. The
`configured_servers` argument is the registry snapshot
(`effective_mcp_servers`); the `mcp_allowlist` argument is the agent's
manifest field. An empty allowlist means "every configured server is
visible to this agent".

The same pattern is used elsewhere any time a per-agent view of MCP is
needed (tool list rendering, route handlers): pull the registry
snapshot, then filter through the manifest allowlist. Never store the
intersection.

## Why the split is intentional

- **Different change frequency.** The registry is rewritten when an
  operator installs / uninstalls / reconfigures a server (rare, global,
  hot-reloaded). The allowlist changes when an operator decides which
  agent gets which server (frequent, per-agent, no reconnection).
- **Different blast radius.** Removing a server from the registry tears
  down its connection. Removing it from one agent's allowlist only
  hides it from that agent's prompt — connection stays up for other
  agents.
- **Different ownership.** The registry is what `librefang-extensions`
  manages (catalog → installer → `[[mcp_servers]]` write). The
  allowlist is what the agent author edits in `agent.toml`. Conflating
  the two would put extension installers into the agent-authoring
  surface and vice versa.

## Control-plane boundary for `librefang-extensions`

The extensions crate is the **only** code path that should write the
registry. It owns:

- `catalog::McpCatalog` — read-only set of templates loaded from
  `~/.librefang/mcp/catalog/*.toml`.
- `installer::install_integration` — pure transform from a catalog
  entry + provided credentials to a fresh `McpServerConfigEntry`. Does
  not write — caller persists the entry into `config.toml` and triggers
  a kernel reload.
- `credentials::CredentialResolver` — pulls secrets out of the vault /
  env / dotenv before they're baked into the registry entry.
- `vault`, `oauth`, `health`, `dotenv`, `http_client` — leaf modules
  consumed by the above.

The agent-side allowlist never flows through this crate. Mutating it is
a kernel API operation:
`KernelApi::set_agent_mcp_servers(agent_id, Vec<String>)` —
`crates/librefang-kernel/src/kernel_api.rs:232` — which writes back to
the agent's manifest on disk and bumps the per-agent reload generation.

## Surfaces (HTTP)

- `GET /api/agents/{id}/mcp_servers` — returns the agent's allowlist
  plus the resolved effective list (registry filtered through
  allowlist). `routes/agents.rs:4091`.
- `PUT /api/agents/{id}/mcp_servers` — replaces the agent's allowlist.
  `routes/agents.rs:4162`. Does **not** touch the registry.
- `GET /api/mcp/servers` — registry view (`routes/skills.rs:3972`).
  Used by the dashboard's Integrations page to render install /
  uninstall.

## What this means for new code

- Adding an MCP server to an agent: write to the allowlist via
  `set_agent_mcp_servers`. Do **not** mutate the registry.
- Installing a new server template: go through `install_integration`,
  persist the returned `McpServerConfigEntry`, then call
  `reload_mcp_servers`. Do **not** write to any agent manifest.
- Reading "what MCP tools does agent X see right now": snapshot the
  registry from `effective_mcp_servers`, filter through the agent's
  manifest allowlist. The filtered view is never cached across turns
  beyond the `mcp_summary_cache`, which is keyed on `(allowlist,
  mcp_generation)` so registry hot-reloads invalidate it.
