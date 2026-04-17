import { describe, expect, it } from "vitest";
import { formatTriggerPattern } from "./triggerPattern";

describe("formatTriggerPattern", () => {
  it("returns unit-variant strings unchanged", () => {
    expect(formatTriggerPattern("lifecycle")).toBe("lifecycle");
    expect(formatTriggerPattern("all")).toBe("all");
    expect(formatTriggerPattern("memory_update")).toBe("memory_update");
  });

  it("formats struct variants as 'variant: value' — the bug in #2703", () => {
    // Before the fix this object was rendered directly in JSX and React
    // threw "Objects are not valid as a React child", blanking the page.
    expect(
      formatTriggerPattern({ agent_spawned: { name_pattern: "worker" } })
    ).toBe("agent_spawned: worker");
    expect(
      formatTriggerPattern({ system_keyword: { keyword: "error" } })
    ).toBe("system_keyword: error");
    expect(
      formatTriggerPattern({ content_match: { substring: "hello world" } })
    ).toBe("content_match: hello world");
  });

  it("falls back to undefined for missing or unrecognized shapes", () => {
    expect(formatTriggerPattern(undefined)).toBeUndefined();
    expect(formatTriggerPattern(null)).toBeUndefined();
    expect(formatTriggerPattern({})).toBeUndefined();
    expect(formatTriggerPattern(42)).toBeUndefined();
  });

  it("returns just the variant name when payload has no string fields", () => {
    expect(formatTriggerPattern({ weird_variant: {} })).toBe("weird_variant");
    expect(formatTriggerPattern({ weird_variant: { n: 1 } })).toBe("weird_variant");
  });
});
