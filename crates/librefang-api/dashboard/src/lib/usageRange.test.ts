import { describe, expect, it } from "vitest";
import {
  USAGE_DAILY_MAX_DAYS,
  USAGE_RANGE_PRESETS,
  dailyDaysFor,
  isInvertedRange,
  isUtcDay,
  normalizeUsageRange,
  rangeSpanDays,
  resolveUsageRange,
  toUtcDay,
} from "./usageRange";

// A fixed instant so the preset arithmetic is asserted against known dates
// rather than against "whatever today is". Chosen mid-month, mid-year, and
// deliberately late in the UTC day: 23:30Z is the window where a helper that
// reached for local time instead of UTC would silently be a day off for any
// viewer east of Greenwich.
const NOW = new Date("2026-03-15T23:30:00Z");

describe("toUtcDay", () => {
  it("formats an instant as its UTC calendar day", () => {
    expect(toUtcDay(NOW)).toBe("2026-03-15");
  });

  it("uses the UTC day, not the local one", () => {
    // 00:30 on the 16th UTC is still the 15th in every negative offset. The
    // answer must follow UTC, because that is what the server filters on.
    expect(toUtcDay(new Date("2026-03-16T00:30:00Z"))).toBe("2026-03-16");
  });
});

describe("isUtcDay", () => {
  it("accepts a well-formed calendar day", () => {
    expect(isUtcDay("2026-03-01")).toBe(true);
    expect(isUtcDay("2024-02-29")).toBe(true);
  });

  it("rejects malformed and impossible dates", () => {
    expect(isUtcDay("")).toBe(false);
    expect(isUtcDay("2026-3-1")).toBe(false);
    expect(isUtcDay("2026-03-01T00:00:00Z")).toBe(false);
    // Shape-valid but not a real day — the regex alone would let this through
    // and the server would answer 400.
    expect(isUtcDay("2026-02-31")).toBe(false);
    expect(isUtcDay("2026-13-01")).toBe(false);
  });
});

describe("resolveUsageRange", () => {
  it("resolves single-day presets to the same start and end", () => {
    expect(resolveUsageRange("today", NOW)).toEqual({
      start_date: "2026-03-15",
      end_date: "2026-03-15",
    });
    expect(resolveUsageRange("yesterday", NOW)).toEqual({
      start_date: "2026-03-14",
      end_date: "2026-03-14",
    });
  });

  it("makes the N-day presets N calendar days inclusive of today", () => {
    // 7 days means today plus the six before it, so the span is 7 — not 8.
    expect(resolveUsageRange("7d", NOW)).toEqual({
      start_date: "2026-03-09",
      end_date: "2026-03-15",
    });
    expect(rangeSpanDays(resolveUsageRange("7d", NOW))).toBe(7);
    expect(rangeSpanDays(resolveUsageRange("14d", NOW))).toBe(14);
    expect(rangeSpanDays(resolveUsageRange("30d", NOW))).toBe(30);
    expect(resolveUsageRange("30d", NOW).start_date).toBe("2026-02-14");
  });

  it("resolves this_month to month-to-date, never into the future", () => {
    expect(resolveUsageRange("this_month", NOW)).toEqual({
      start_date: "2026-03-01",
      end_date: "2026-03-15",
    });
  });

  it("resolves last_month to the whole previous calendar month", () => {
    expect(resolveUsageRange("last_month", NOW)).toEqual({
      start_date: "2026-02-01",
      end_date: "2026-02-28",
    });
  });

  it("gets February's length right in a leap year", () => {
    // 2024 is a leap year, so last_month from March must end on the 29th. This
    // is why the implementation uses "day 0 of this month" instead of a table.
    expect(resolveUsageRange("last_month", new Date("2024-03-10T12:00:00Z"))).toEqual({
      start_date: "2024-02-01",
      end_date: "2024-02-29",
    });
  });

  it("rolls back across a year boundary for last_month in January", () => {
    expect(resolveUsageRange("last_month", new Date("2026-01-07T12:00:00Z"))).toEqual({
      start_date: "2025-12-01",
      end_date: "2025-12-31",
    });
  });

  it("crosses a month boundary for a 30-day window from early in a month", () => {
    expect(resolveUsageRange("30d", new Date("2026-01-05T12:00:00Z"))).toEqual({
      start_date: "2025-12-07",
      end_date: "2026-01-05",
    });
  });

  it("resolves all to no bounds at all", () => {
    // Not a very wide range — genuinely unbounded, so the server takes its
    // unfiltered path and the request matches the pre-#8062 one.
    expect(resolveUsageRange("all", NOW)).toEqual({});
  });

  it("resolves every declared preset to a valid window", () => {
    for (const preset of USAGE_RANGE_PRESETS) {
      const range = resolveUsageRange(preset, NOW);
      for (const bound of [range.start_date, range.end_date]) {
        if (bound !== undefined) expect(isUtcDay(bound)).toBe(true);
      }
      expect(isInvertedRange(range)).toBe(false);
      // Nothing may reach past today, or the caption describes a window that
      // cannot contain data.
      if (range.end_date) expect(range.end_date <= toUtcDay(NOW)).toBe(true);
    }
  });
});

describe("normalizeUsageRange", () => {
  it("passes through a valid range", () => {
    const range = { start_date: "2026-03-01", end_date: "2026-03-31" };
    expect(normalizeUsageRange(range)).toEqual(range);
  });

  it("drops blank and half-typed bounds instead of forwarding a 400", () => {
    expect(normalizeUsageRange({ start_date: "", end_date: "" })).toEqual({});
    expect(normalizeUsageRange({ start_date: "2026-0" })).toEqual({});
    expect(
      normalizeUsageRange({ start_date: "2026-03-01", end_date: "2026-0" }),
    ).toEqual({ start_date: "2026-03-01" });
  });

  it("falls back to the unbounded window for an inverted range", () => {
    expect(
      normalizeUsageRange({ start_date: "2026-03-31", end_date: "2026-03-01" }),
    ).toEqual({});
  });

  it("keeps a single-day range, which is not inverted", () => {
    const sameDay = { start_date: "2026-03-15", end_date: "2026-03-15" };
    expect(normalizeUsageRange(sameDay)).toEqual(sameDay);
    expect(isInvertedRange(sameDay)).toBe(false);
  });
});

describe("isInvertedRange", () => {
  it("only reports true for two valid bounds in the wrong order", () => {
    expect(
      isInvertedRange({ start_date: "2026-03-31", end_date: "2026-03-01" }),
    ).toBe(true);
    expect(
      isInvertedRange({ start_date: "2026-03-01", end_date: "2026-03-31" }),
    ).toBe(false);
    // A half-typed bound is not yet an inversion — the picker should not shout
    // at the operator mid-keystroke.
    expect(isInvertedRange({ start_date: "2026-03-31", end_date: "2026-0" })).toBe(
      false,
    );
    expect(isInvertedRange({ start_date: "2026-03-31" })).toBe(false);
  });
});

describe("dailyDaysFor", () => {
  it("returns no days for a bounded window", () => {
    // The endpoint answers 400 when `days` arrives with a range.
    expect(dailyDaysFor({ start_date: "2026-03-01", end_date: "2026-03-31" })).toBeUndefined();
    expect(dailyDaysFor({ start_date: "2026-03-01" })).toBeUndefined();
    expect(dailyDaysFor({ end_date: "2026-03-31" })).toBeUndefined();
  });

  it("returns the endpoint maximum for the unbounded window", () => {
    expect(dailyDaysFor({})).toBe(USAGE_DAILY_MAX_DAYS);
    expect(USAGE_DAILY_MAX_DAYS).toBe(366);
  });
});

describe("rangeSpanDays", () => {
  it("counts both endpoints", () => {
    expect(rangeSpanDays({ start_date: "2026-03-01", end_date: "2026-03-01" })).toBe(1);
    expect(rangeSpanDays({ start_date: "2026-03-01", end_date: "2026-03-31" })).toBe(31);
  });

  it("returns null when either end is open", () => {
    expect(rangeSpanDays({})).toBeNull();
    expect(rangeSpanDays({ start_date: "2026-03-01" })).toBeNull();
    expect(rangeSpanDays({ end_date: "2026-03-31" })).toBeNull();
  });

  it("is not thrown off by a DST transition", () => {
    // Both bounds are parsed as UTC midnight, so a range spanning the US and
    // EU DST changes is still an exact whole number of days. A local-time
    // implementation would land on 30.958… and floor to 30.
    expect(rangeSpanDays({ start_date: "2026-03-01", end_date: "2026-03-31" })).toBe(31);
    expect(rangeSpanDays({ start_date: "2026-10-01", end_date: "2026-11-01" })).toBe(32);
  });
});
