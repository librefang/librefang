/**
 * Format a date/time string or Date object as locale date+time.
 * e.g. "3/24/2026, 10:30:00 AM"
 */
export function formatDateTime(value: string | number | Date | undefined | null): string {
  const date = validDate(value);
  return date ? date.toLocaleString() : "-";
}

/**
 * Format as locale date only.
 * e.g. "3/24/2026"
 */
export function formatDate(value: string | number | Date | undefined | null): string {
  const date = validDate(value);
  return date ? date.toLocaleDateString() : "-";
}

/**
 * Format as locale time only.
 * e.g. "10:30:00 AM"
 */
export function formatTime(value: string | number | Date | undefined | null): string {
  const date = validDate(value);
  return date ? date.toLocaleTimeString() : "-";
}

/**
 * Format a timestamp that arrives in SQLite's `datetime('now')` shape.
 *
 * A column defaulted to `datetime('now')` stores `YYYY-MM-DD HH:MM:SS`: UTC,
 * space-separated, carrying no offset.
 * That is not a form the spec requires `Date` to accept, and parsing it as-is
 * would read a UTC instant as local time, so the space becomes a `T` and a `Z`
 * is appended — but only when the value does not already declare a zone, since
 * appending a second one to an RFC 3339 string yields `Invalid Date`.
 *
 * An unparseable value is returned verbatim. `new Date` does not throw on bad
 * input, so a `try`/`catch` around it never fires and the caller would render
 * the literal text "Invalid Date"; showing the stored value is the honest
 * failure.
 */
export function formatSqliteDateTime(value: string | undefined | null): string {
  if (!value) return "—";
  const iso = value.includes(" ") ? value.replace(" ", "T") : value;
  const zoned = /(?:Z|[+-]\d{2}:?\d{2})$/i.test(iso) ? iso : `${iso}Z`;
  const date = new Date(zoned);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function validDate(value: string | number | Date | undefined | null): Date | undefined {
  if (value === null || value === undefined || value === "") return undefined;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

/**
 * Format a timestamp as relative time ("just now", "3m ago", "2h ago", "5d ago").
 */
const rtfCache = new Map<string, Intl.RelativeTimeFormat>();

function getRtf(locale: string): Intl.RelativeTimeFormat {
  let rtf = rtfCache.get(locale);
  if (!rtf) {
    rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
    rtfCache.set(locale, rtf);
  }
  return rtf;
}

export function formatRelativeTime(value: string | number | Date | undefined | null, locale?: string, nowMs?: number): string {
  const date = validDate(value);
  if (!date) return "-";
  const now = nowMs ?? Date.now();
  if (!Number.isFinite(now)) return "-";
  const diff = now - date.getTime();
  const direction = diff >= 0 ? -1 : 1;
  const seconds = Math.floor(Math.abs(diff) / 1000);
  const defaultLocale = typeof navigator !== "undefined" && navigator.language
    ? navigator.language
    : "en";
  const rtf = getRtf(locale ?? defaultLocale);
  if (seconds < 60) return rtf.format(direction * seconds, "second");
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return rtf.format(direction * minutes, "minute");
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return rtf.format(direction * hours, "hour");
  const days = Math.floor(hours / 24);
  return rtf.format(direction * days, "day");
}

/**
 * Format uptime duration in seconds as human-readable string.
 * e.g. 90 → "1m", 3700 → "1h 1m", 90000 → "1d 1h"
 */
export function formatUptime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "-";
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
}
