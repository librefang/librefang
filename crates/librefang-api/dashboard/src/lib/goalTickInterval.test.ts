import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import {
  DEFAULT_GOAL_TICK_INTERVAL_SECS,
  MAX_GOAL_TICK_INTERVAL_SECS,
  MIN_GOAL_TICK_INTERVAL_SECS,
  parseGoalTickInterval,
} from "./goalTickInterval";

// Without this the dashboard copy is free to drift: it would go on refusing a
// value the API has started accepting, or admit one it has started refusing and
// surface the 400 as a toast the operator cannot act on. Same approach as
// `status.test.ts`, which mirrors `AuthStatus::is_available` out of the Rust
// source rather than trusting a second copy of the list.
describe("the dashboard's copy of the cadence bounds", () => {
  it("matches the constants librefang-types actually declares", () => {
    const source = readFileSync(
      resolve(process.cwd(), "../../librefang-types/src/goal.rs"),
      "utf8",
    );
    const declared = (name: string): number => {
      const match = source.match(
        new RegExp(`pub const ${name}: u64 = (\\d+);`),
      );
      expect(match, `${name} must be discoverable in goal.rs`).toBeTruthy();
      return Number(match![1]);
    };

    expect(MIN_GOAL_TICK_INTERVAL_SECS).toBe(declared("MIN_GOAL_TICK_INTERVAL_SECS"));
    expect(MAX_GOAL_TICK_INTERVAL_SECS).toBe(declared("MAX_GOAL_TICK_INTERVAL_SECS"));
    expect(DEFAULT_GOAL_TICK_INTERVAL_SECS).toBe(
      declared("DEFAULT_GOAL_TICK_INTERVAL_SECS"),
    );
  });
});

describe("parseGoalTickInterval", () => {
  it("reads a blank field as the backend's 'use the default' signal", () => {
    expect(parseGoalTickInterval("")).toBeNull();
    expect(parseGoalTickInterval("   ")).toBeNull();
  });

  it("accepts both ends of the range the API enforces", () => {
    expect(parseGoalTickInterval(String(MIN_GOAL_TICK_INTERVAL_SECS))).toBe(
      MIN_GOAL_TICK_INTERVAL_SECS,
    );
    expect(parseGoalTickInterval(String(MAX_GOAL_TICK_INTERVAL_SECS))).toBe(
      MAX_GOAL_TICK_INTERVAL_SECS,
    );
    expect(parseGoalTickInterval(" 900 ")).toBe(900);
  });

  // Each of these reached `validate_tick_interval` and came back a 400 that
  // took the rest of the edit with it.
  it.each([
    ["one under the floor", String(MIN_GOAL_TICK_INTERVAL_SECS - 1)],
    ["one over the ceiling", String(MAX_GOAL_TICK_INTERVAL_SECS + 1)],
    ["a fraction", "1.5"],
    ["a negative", "-1"],
    ["not a number at all", "soon"],
  ])("refuses %s rather than spending a request on it", (_label, raw) => {
    expect(parseGoalTickInterval(raw)).toBeUndefined();
  });

  it("keeps a default that sits inside its own bounds", () => {
    expect(DEFAULT_GOAL_TICK_INTERVAL_SECS).toBeGreaterThanOrEqual(MIN_GOAL_TICK_INTERVAL_SECS);
    expect(DEFAULT_GOAL_TICK_INTERVAL_SECS).toBeLessThanOrEqual(MAX_GOAL_TICK_INTERVAL_SECS);
  });
});
