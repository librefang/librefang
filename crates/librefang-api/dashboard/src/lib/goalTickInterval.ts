/**
 * Bounds and default for a goal run's tick interval.
 *
 * Mirrors `MIN_GOAL_TICK_INTERVAL_SECS`, `MAX_GOAL_TICK_INTERVAL_SECS` and
 * `DEFAULT_GOAL_TICK_INTERVAL_SECS` in `crates/librefang-types/src/goal.rs`,
 * which is what `validate_tick_interval` in `routes/goals.rs` enforces.
 *
 * They live here rather than inline because the three numbers were spelled out
 * in the create input, the edit input, and the "(default 2)" baked into five
 * translated placeholders. None of those fail loudly when the Rust constants
 * move: the dashboard would go on refusing a value the API now accepts, or
 * admit one it now rejects and surface the 400 as a toast the operator cannot
 * act on. One copy is still a copy, but it is a copy with a single place to
 * fix and a test that names its counterpart.
 */
export const MIN_GOAL_TICK_INTERVAL_SECS = 1;
export const MAX_GOAL_TICK_INTERVAL_SECS = 86400;
export const DEFAULT_GOAL_TICK_INTERVAL_SECS = 2;

/**
 * Read a cadence out of a form field.
 *
 * Returns `null` for a blank field — the API takes an absent value as "use the
 * default" — the number itself for a cadence the API will accept, and
 * `undefined` for anything it would refuse.
 *
 * The third case is the reason this exists. The edit row is not a `<form>`, so
 * the `min` / `max` on its input never trigger constraint validation, and a
 * rejected cadence used to travel to the server inside the same payload as the
 * title, status, progress and agent changes. `validate_tick_interval` runs
 * before the `structured_modify` transaction, so the whole edit was discarded
 * over one bad field.
 */
export function parseGoalTickInterval(raw: string): number | null | undefined {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed)) return undefined;
  if (parsed < MIN_GOAL_TICK_INTERVAL_SECS || parsed > MAX_GOAL_TICK_INTERVAL_SECS) {
    return undefined;
  }
  return parsed;
}
