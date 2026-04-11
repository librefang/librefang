# ADR-CODE-001: Large File Decomposition Strategy

**Status:** Proposed
**Date:** 2026-04-10
**Related:** `AGENTS.md`, `docs/multi-tenant/CURRENT-CODE-AUDIT.md`

---

## Decision

LibreFang will adopt a codebase-wide file decomposition policy for Rust source
files.

The target is not "small files at any cost." The target is files with one
dominant responsibility, predictable review scope, and stable extraction
boundaries.

Effective policy:

- `500` lines is the target upper bound for normal handwritten Rust modules
- `800` lines is the hard ceiling for new or substantially rewritten files
- `1500+` lines is a mandatory decomposition threshold unless explicitly exempt
- existing oversized files will be reduced through phased, mechanical
  extraction, not broad rewrites

For current code, the decomposition program will be driven by responsibility
boundaries first, line count second.

---

## Problem

LibreFang currently has a large-file concentration problem, not a one-off
outlier problem.

Current repository counts for `crates/**/*.rs`:

- `288` files exceed `500` lines
- `81` files exceed `1000` lines
- `28` files exceed `2000` lines
- `7` files exceed `5000` lines

The most severe current offenders are:

| File | Lines | Class |
|---|---:|---|
| `crates/librefang-kernel/src/kernel.rs` | 14,205 | kernel god object / orchestration sink |
| `crates/librefang-cli/src/main.rs` | 10,704 | command dispatch sink |
| `crates/librefang-runtime/src/agent_loop.rs` | 6,464 | runtime execution pipeline |
| `crates/librefang-runtime/src/tool_runner.rs` | 6,435 | tool execution switchboard |
| `crates/librefang-api/src/routes/agents.rs` | 6,195 | route aggregator |
| `crates/librefang-types/src/config/types.rs` | 5,397 | schema accretion sink |
| `crates/librefang-api/src/routes/system.rs` | 5,065 | mixed-scope route aggregator |

This causes predictable engineering problems:

- review scope is too broad and hides regressions
- unrelated changes collide in the same files
- extraction gets harder over time because the file becomes the default sink
- test location and ownership become unclear
- route, runtime, and config policy drift are harder to detect

The goal of this ADR is to create one concrete plan across the codebase instead
of handling each file ad hoc.

---

## Scope

This ADR applies to handwritten Rust source files under `crates/`.

It governs:

- decomposition thresholds
- allowed exceptions
- module extraction rules
- sequencing for the current oversized files
- enforcement expectations for future changes

It does not require:

- public API redesign where mechanical extraction is sufficient
- crate reshuffling as the first step
- style churn unrelated to module boundaries

---

## Classification

Oversized files in LibreFang fall into a small number of recurring classes.

### 1. Aggregator files

These collect many unrelated endpoints, commands, or handlers into one module.

Examples:

- `librefang-cli/src/main.rs`
- `librefang-api/src/routes/agents.rs`
- `librefang-api/src/routes/system.rs`
- `librefang-api/src/routes/skills.rs`
- `librefang-api/src/routes/channels.rs`

Preferred fix:

- keep one thin top-level router or command registration module
- move handlers into sibling modules by subdomain
- leave route paths and CLI UX unchanged during extraction

### 2. Orchestrator files

These own a central type and accumulate unrelated behavior via large impl
blocks.

Examples:

- `librefang-kernel/src/kernel.rs`
- `librefang-kernel/src/workflow.rs`
- `librefang-kernel/src/cron.rs`

Preferred fix:

- keep the primary type definition stable
- move impl blocks and helpers into `mod` siblings under a directory module
- avoid splitting the owning struct before behavior is separated

### 3. Pipeline files

These encode an execution flow with retries, parsing, normalization, and side
effects in one place.

Examples:

- `librefang-runtime/src/agent_loop.rs`
- `librefang-runtime/src/tool_runner.rs`
- `librefang-memory/src/proactive.rs`

Preferred fix:

- isolate stage helpers from the main loop
- separate parsing/normalization from side-effect execution
- isolate provider- or tool-family-specific behavior behind focused modules

### 4. Schema sink files

These accumulate many config or model structs over time.

Examples:

- `librefang-types/src/config/types.rs`
- `librefang-types/src/agent.rs`
- `librefang-types/src/memory.rs`

Preferred fix:

- split by domain, not by arbitrary alphabetic grouping
- keep one top-level re-export module for compatibility
- move validation and defaulting close to the owned type group

### 5. Parser / protocol / generated-adjacent files

These can be large for legitimate reasons.

Examples:

- parser implementations
- protocol compatibility modules
- dense benchmark or test fixtures

Preferred fix:

- isolate them clearly
- document the reason they exceed the normal limit
- do not treat them as justification for large application modules

---

## Rules

1. New files should target `<= 500` lines.
2. New files may exceed `500` lines only when a single responsibility remains
   clear and a split would be artificial.
3. New or substantially rewritten files must not exceed `800` lines without an
   explicit note in the PR or task record.
4. Files `> 1500` lines require a decomposition plan before feature growth
   continues in that file.
5. Mechanical extraction is preferred over semantic redesign in the first pass.
6. Top-level router and command modules may remain as thin registries, but
   handler bodies should live in focused submodules.
7. Inline test modules in very large files should be moved out early unless the
   tests require unusually tight locality.
8. Shared helpers should move to domain modules, not generic `utils.rs`
   dumping grounds.
9. Line count alone does not define correctness; responsibility coherence does.
10. Exemptions must be explicit and local to the file family, not assumed
    globally.

---

## Exceptions

The following categories may exceed the normal thresholds when justified:

- parser and grammar implementations
- benchmark-heavy files
- integration test harnesses with scenario-heavy setup
- generated or generated-adjacent code
- tightly coupled protocol compatibility tables

Even when exempt, these files should still be reviewed for obvious internal
module boundaries.

Application orchestration, CLI dispatch, route modules, and config schemas are
not exempt categories.

---

## Current Priority Inventory

The decomposition program will start with the files whose size and change
surface create the highest daily engineering cost.

| File | Lines | Primary issue | Required first extraction boundary |
|---|---:|---|---|
| `crates/librefang-kernel/src/kernel.rs` | 14,205 | central orchestrator accumulated unrelated impls | split impl blocks into `kernel/` submodules |
| `crates/librefang-cli/src/main.rs` | 10,704 | command dispatch sink | move command handlers to `commands/` modules |
| `crates/librefang-runtime/src/agent_loop.rs` | 6,464 | mixed loop, retry, parsing, recovery | separate loop stages, retries, tool-call recovery |
| `crates/librefang-runtime/src/tool_runner.rs` | 6,435 | one giant tool switchboard | split by tool family and execution plumbing |
| `crates/librefang-api/src/routes/agents.rs` | 6,195 | many endpoint classes in one file | split CRUD, sessions, files, messaging, streaming |
| `crates/librefang-types/src/config/types.rs` | 5,397 | schema accretion sink | split config by domain and re-export |
| `crates/librefang-api/src/routes/system.rs` | 5,065 | mixed public/admin/system concerns | split audit, approvals, sessions, backups, registry |
| `crates/librefang-api/src/routes/skills.rs` | 4,911 | skills, hands, MCP, integrations, extensions mixed | split by product surface |
| `crates/librefang-migrate/src/openclaw.rs` | 4,673 | migration pipeline concentration | split parsing, mapping, import, reporting |
| `crates/librefang-channels/src/bridge.rs` | 4,618 | startup, routing, registry, lifecycle mixed | split bridge boot, adapter lifecycle, ingress routing |
| `crates/librefang-api/src/routes/channels.rs` | 4,219 | config, bootstrap, runtime ops mixed | split config CRUD, bootstrap, diagnostics, testing |
| `crates/librefang-api/src/channel_bridge.rs` | 4,153 | channel API bridge sink | split request mapping, runtime bridge, ownership logic |
| `crates/librefang-kernel/src/workflow.rs` | 4,035 | definitions, execution, persistence, templates mixed | split engine, store, run state, templates |
| `crates/librefang-memory/src/proactive.rs` | 3,865 | proactive memory policy sink | split retrieval, scoring, summarization, persistence |

---

## Concrete Module Targets

### `crates/librefang-kernel/src/kernel.rs`

Observed structure:

- giant `LibreFangKernel` definition
- multiple unrelated `impl LibreFangKernel` blocks
- boot logic, routing, messaging, tool availability, prompt assembly,
  provider sync, workflow helpers, approval helpers, `KernelHandle`, and tests
  all in one file

Target shape:

- `kernel/mod.rs`: struct, core types, public exports
- `kernel/boot.rs`
- `kernel/facade.rs`
- `kernel/agents.rs`
- `kernel/messaging.rs`
- `kernel/providers.rs`
- `kernel/tools.rs`
- `kernel/prompting.rs`
- `kernel/workflows.rs`
- `kernel/notifications.rs`
- `kernel/kernel_handle.rs`
- `kernel/tests.rs`

Rule:

- keep `LibreFangKernel` as the owning type through phase 1

### `crates/librefang-cli/src/main.rs`

Observed structure:

- clap type definitions
- daemon management
- init and config flows
- agent commands
- channel commands
- hand commands
- workflow commands
- vault, auth, models, approvals, cron, webhooks, service, update, uninstall

Target shape:

- `main.rs`: clap structs, global bootstrapping, top-level dispatch only
- `commands/init.rs`
- `commands/daemon.rs`
- `commands/agent.rs`
- `commands/channel.rs`
- `commands/hand.rs`
- `commands/config.rs`
- `commands/models.rs`
- `commands/skill.rs`
- `commands/system.rs`
- `commands/service.rs`
- `commands/update.rs`
- `commands/webhooks.rs`

Rule:

- do not change CLI command names or help surface during extraction

### `crates/librefang-runtime/src/agent_loop.rs`

Observed structure:

- message normalization
- approval signal handling
- model ID normalization
- main loop execution
- retry logic
- streaming path
- hallucinated tool-call recovery
- multiple ad hoc parsers

Target shape:

- `agent_loop/mod.rs`
- `agent_loop/message_prep.rs`
- `agent_loop/model_ids.rs`
- `agent_loop/retry.rs`
- `agent_loop/non_streaming.rs`
- `agent_loop/streaming.rs`
- `agent_loop/tool_call_recovery.rs`
- `agent_loop/parsing.rs`

Rule:

- preserve the current external entry points:
  `run_agent_loop`, `run_agent_loop_streaming`, `strip_provider_prefix`

### `crates/librefang-runtime/src/tool_runner.rs`

Observed structure:

- policy checks
- tool dispatch
- builtin tool definition inventory
- many tool-family implementations inline

Target shape:

- `tool_runner/mod.rs`
- `tool_runner/dispatch.rs`
- `tool_runner/definitions.rs`
- `tool_runner/files.rs`
- `tool_runner/web.rs`
- `tool_runner/agents.rs`
- `tool_runner/memory.rs`
- `tool_runner/scheduling.rs`
- `tool_runner/channels.rs`
- `tool_runner/media.rs`
- `tool_runner/process.rs`
- `tool_runner/canvas.rs`

Rule:

- no tool schema or user-visible tool name changes in phase 1

### `crates/librefang-api/src/routes/agents.rs`

Target split:

- `routes/agents/mod.rs`
- `routes/agents/crud.rs`
- `routes/agents/messaging.rs`
- `routes/agents/streaming.rs`
- `routes/agents/sessions.rs`
- `routes/agents/files.rs`
- `routes/agents/uploads.rs`
- `routes/agents/config.rs`
- `routes/agents/metrics.rs`

### `crates/librefang-api/src/routes/system.rs`

Target split:

- `routes/system/mod.rs`
- `routes/system/profiles.rs`
- `routes/system/audit.rs`
- `routes/system/sessions.rs`
- `routes/system/approvals.rs`
- `routes/system/webhooks.rs`
- `routes/system/backups.rs`
- `routes/system/pairing.rs`
- `routes/system/queue.rs`
- `routes/system/registry.rs`
- `routes/system/tools.rs`

### `crates/librefang-api/src/routes/skills.rs`

Target split:

- `routes/skills/mod.rs`
- `routes/skills/catalog.rs`
- `routes/skills/install.rs`
- `routes/skills/hands.rs`
- `routes/skills/mcp.rs`
- `routes/skills/integrations.rs`
- `routes/skills/extensions.rs`
- `routes/skills/marketplace.rs`

### `crates/librefang-types/src/config/types.rs`

Target split:

- `config/mod.rs`: re-exports and top-level glue
- `config/core.rs`
- `config/network.rs`
- `config/providers.rs`
- `config/memory.rs`
- `config/channels.rs`
- `config/security.rs`
- `config/runtime.rs`
- `config/plugins.rs`
- `config/integrations.rs`
- `config/defaults.rs` only if shared helpers cannot live near owned domains

Rule:

- compatibility stays at the type path level through re-exports

---

## Phased Execution Plan

### Phase 0: Baseline and guardrails

- record the oversized-file inventory in this ADR
- add a repo script or `xtask` check that reports files over `500`, `800`,
  `1500`, and `2000` lines
- do not fail CI yet; report only

### Phase 1: Mechanical extractions in the critical path

Priority order:

1. `librefang-kernel/src/kernel.rs`
2. `librefang-runtime/src/agent_loop.rs`
3. `librefang-runtime/src/tool_runner.rs`
4. `librefang-api/src/routes/agents.rs`
5. `librefang-types/src/config/types.rs`
6. `librefang-api/src/routes/system.rs`
7. `librefang-api/src/routes/skills.rs`
8. `librefang-cli/src/main.rs`

Execution rule:

- move code first
- keep public APIs stable
- run existing tests after each file family split
- avoid semantic rewrites while splitting

### Phase 2: Secondary large-file families

Start after the phase 1 core files are materially reduced:

- `librefang-migrate/src/openclaw.rs`
- `librefang-channels/src/bridge.rs`
- `librefang-api/src/routes/channels.rs`
- `librefang-api/src/channel_bridge.rs`
- `librefang-kernel/src/workflow.rs`
- `librefang-memory/src/proactive.rs`

### Phase 3: Broad normalization

- work through the remaining `> 2000` line files
- reduce route modules and driver modules opportunistically
- move large inline tests where locality no longer adds value

### Phase 4: Enforcement

When the critical path is stable:

- fail CI on newly introduced files over `800` lines
- fail CI on growth of already-flagged files unless the change reduces size or
  is explicitly exempted

---

## Acceptance Criteria

This ADR is considered implemented when all of the following are true:

- the top seven `5000+` line files have approved decomposition plans or have
  been reduced below `5000`
- `kernel.rs`, `agent_loop.rs`, `tool_runner.rs`, and `cli/main.rs` are no
  longer single-file execution sinks
- route modules use thin `mod.rs` registries with handler bodies in submodules
- config schemas are split by domain with stable public re-exports
- CI reports large-file thresholds and prevents regression on new work

---

## Consequences

### Positive

- narrower reviews and clearer ownership
- lower merge conflict pressure in hot files
- easier testing because helper boundaries become explicit
- reduced incentive to keep adding unrelated behavior to central modules
- future ADRs and specs can refer to stable domain modules instead of one sink
  file

### Negative

- short-term churn in imports and module plumbing
- some extraction passes will produce temporarily awkward internal boundaries
- reviewers must resist mixing decomposition with semantic changes

---

## Non-Goals

- reducing every file below `500` immediately
- forcing identical module layouts across all crates
- rewriting working logic merely to satisfy line count
- treating parser-heavy or benchmark-heavy files as equivalent to application
  sinks

---

## Implementation Notes

For the critical path files, the first extraction should usually be whichever of
these is easiest:

- move inline tests out
- move helper functions out
- move large impl blocks into sibling modules
- move handler families into submodules behind a thin router

The first pass should change file ownership boundaries more than behavior.
