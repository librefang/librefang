/**
 * The locale every number on the dashboard is formatted in.
 *
 * Pinned rather than left to resolve from the environment. `toLocaleString()`
 * with no argument, and `Intl.NumberFormat(undefined, …)`, follow the host's
 * `LC_ALL` / `LANG` under Node and the browser's language on a page — so the
 * same figure renders as `2,000` for one operator and `2000` for another
 * (Spanish does not group four-digit numbers) against the same daemon.
 *
 * It also made `AnalyticsPage.test.tsx` fail on `main` itself wherever the
 * environment locale was not English, because the component and the assertion
 * disagreed about which locale was in force (#8156).
 *
 * Dates deliberately keep the ambient locale — see `lib/datetime.ts`. A
 * timestamp is read as "when, for me"; a token count is read against the row
 * above it.
 */
export const NUMBER_LOCALE = "en-US";

/**
 * Format an integer with thousands separators: `2000` → `2,000`.
 *
 * Use in place of calling `toLocaleString` on the value directly. A nullish
 * value renders as `0`, which is what the call sites' inline `?? 0` was
 * already doing.
 */
const GROUPED = new Intl.NumberFormat(NUMBER_LOCALE);

export function formatNumber(value: number | null | undefined): string {
  return GROUPED.format(value ?? 0);
}

/**
 * Format a number with compact units (K / M / B).
 * Hover-friendly: callers can use the raw number as a `title` attribute.
 */
const COMPACT_DECIMAL = new Intl.NumberFormat(NUMBER_LOCALE, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

export function formatCompact(n: number): string {
  if (!Number.isFinite(n)) return "—";
  const abs = Math.abs(n);
  if (abs >= 999_950_000) return `${COMPACT_DECIMAL.format(n / 1_000_000_000)}B`;
  if (abs >= 999_950) return `${COMPACT_DECIMAL.format(n / 1_000_000)}M`;
  if (abs >= 1_000) return `${COMPACT_DECIMAL.format(n / 1_000)}K`;
  return formatNumber(n);
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
