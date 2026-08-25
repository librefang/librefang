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
  tJustNow: () => string,
): string {
  if (ts === undefined || ts === 0) return tNever();
  const diff = ts - now;
  const absSeconds = Math.abs(diff) / 1000;
  // Anything within ~30s reads as "just now" rather than "in 0 minutes" /
  // "this minute" — Intl.RelativeTimeFormat with numeric:"auto" produces
  // locale-dependent and frequently awkward strings at the zero-crossing.
  if (absSeconds < 30) return tJustNow();
  const rtf = getRelativeTimeFormat(locale);
  const direction = Math.sign(diff);
  const roundedMinutes = Math.round(absSeconds / 60);
  if (roundedMinutes < 60) {
    return rtf.format(direction * roundedMinutes, "minute");
  }
  const roundedHours = Math.round((absSeconds / 3_600) * 10) / 10;
  if (roundedHours < 24) {
    return rtf.format(direction * roundedHours, "hour");
  }
  const roundedDays = Math.round((absSeconds / 86_400) * 10) / 10;
  return rtf.format(direction * roundedDays, "day");
}

function formatUnitValue(value: number): string {
  const nearestInteger = Math.round(value);
  return Math.abs(value - nearestInteger) < 1e-9
    ? nearestInteger.toFixed(0)
    : value.toFixed(1);
}

// Human-readable duration for effective_min_hours. Switches between minutes,
// hours, days, and weeks so "every 168h" renders as "every 1w" etc.
export function formatHours(
  hours: number,
  unit: { minute: string; hour: string; day: string; week: string },
): string {
  if (hours < 1) return `${(hours * 60).toFixed(0)}${unit.minute}`;
  if (hours < 24) return `${formatUnitValue(hours)}${unit.hour}`;
  const days = hours / 24;
  if (days < 7) return `${formatUnitValue(days)}${unit.day}`;
  const weeks = days / 7;
  return `${formatUnitValue(weeks)}${unit.week}`;
}

// Serialize KV values to display strings. AgentKvRows applies its separate
// table-cell and title-preview truncation limits after calling this helper.
export function formatKvValue(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
