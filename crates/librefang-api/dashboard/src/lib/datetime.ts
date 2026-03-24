/**
 * Format a date/time string or Date object as locale date+time.
 * e.g. "3/24/2026, 10:30:00 AM"
 */
export function formatDateTime(value: string | number | Date | undefined | null): string {
  if (!value) return "-";
  return new Date(value).toLocaleString();
}

/**
 * Format as locale date only.
 * e.g. "3/24/2026"
 */
export function formatDate(value: string | number | Date | undefined | null): string {
  if (!value) return "-";
  return new Date(value).toLocaleDateString();
}

/**
 * Format as locale time only.
 * e.g. "10:30:00 AM"
 */
export function formatTime(value: string | number | Date | undefined | null): string {
  if (!value) return "-";
  return new Date(value).toLocaleTimeString();
}

/**
 * Format a timestamp as relative time ("just now", "3m ago", "2h ago", "5d ago").
 */
export function formatRelativeTime(value: string | number | Date | undefined | null): string {
  if (!value) return "-";
  const diff = Date.now() - new Date(value).getTime();
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

/**
 * Format uptime duration in seconds as human-readable string.
 * e.g. 90 → "1m", 3700 → "1h 1m", 90000 → "1d 1h"
 */
export function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
}
