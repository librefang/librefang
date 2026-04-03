# Claude Flow Swarm — LibreFang Quick Start

## One-Time Setup

Initialize the swarm with hierarchical-mesh topology (queen-led with fallback mesh):

```bash
npx @claude-flow/cli@latest swarm init \
  --topology hierarchical \
  --max-agents 12 \
  --strategy specialized
```

## Check Status

```bash
npx @claude-flow/cli@latest swarm status
npx @claude-flow/cli@latest swarm monitor
```

## Invoke from Claude Code

### Single Agent Spawn
```bash
npx @claude-flow/cli@latest agent spawn -t coder --name librefang-impl
```

### Batch Spawn (Hive Mind)
```bash
npx @claude-flow/cli@latest hive-mind spawn --count 5 --type worker
```

## Agent Role Matrix (LibreFang 10-Step Workflow)

| Step | Role | Agent | Invocation |
|------|------|-------|-----------|
| 1 | ADR Review | `adr-reviewer` | `agent spawn -t adr-reviewer` |
| 2 | SPEC Validator | `qe-requirements-validator` | Built-in AQE agent |
| 3 | Sherlock Baseline | `sherlock-review` | `agent spawn -t sherlock` |
| 4 | PLAN Creator | `plan-generator` | Part of flow-coach |
| 5 | AQE Pre-Code | `devils-advocate`, `impact-analyzer` | Built-in AQE agents |
| 6-7 | RED → GREEN | `coder`, `tester` | Spawn both, coordinate |
| 8 | REFACTOR | `refactoring-agent` | `agent spawn -t refactor` |
| 9 | Review | `code-review-agent` | Built-in + peer review |
| 10 | Final Sherlock | `sherlock-review` | Re-run for verdict |

## Topology: Hierarchical-Mesh

```
        👑 QUEEN (Coordinator)
        ├── Coder Agent
        ├── Tester Agent
        ├── Reviewer Agent
        └── Validator Agent

    + Mesh fallback if queen unavailable
```

**Auto-scale**: Up to 12 agents (adjustable in `.claude-flow/config.yaml`)

## Memory Backend

- **HNSW Indexing**: 768-dim embeddings (all-MiniLM-L6-v2)
- **Learning Bridge**: Enabled (SONA mode: balanced)
- **Consolidation**: Every 10 memory accesses
- **Max scope**: 5000 nodes per project

## MCP Tools Available

```javascript
// Initialize swarm
mcp__claude-flow__swarm_init(topology, maxAgents, strategy)

// Spawn an agent
mcp__claude-flow__agent_spawn(role, capabilities, scope)

// Orchestrate multi-agent task
mcp__claude-flow__task_orchestrate(swarmId, task, constraints)

// Check memory usage
mcp__claude-flow__memory_usage(agentId)

// Coordinate between agents
mcp__claude-flow__coordination_sync(swarmId, state)

// Load balance work
mcp__claude-flow__load_balance(swarmId)
```

## Workflow Example: Implement a New Endpoint

```bash
# 1. Start swarm
npx @claude-flow/cli@latest swarm init --topology hierarchical --max-agents 8

# 2. Spawn agents for the 10-step workflow
npx @claude-flow/cli@latest agent spawn -t requirements-validator --name step-2
npx @claude-flow/cli@latest agent spawn -t sherlock --name step-3
npx @claude-flow/cli@latest agent spawn -t coder --name step-6-7
npx @claude-flow/cli@latest agent spawn -t tester --name step-7
npx @claude-flow/cli@latest agent spawn -t code-review --name step-9

# 3. Orchestrate task
mcp__claude-flow__task_orchestrate \
  --swarmId=librefang-main \
  --task="implement new budget endpoint" \
  --constraints="10-step workflow, zero clippy warnings, 2100+ tests pass"

# 4. Monitor progress
npx @claude-flow/cli@latest swarm monitor
```

## Persistence

- Swarm state: `.claude-flow/swarm/`
- Session logs: `.claude-flow/sessions/`
- Memory: `.claude-flow/data/` (hybrid backend)
- Metrics: `.claude-flow/metrics/`

All persisted on-disk (no external DB required).

## Config Location

`/Users/danielalberttis/Desktop/Projects/librefang/.claude-flow/config.yaml`

Edit `maxAgents` or `coordinationStrategy` directly — hot-reloads on next command.
