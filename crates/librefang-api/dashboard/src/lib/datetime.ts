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
