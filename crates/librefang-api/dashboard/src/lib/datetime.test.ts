import { describe, expect, it, vi } from "vitest";
import { formatDate, formatDateTime, formatRelativeTime, formatSqliteDateTime, formatTime, formatUptime } from "./datetime";

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

describe("formatSqliteDateTime", () => {
  // The exact shape `manifest_versions.timestamp` carries: the column defaults
  // to SQLite's `datetime('now')` and the insert never supplies the value, so
  // the API hands the dashboard a space-separated UTC string with no offset.
  const SQLITE_NOW = "2026-09-05 13:00:00";

  it("reads a space-separated SQLite timestamp as UTC, not as local time", () => {
    // The bug this pins: `new Date("2026-09-05 13:00:00")` is local time, so an
    // operator east of UTC saw the change stamped hours off. Compare against the
    // instant the string actually denotes rather than a formatted literal, which
    // would only restate whatever locale the test host happens to run in.
    const expected = new Date(Date.UTC(2026, 8, 5, 13, 0, 0));
    expect(formatSqliteDateTime(SQLITE_NOW)).toBe(expected.toLocaleString());
  });

  it("does not append a second zone to a value that already carries one", () => {
    // If the column is ever migrated to a chrono RFC 3339 serialisation, the
    // string arrives with its own `Z`. Blindly appending another one yields
    // `Invalid Date`, so the offset-bearing form has to pass through untouched.
    const rfc3339 = "2026-09-05T13:00:00Z";
    const expected = new Date(Date.UTC(2026, 8, 5, 13, 0, 0));
    expect(formatSqliteDateTime(rfc3339)).toBe(expected.toLocaleString());
    expect(formatSqliteDateTime("2026-09-05T15:00:00+02:00")).toBe(expected.toLocaleString());
  });

  it("falls back to the raw string rather than rendering 'Invalid Date'", () => {
    // `new Date(...)` does not throw on a malformed value, so the previous
    // try/catch never fired and the tab rendered the literal text
    // "Invalid Date". Showing the stored value is the honest failure.
    expect(formatSqliteDateTime("not-a-timestamp")).toBe("not-a-timestamp");
  });

  it("returns the placeholder for a missing timestamp", () => {
    expect(formatSqliteDateTime(undefined)).toBe("—");
    expect(formatSqliteDateTime("")).toBe("—");
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
