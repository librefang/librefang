import { describe, expect, it } from "vitest";
import {
  CHECK_INTERVAL_LADDER,
  COST_PER_DAY_LADDER,
  COST_PER_HOUR_LADDER,
  COST_PER_MONTH_LADDER,
  CPU_TIME_MS_LADDER,
  HEARTBEAT_INTERVAL_LADDER,
  HEARTBEAT_KEEP_RECENT_LADDER,
  HEARTBEAT_TIMEOUT_LADDER,
  LLM_TOKENS_PER_HOUR_LADDER,
  MAX_ITERATIONS_LADDER,
  MAX_RESTARTS_LADDER,
  MEMORY_BYTES_LADDER,
  NETWORK_BYTES_PER_HOUR_LADDER,
  ROUTING_THRESHOLD_LADDER,
  THINKING_BUDGET_LADDER,
  TOOL_CALLS_PER_MINUTE_LADDER,
  formatBytes,
  formatCount,
  formatMillis,
  formatSeconds,
  formatUsd,
} from "./quantityLadders";

// A rung is the only thing standing between an operator and a blank number
// box, so a rung that reads wrong is worse than no rung: it names a value the
// control does not store, and the field looks broken about its own contents.
describe("quantity ladders — rung labels", () => {
  it("writes decimal budgets in decimal units", () => {
    // Not `formatTokens`, which is binary: 1_000_000 would render "976K" and
    // the rung would disagree with the number it sets.
    expect(formatCount(1_000_000)).toBe("1M");
    expect(formatCount(100_000)).toBe("100K");
    expect(formatCount(10_000)).toBe("10K");
    expect(formatCount(7)).toBe("7");
  });

  it("writes byte quotas in the units they are enforced in", () => {
    // 268435456 as a rung is a number nobody reads; "256 MB" is the quota.
    expect(formatBytes(256 * 1024 * 1024)).toBe("256 MB");
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1 GB");
    expect(formatBytes(4 * 1024 * 1024 * 1024)).toBe("4 GB");
  });

  it("writes durations in the largest unit that divides evenly", () => {
    expect(formatSeconds(3600)).toBe("1 h");
    expect(formatSeconds(1800)).toBe("30 min");
    expect(formatSeconds(900)).toBe("15 min");
    expect(formatSeconds(45)).toBe("45 s");
    expect(formatMillis(60_000)).toBe("1 min");
    expect(formatMillis(5000)).toBe("5 s");
    expect(formatMillis(500)).toBe("500 ms");
  });

  it("writes money with a currency mark", () => {
    expect(formatUsd(0.1)).toBe("$0.1");
    expect(formatUsd(50)).toBe("$50");
    expect(formatUsd(10_000)).toBe("$10K");
  });
});

describe("quantity ladders — the ladders themselves", () => {
  const ALL: ReadonlyArray<readonly [string, readonly number[]]> = [
    ["LLM_TOKENS_PER_HOUR", LLM_TOKENS_PER_HOUR_LADDER],
    ["TOOL_CALLS_PER_MINUTE", TOOL_CALLS_PER_MINUTE_LADDER],
    ["COST_PER_HOUR", COST_PER_HOUR_LADDER],
    ["COST_PER_DAY", COST_PER_DAY_LADDER],
    ["COST_PER_MONTH", COST_PER_MONTH_LADDER],
    ["NETWORK_BYTES_PER_HOUR", NETWORK_BYTES_PER_HOUR_LADDER],
    ["MEMORY_BYTES", MEMORY_BYTES_LADDER],
    ["CPU_TIME_MS", CPU_TIME_MS_LADDER],
    ["MAX_ITERATIONS", MAX_ITERATIONS_LADDER],
    ["MAX_RESTARTS", MAX_RESTARTS_LADDER],
    ["HEARTBEAT_INTERVAL", HEARTBEAT_INTERVAL_LADDER],
    ["HEARTBEAT_TIMEOUT", HEARTBEAT_TIMEOUT_LADDER],
    ["HEARTBEAT_KEEP_RECENT", HEARTBEAT_KEEP_RECENT_LADDER],
    ["ROUTING_THRESHOLD", ROUTING_THRESHOLD_LADDER],
    ["THINKING_BUDGET", THINKING_BUDGET_LADDER],
    ["CHECK_INTERVAL", CHECK_INTERVAL_LADDER],
  ];

  // `StepLadderInput` renders the rungs in the order it is given and marks the
  // matching one as pressed. An unsorted ladder puts the buttons out of order
  // under a control whose whole point is that the scale is legible.
  it.each(ALL)("%s is ascending", (_name, ladder) => {
    expect([...ladder]).toEqual([...ladder].slice().sort((a, b) => a - b));
  });

  it.each(ALL)("%s has no duplicate rungs", (_name, ladder) => {
    expect(new Set(ladder).size).toBe(ladder.length);
  });

  it.each(ALL)("%s labels every rung distinctly", (name, ladder) => {
    // Two rungs that print the same are two buttons an operator cannot tell
    // apart, and one of them sets a value the other's label claims.
    const formatters = {
      LLM_TOKENS_PER_HOUR: formatCount,
      TOOL_CALLS_PER_MINUTE: formatCount,
      COST_PER_HOUR: formatUsd,
      COST_PER_DAY: formatUsd,
      COST_PER_MONTH: formatUsd,
      NETWORK_BYTES_PER_HOUR: formatBytes,
      MEMORY_BYTES: formatBytes,
      CPU_TIME_MS: formatMillis,
      MAX_ITERATIONS: formatCount,
      MAX_RESTARTS: formatCount,
      HEARTBEAT_INTERVAL: formatSeconds,
      HEARTBEAT_TIMEOUT: formatSeconds,
      HEARTBEAT_KEEP_RECENT: formatCount,
      ROUTING_THRESHOLD: formatCount,
      THINKING_BUDGET: formatCount,
      CHECK_INTERVAL: formatSeconds,
    } as const;
    const fmt = formatters[name as keyof typeof formatters];
    const labels = ladder.map(fmt);
    expect(new Set(labels).size).toBe(labels.length);
  });
});
