import { describe, expect, it } from "vitest";
import { formatDate, formatDateTime, formatRelativeTime, formatTime, formatUptime } from "./datetime";

describe("date formatting", () => {
  it("accepts the Unix epoch instead of treating zero as missing", () => {
    expect(formatDateTime(0)).not.toBe("-");
    expect(formatDate(0)).not.toBe("-");
    expect(formatTime(0)).not.toBe("-");
    expect(formatRelativeTime(0, "en", 1_000)).not.toBe("-");
  });

  it("returns a stable placeholder for missing and invalid dates", () => {
    for (const value of [undefined, null, "", "not-a-date"] as const) {
      expect(formatDateTime(value)).toBe("-");
      expect(formatDate(value)).toBe("-");
      expect(formatTime(value)).toBe("-");
      expect(formatRelativeTime(value, "en", 1_000)).toBe("-");
    }
  });

  it("rejects a non-finite relative-time clock", () => {
    expect(formatRelativeTime(0, "en", Number.NaN)).toBe("-");
    expect(formatRelativeTime(0, "en", Number.POSITIVE_INFINITY)).toBe("-");
  });
});

describe("formatUptime", () => {
  it("rejects negative and non-finite durations", () => {
    expect(formatUptime(-1)).toBe("-");
    expect(formatUptime(Number.NaN)).toBe("-");
    expect(formatUptime(Number.POSITIVE_INFINITY)).toBe("-");
  });

  it("floors fractional seconds consistently", () => {
    expect(formatUptime(30.9)).toBe("30s");
  });
});
