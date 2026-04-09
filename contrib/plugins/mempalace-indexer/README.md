# mempalace-indexer

LibreFang context engine plugin that integrates [MemPalace](https://github.com/milla-jovovich/mempalace) for persistent, semantic agent memory. No API keys, no cloud — fully local.

## Quick start

```bash
# Install plugin + Python dependencies
librefang plugin install mempalace-indexer --from local contrib/plugins/mempalace-indexer
librefang plugin requirements mempalace-indexer

# Initialize and index your data
mempalace init /path/to/workspace --yes
mempalace mine /path/to/workspace

# (Optional) Add MCP server for 19 explicit memory tools
# Add to config.toml:
# [[mcp_servers]]
# name = "mempalace"
# timeout_secs = 60
# [mcp_servers.transport]
# type = "stdio"
# command = "python3"
# args = ["-m", "mempalace.mcp_server"]
```

Restart the daemon. Done.

## What it does

| Hook | When | What |
|------|------|------|
| `ingest` | Message arrives | Searches palace for relevant memories, injects into prompt |
| `after_turn` | After LLM responds | Saves memorable turns (decisions, events, contacts) automatically |

The after_turn hook is a safety net — the agent can also save explicitly via `mcp_mempalace_add_drawer`. When it does, the hook skips that turn (no duplicates).

## Filtering

Not everything gets saved. Three filters run before indexing:

1. **Dedup** — skip if agent already used `mcp_mempalace_add_drawer`
2. **Length** — skip exchanges < 80 chars
3. **Relevance** — skip if no keywords match (decisions, appointments, contacts, preferences, etc.)

Tool calls, stack traces, and code blocks are stripped.

## Configuration

Set `MEMPALACE_PALACE_PATH` env var or create `~/.mempalace/config.json`:

```json
{"palace_path": "/your/persistent/path/palace"}
```

Default: `~/.mempalace/palace`
