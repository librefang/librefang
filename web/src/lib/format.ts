// Compact number formatting for marketplace stats. Mirrors the threshold
// behavior commonly seen on package registries: keep raw digits below 1k,
// switch to k/m above. One decimal place matches GitHub's stargazer pill
// and npm's download counters. Cascades up a unit instead of rendering
// "1000.0k" / "1000.0m" at the upper edge of each band.
export function fmtNum(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '0'
  if (n < 1000) return String(Math.trunc(n))
  if (n < 999_950) return `${(n / 1_000).toFixed(1)}k`
  if (n < 999_950_000) return `${(n / 1_000_000).toFixed(1)}m`
  return `${(n / 1_000_000_000).toFixed(1)}b`
}
