// Cached per-locale Intl.RelativeTimeFormat — instantiation isn't free and
// the same locale is reused on every Auto-Dream row.
const _rtfCache = new Map<string, Intl.RelativeTimeFormat>();

export function getRelativeTimeFormat(locale: string): Intl.RelativeTimeFormat {
  let rtf = _rtfCache.get(locale);
  if (!rtf) {
    rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto", style: "narrow" });
    _rtfCache.set(locale, rtf);
  }
  return rtf;
}

// Format an epoch-ms into a short human-readable "N hours ago" / "in N hours"
// label. Returns the result of `tNever()` when ts is 0 or undefined — the
// status endpoint omits `next_eligible_at_ms` for never-dreamed agents and
// `last_consolidated_at_ms` is 0 in the same case.
export function formatRelativeMs(
  ts: number | undefined,
  now: number,
  locale: string,
  tNever: () => string,
  tJustNow?: () => string,
): string {
  if (ts === undefined || ts === 0) return tNever();
  const diff = ts - now;
  const absSeconds = Math.abs(diff) / 1000;
  // Anything within ~30s reads as "just now" rather than "in 0 minutes" /
  // "this minute" — Intl.RelativeTimeFormat with numeric:"auto" produces
  // locale-dependent and frequently awkward strings at the zero-crossing.
  if (absSeconds < 30) return tJustNow ? tJustNow() : "just now";
  const absMinutes = Math.abs(diff) / 60_000;
  const rtf = getRelativeTimeFormat(locale);
  if (absMinutes < 60) {
    return rtf.format(Math.round(diff / 60_000), "minute");
  }
  const absHours = absMinutes / 60;
  if (absHours < 24) {
    return rtf.format(parseFloat((diff / 3_600_000).toFixed(1)), "hour");
  }
  return rtf.format(parseFloat((diff / 86_400_000).toFixed(1)), "day");
}

// Human-readable duration for effective_min_hours. Switches between minutes,
// hours, days, and weeks so "every 168h" renders as "every 1w" etc.
export function formatHours(
  hours: number,
  unit: { minute: string; hour: string; day: string; week: string },
): string {
  if (hours < 1) return `${(hours * 60).toFixed(0)}${unit.minute}`;
  if (hours < 24) return `${hours % 1 === 0 ? hours.toFixed(0) : hours.toFixed(1)}${unit.hour}`;
  const days = hours / 24;
  if (days < 7) return `${days % 1 === 0 ? days.toFixed(0) : days.toFixed(1)}${unit.day}`;
  const weeks = days / 7;
  return `${weeks % 1 === 0 ? weeks.toFixed(0) : weeks.toFixed(1)}${unit.week}`;
}

// Truncate long KV values for table rendering. Full value remains in the
// title attribute on hover (capped separately via KV_TITLE_TRUNCATE).
export function formatKvValue(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
