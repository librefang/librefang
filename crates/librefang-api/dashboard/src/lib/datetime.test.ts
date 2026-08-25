import { describe, expect, it, vi } from "vitest";
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

  it("selects hour and day units for future dates", () => {
    const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
    expect(formatRelativeTime(2 * 60 * 60 * 1_000, "en", 0)).toBe(rtf.format(2, "hour"));
    expect(formatRelativeTime(2 * 24 * 60 * 60 * 1_000, "en", 0)).toBe(rtf.format(2, "day"));
  });

  it("uses the browser locale when no locale is supplied", () => {
    vi.stubGlobal("navigator", { language: "fr" });
    const expected = new Intl.RelativeTimeFormat("fr", { numeric: "auto" })
      .format(-2, "minute");
    expect(formatRelativeTime(0, undefined, 120_000)).toBe(expected);
    vi.unstubAllGlobals();
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
