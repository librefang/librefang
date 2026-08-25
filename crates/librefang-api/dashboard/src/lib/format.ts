/**
 * Format a number with compact units (K / M / B).
 * Hover-friendly: callers can use the raw number as a `title` attribute.
 */
const COMPACT_DECIMAL = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

export function formatCompact(n: number): string {
  if (!Number.isFinite(n)) return "—";
  const abs = Math.abs(n);
  if (abs >= 999_950_000) return `${COMPACT_DECIMAL.format(n / 1_000_000_000)}B`;
  if (abs >= 999_950) return `${COMPACT_DECIMAL.format(n / 1_000_000)}M`;
  if (abs >= 1_000) return `${COMPACT_DECIMAL.format(n / 1_000)}K`;
  return n.toLocaleString();
}

/**
 * Format a USD cost value.
 * Small amounts show 4 decimals, larger amounts show 2.
 */
export function formatCost(usd: number): string {
  if (!Number.isFinite(usd)) return "—";
  const sign = usd < 0 ? "-" : "";
  const abs = Math.abs(usd);
  const body = abs < 0.01 ? abs.toFixed(4) : abs.toFixed(2);
  return `${sign}$${body}`;
}

/**
 * Format byte sizes with appropriate units (B / KB / MB / GB).
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return "—";
  bytes = Math.max(0, bytes);
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}
