# Goals Subsystem — PRD: Bugs, Gaps & Defensive Improvements

2026-07-29 — 9-agent parallel audit, 967K tokens, 202 tool calls

## Critical

### CRITICAL #1: Orphaned `RunHandle` on early-exit path in `start()`
**FILE:** `crates/librefang-kernel/src/goal_runner.rs:317-348`
tokio::spawn runs BEFORE DashMap::insert. If run_loop exits immediately,
remove_if is no-op, then insert creates orphaned stale entry.
**FIX:** Swap — insert into runs BEFORE tokio::spawn.

### CRITICAL #2: TUI create form renders no visible text input  
**FILE:** `crates/librefang-cli/src/tui/screens/goals.rs:630`
`_value` (underscore) suppresses rendering. Users type blind.
**FIX:** Bind to `value`, render into chunk[6], move hint elsewhere.

## High

### HIGH #1: Depth-based tool filtering is dead code
**FILE:** `crates/librefang-kernel/src/kernel/tools_and_skills.rs:312`
Production path hardcodes `depth: 0`. SUBAGENT_DENY_LEAF never fires.
**FIX:** Thread real depth or add AGENT_CALL_DEPTH guard to agent_spawn.

## Medium

### MEDIUM #1: Concurrent start() calls orphan a task (token drain)
### MEDIUM #2: No error-streak cap for permanent errors (burns all iterations)
### MEDIUM #3: patch_goal silently succeeds on missing goal
### MEDIUM #4: load_goal silently discards schema-mismatch errors
### MEDIUM #5: Error-iteration accounting asymmetry
### MEDIUM #6: get_goal_children returns 200 with leaked error
### MEDIUM #7: list_goals silently returns empty on storage failure
### MEDIUM #8: GOAL_BLOCKED conflated with operator stop
### MEDIUM #9: "Goal vanished" maps to Finished (should be Stopped)
### MEDIUM #10-16: TUI UX issues (no validation, Enter doesn't clear filter, selection resets, submit hint stale, edit shows raw keys, empty goallist has no "create" action)
### MEDIUM #17: Missing translation keys (goals.status, goals.progress)
### MEDIUM #18: Create form hidden in empty state (ALREADY FIXED)
### MEDIUM #19: No CLI /goal command (ALREADY CREATED on librefang_v2)

## Low (10 items)

TOCTOU race in start_goal_run, delete doesn't stop run, structured_modify error ignored,
agent_id not validated as UUID at write time, GoalRunState lacks #[serde(default)],
Goal.status lacks #[serde(default)], whitespace-only title passes, completed_at missing,
shutdown_rx.borrow() latent panic, unparseable agent_id in recovery.
