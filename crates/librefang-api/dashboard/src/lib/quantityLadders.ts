// Step ladders for the agent manifest's numeric fields.
//
// These replace bare `<input type="number">` boxes — seventeen of them, one
// per quota, heartbeat and threshold the manifest accepts. A bare number asks
// the operator to already know the field's scale: is a token budget 10_000 or
// 10_000_000? Is `max_iterations` a handful or a few hundred? The input
// answered none of that, so the only way to fill it in was to guess and then
// find out from a bill or a stopped agent.
//
// A short ladder plus a custom rung answers it by showing the shape of the
// scale, and keeps the exception reachable. Same reasoning as
// `modelParamLadders.ts`, which did this for the sampling parameters.
//
// The ladders are deliberately coarse. A rung is an order of magnitude, not a
// precise setting: an operator who needs 45_000 types it into the custom field,
// and one who has no opinion picks from a scale they can read.

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

/**
 * Decimal counts — "10K", "100K", "1M".
 *
 * Deliberately not `formatTokens`, which is binary and renders 1_000_000 as
 * "976K". Token *budgets* are chosen in round decimal figures, and rendering
 * the rung as something other than the number it stores makes the control look
 * wrong about its own value.
 */
export function formatCount(value: number): string {
  if (value >= 1_000_000_000) return `${value / 1_000_000_000}G`;
  if (value >= 1_000_000) return `${value / 1_000_000}M`;
  if (value >= 1_000) return `${value / 1_000}K`;
  return String(value);
}

/** Binary sizes, in the units the quota is enforced in — "256 MB", "1 GB". */
export function formatBytes(value: number): string {
  if (value >= GB) return `${value / GB} GB`;
  if (value >= MB) return `${value / MB} MB`;
  if (value >= KB) return `${value / KB} KB`;
  return `${value} B`;
}

/** Durations held in seconds — "30 s", "5 min", "2 h". */
export function formatSeconds(value: number): string {
  if (value >= 3600 && value % 3600 === 0) return `${value / 3600} h`;
  if (value >= 60 && value % 60 === 0) return `${value / 60} min`;
  return `${value} s`;
}

/** Durations held in milliseconds — "500 ms", "5 s", "2 min". */
export function formatMillis(value: number): string {
  if (value >= 60_000 && value % 60_000 === 0) return `${value / 60_000} min`;
  if (value >= 1000 && value % 1000 === 0) return `${value / 1000} s`;
  return `${value} ms`;
}

/** Dollar amounts — "$0.10", "$5", "$1K". */
export function formatUsd(value: number): string {
  if (value >= 1000 && value % 1000 === 0) return `$${formatCount(value)}`;
  return `$${value}`;
}

// ---------------------------------------------------------------- budgets

/**
 * LLM tokens per hour. Rungs are the order-of-magnitude sequence: an idle
 * agent on a small model sits near the bottom, a batch worker near the top.
 */
export const LLM_TOKENS_PER_HOUR_LADDER = [
  10_000, 100_000, 1_000_000, 10_000_000, 100_000_000,
] as const;

/** Tool calls per minute. */
export const TOOL_CALLS_PER_MINUTE_LADDER = [5, 10, 30, 60, 120, 300, 600] as const;

/**
 * Spend caps, hourly / daily / monthly.
 *
 * Three ladders rather than one because each is read against a different
 * period: $50/h is a runaway agent, $50/month is a shoestring budget.
 */
export const COST_PER_HOUR_LADDER = [0.1, 0.5, 1, 5, 10, 50, 100] as const;
export const COST_PER_DAY_LADDER = [1, 5, 10, 25, 50, 100, 500] as const;
export const COST_PER_MONTH_LADDER = [10, 50, 100, 500, 1000, 5000, 10_000] as const;

/** Bytes pulled over the network per rolling hour. */
export const NETWORK_BYTES_PER_HOUR_LADDER = [
  10 * MB, 100 * MB, 500 * MB, 1 * GB, 5 * GB, 10 * GB,
] as const;

/** WASM heap ceiling for the agent's module. */
export const MEMORY_BYTES_LADDER = [
  64 * MB, 128 * MB, 256 * MB, 512 * MB, 1 * GB, 2 * GB, 4 * GB,
] as const;

/** CPU time per invocation, in milliseconds. */
export const CPU_TIME_MS_LADDER = [1000, 5000, 10_000, 30_000, 60_000, 300_000] as const;

// ------------------------------------------------------------- lifecycle

/**
 * Messages kept in the trimmed history.
 *
 * The kernel's compiled default is 60 and it clamps anything below 4, so the
 * ladder starts above the point where the value stops meaning what the
 * operator expects and stops being applied.
 */
export const MAX_HISTORY_MESSAGES_LADDER = [20, 40, 60, 100, 200, 500] as const;

/**
 * The floor the runtime raises a lower value to
 * (`agent_loop::history::MIN_HISTORY_MESSAGES`). Bound to the custom field so
 * the control cannot offer a number the runtime will silently change, and
 * named here rather than typed as a literal so the two stay findable together.
 */
export const MIN_HISTORY_MESSAGES = 4;

/**
 * Concurrent invocations of one agent.
 *
 * Deliberately small: a `persistent` session clamps this to 1 whatever is
 * stored, so the useful range is single digits and a ladder that suggested 64
 * would be offering a number the runtime refuses in the common case.
 */
export const MAX_CONCURRENT_INVOCATIONS_LADDER = [1, 2, 4, 8, 16, 32] as const;

/** Cosine similarity floor. A fraction, so the rungs are fractions. */
export const MIN_SIMILARITY_LADDER = [0.3, 0.5, 0.6, 0.7, 0.8, 0.9] as const;

/**
 * Fraction of the hourly token budget an agent may spend in any single minute.
 *
 * Not `MIN_SIMILARITY_LADDER`: the compiled default is 0.2 and the runtime
 * clamps to 0.01..=1.0, so the rungs are the ones an operator is choosing
 * between — a fifth, a quarter, half, or the whole hourly budget in one minute,
 * which is the "no burst restriction" end.
 */
export const BURST_RATIO_LADDER = [0.05, 0.1, 0.2, 0.25, 0.5, 1] as const;

/** Durations held in hours — "6 h", "1 day". */
export function formatHours(value: number): string {
  if (value >= 24 && value % 24 === 0) return `${value / 24} d`;
  return `${value} h`;
}

/** Fractions as percentages — "70%", which is how a floor is talked about. */
export function formatPercent(value: number): string {
  return `${Math.round(value * 100)}%`;
}

/** Pending skill candidates retained before the oldest is dropped. */
export const SKILL_WORKSHOP_MAX_PENDING_LADDER = [5, 10, 25, 50, 100] as const;

/** Age, in days, past which an unapproved candidate expires. */
export const SKILL_WORKSHOP_MAX_AGE_LADDER = [1, 7, 14, 30, 90] as const;

// ------------------------------------------------------- channel overrides

/** Messages a channel may send per minute, per channel and per user. */
export const CHANNEL_RATE_LIMIT_LADDER = [1, 5, 10, 30, 60, 120] as const;

/** How long a channel waits for the next message before answering. */
export const CHANNEL_DEBOUNCE_MS_LADDER = [0, 250, 500, 1000, 2000, 5000] as const;

/** Debounce window ceiling, in milliseconds. */
export const CHANNEL_DEBOUNCE_MAX_LADDER = [1000, 2000, 5000, 10_000, 30_000] as const;

/** Messages buffered during a debounce window. */
export const CHANNEL_DEBOUNCE_BUFFER_LADDER = [8, 16, 32, 64, 128] as const;

/** Minutes an auto-routed conversation stays on its agent. */
export const CHANNEL_ROUTE_TTL_LADDER = [5, 15, 30, 60, 120, 240] as const;

/** Confidence a routing candidate needs, on the router's own scale. */
export const CHANNEL_ROUTE_CONFIDENCE_LADDER = [1, 3, 5, 7, 9] as const;

/** Bonus applied to the incumbent agent, so routing does not flap. */
export const CHANNEL_ROUTE_BONUS_LADDER = [1, 2, 4, 6, 8] as const;

/** How far a contender must diverge before it takes over. */
export const CHANNEL_ROUTE_DIVERGENCE_LADDER = [1, 2, 3, 5, 8] as const;

/** Seconds a thread stays owned by the agent that started it. */
export const CHANNEL_THREAD_OWNERSHIP_TTL_LADDER = [60, 300, 600, 1800, 3600] as const;

// ------------------------------------------------------------ compaction

/** Message count that triggers compaction. */
export const COMPACTION_THRESHOLD_LADDER = [10, 20, 40, 60, 100, 200] as const;

/** Recent messages preserved verbatim through a compaction. */
export const COMPACTION_KEEP_RECENT_LADDER = [3, 5, 10, 20, 40, 60] as const;

/** Token budget for the summary the compaction produces. */
export const COMPACTION_SUMMARY_TOKENS_LADDER = [1024, 2048, 4096, 8192] as const;

/** Fraction of the context window that triggers token-based compaction. */
export const COMPACTION_TOKEN_RATIO_LADDER = [0.5, 0.6, 0.7, 0.8, 0.9] as const;

/** Chars per summarisation chunk. */
export const COMPACTION_CHUNK_CHARS_LADDER = [2000, 4000, 8000, 16000] as const;

/** Retry attempts for a summarisation call that fails. */
export const COMPACTION_MAX_RETRIES_LADDER = [1, 2, 3, 5] as const;

/** Consecutive developer-tool steps before they are collapsed into one. */
export const COMPACTION_LOOP_STEPS_LADDER = [2, 3, 5, 10, 20] as const;

/** Age, in turns, past which an assistant message loses its reasoning. */
export const COMPACTION_STRIP_REASONING_LADDER = [0, 1, 2, 5, 10] as const;

// -------------------------------------------------------------- memory

/**
 * How long an agent must have been idle before it may dream.
 *
 * Hours, and coarse: the value exists to stop an agent consolidating on every
 * turn, so the useful range is "a few hours" to "a few days" and nothing in
 * between means anything.
 */
export const AUTO_DREAM_MIN_HOURS_LADDER = [1, 6, 12, 24, 48, 72] as const;

/** How many sessions must have accumulated before it may dream. */
export const AUTO_DREAM_MIN_SESSIONS_LADDER = [1, 5, 10, 25, 50, 100] as const;

/** Wall-clock timeout for the async tasks an agent spawns. */
export const ASYNC_TASK_TIMEOUT_LADDER = [30, 60, 300, 900, 1800, 3600] as const;

// ------------------------------------------------------------ autonomous

/** Iterations per invocation. */
export const MAX_ITERATIONS_LADDER = [5, 10, 25, 50, 100, 250, 500] as const;

/** Restarts before the agent is stopped for good. */
export const MAX_RESTARTS_LADDER = [0, 1, 2, 3, 5, 10, 25] as const;

/** Heartbeat cadence, in seconds. */
export const HEARTBEAT_INTERVAL_LADDER = [30, 60, 300, 900, 1800, 3600] as const;

/** How long a heartbeat may go unanswered before it counts as missed. */
export const HEARTBEAT_TIMEOUT_LADDER = [30, 60, 120, 300, 600, 1800] as const;

/** Recent heartbeats kept in the agent's context. */
export const HEARTBEAT_KEEP_RECENT_LADDER = [1, 5, 10, 20, 50, 100] as const;

// -------------------------------------------------------------- routing

/**
 * Complexity-router thresholds, in **tokens**.
 *
 * `simple_threshold` is the token count below which a turn is simple and
 * `complex_threshold` the count above which it is complex, so the rungs are
 * token counts and not an abstract score — a rung of "8" would mean nothing.
 * Both share a ladder because they are the same quantity at two cut points;
 * the invariant that simple < complex is the form's to enforce, not the
 * ladder's.
 */
export const ROUTING_THRESHOLD_LADDER = [
  1000, 2000, 4000, 8000, 16_000, 32_000, 64_000, 128_000,
] as const;

// ------------------------------------------------------------- thinking

/** Extended-thinking budget, in tokens. */
export const THINKING_BUDGET_LADDER = [
  1024, 2048, 4096, 8192, 16_384, 32_768, 65_536,
] as const;

// ------------------------------------------------------------- schedule

/** Cadence of a continuous-mode agent, in seconds. */
export const CHECK_INTERVAL_LADDER = [5, 15, 30, 60, 300, 900, 3600] as const;
