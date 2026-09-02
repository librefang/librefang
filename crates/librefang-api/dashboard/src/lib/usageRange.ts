/**
 * Date-range presets for the Analytics page (#8062).
 *
 * The `/api/usage/*` endpoints have accepted `start_date` / `end_date` since #7891, but the page only ever asked for a fixed 7-day window, so an operator who needed "what did we spend last month" had to query the API by hand.
 *
 * # Everything here is UTC, on purpose
 *
 * The server parses both bounds as **UTC calendar days** (`librefang_memory::usage::DateRange`), and `usage_events.timestamp` is written as `Utc::now()`.
 * Resolving "today" against the viewer's local midnight would therefore hand the server a window shifted by the local offset — up to a full day of spend attributed to the wrong date, which is the quiet kind of wrong a cost report never recovers from.
 * So every helper below works in UTC and the picker labels itself as UTC rather than pretending otherwise.
 */

/** The preset buttons, in the order they render. */
export const USAGE_RANGE_PRESETS = [
  "today",
  "yesterday",
  "7d",
  "14d",
  "30d",
  "this_month",
  "last_month",
  "all",
] as const;

export type UsageRangePreset = (typeof USAGE_RANGE_PRESETS)[number];

/**
 * A resolved window, in the exact shape the query string takes.
 *
 * An absent bound means "unbounded on that side", matching the server's
 * treatment of a missing or empty parameter.
 */
export type UsageRange = {
  start_date?: string;
  end_date?: string;
};

/**
 * Upper bound the daily endpoint enforces on its rolling `days` parameter
 * (`MAX_DAILY_DAYS` in `routes/budget.rs`). Asking for more is a 400.
 */
export const USAGE_DAILY_MAX_DAYS = 366;

/** Format an instant as the `YYYY-MM-DD` UTC calendar day the API expects. */
export function toUtcDay(date: Date): string {
  return date.toISOString().slice(0, 10);
}

/** `YYYY-MM-DD` is a valid calendar date (not just a well-shaped string). */
export function isUtcDay(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  // `2026-02-31` matches the regex but is not a day; round-tripping through
  // Date catches it. `2026-13-01` produces an *invalid* Date whose
  // `toISOString()` throws rather than returning a mismatched string, so the
  // timestamp is checked for NaN before it is formatted.
  const ms = Date.parse(`${value}T00:00:00Z`);
  if (Number.isNaN(ms)) return false;
  return toUtcDay(new Date(ms)) === value;
}

function utcDayOffset(now: Date, days: number): Date {
  return new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + days),
  );
}

/**
 * Turn a preset into concrete bounds.
 *
 * Windows are inclusive on both ends and never extend past today: a report is
 * being read now, so a range that runs into the future only makes the caption
 * lie about what it covers. `7d` therefore means "today and the six days
 * before it", which is 7 calendar days, and `this_month` is month-to-date.
 *
 * `all` resolves to `{}` — no bounds at all, rather than a very wide range, so
 * the server takes its genuinely unfiltered path.
 */
export function resolveUsageRange(
  preset: UsageRangePreset,
  now: Date = new Date(),
): UsageRange {
  const today = toUtcDay(now);
  switch (preset) {
    case "today":
      return { start_date: today, end_date: today };
    case "yesterday": {
      const y = toUtcDay(utcDayOffset(now, -1));
      return { start_date: y, end_date: y };
    }
    case "7d":
      return { start_date: toUtcDay(utcDayOffset(now, -6)), end_date: today };
    case "14d":
      return { start_date: toUtcDay(utcDayOffset(now, -13)), end_date: today };
    case "30d":
      return { start_date: toUtcDay(utcDayOffset(now, -29)), end_date: today };
    case "this_month":
      return {
        start_date: toUtcDay(
          new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1)),
        ),
        end_date: today,
      };
    case "last_month": {
      const firstOfLast = new Date(
        Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - 1, 1),
      );
      // Day 0 of this month is the last day of the previous one, which is how
      // this avoids a 28/29/30/31 table and the leap-year special case.
      const lastOfLast = new Date(
        Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 0),
      );
      return {
        start_date: toUtcDay(firstOfLast),
        end_date: toUtcDay(lastOfLast),
      };
    }
    case "all":
      return {};
  }
}

/**
 * Only send parameters the server can act on.
 *
 * A blank or half-typed custom bound is dropped rather than forwarded: the
 * endpoints answer `400` for a malformed date, and a picker mid-edit should not
 * turn the whole page red.
 */
export function normalizeUsageRange(range: UsageRange): UsageRange {
  const out: UsageRange = {};
  if (range.start_date && isUtcDay(range.start_date)) {
    out.start_date = range.start_date;
  }
  if (range.end_date && isUtcDay(range.end_date)) {
    out.end_date = range.end_date;
  }
  // An inverted range is a 400 from every endpoint, so it never reaches one:
  // the picker keeps rendering the raw bounds and says they are inverted, while
  // the queries fall back to the unbounded window. Not to the last valid one —
  // this function is pure and holds no history — which is why the picker's
  // message says "showing the unfiltered window" rather than implying the
  // previous numbers are still on screen.
  if (out.start_date && out.end_date && out.end_date < out.start_date) {
    return {};
  }
  return out;
}

/** Whether a range is inverted, so the picker can say so instead of erroring. */
export function isInvertedRange(range: UsageRange): boolean {
  return Boolean(
    range.start_date &&
      range.end_date &&
      isUtcDay(range.start_date) &&
      isUtcDay(range.end_date) &&
      range.end_date < range.start_date,
  );
}

/**
 * How many days the daily endpoint should report on.
 *
 * `/api/usage/daily` rejects `days` combined with an explicit range, so a
 * bounded window sends dates and no `days`, and only the unbounded `all` case
 * needs a number. It gets the endpoint's maximum: the response is one row per
 * day, so 366 is both the cap and the widest chart the page can draw.
 */
export function dailyDaysFor(range: UsageRange): number | undefined {
  const bounded = Boolean(range.start_date || range.end_date);
  return bounded ? undefined : USAGE_DAILY_MAX_DAYS;
}

/** Inclusive day count of a fully-bounded range; `null` when either end is open. */
export function rangeSpanDays(range: UsageRange): number | null {
  if (!range.start_date || !range.end_date) return null;
  const start = Date.parse(`${range.start_date}T00:00:00Z`);
  const end = Date.parse(`${range.end_date}T00:00:00Z`);
  if (Number.isNaN(start) || Number.isNaN(end)) return null;
  return Math.floor((end - start) / 86_400_000) + 1;
}
