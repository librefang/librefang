// Fetch a raw manifest from the registry. Tries the stats.librefang.ai proxy
// first (commit info, CDN, click-tracking) and falls back to raw.githubusercontent
// when the proxy is unavailable or returns a client/server error. The proxy
// is best-effort: until its /api/registry/raw endpoint is live, every request
// reaches GitHub directly.

const PROXY = 'https://stats.librefang.ai/api/registry/raw'
const GH_RAW = 'https://raw.githubusercontent.com/librefang/librefang-registry/main'

export async function fetchRegistryRaw(path: string): Promise<string> {
  try {
    const res = await fetch(`${PROXY}?path=${encodeURIComponent(path)}`)
    if (res.ok) return res.text()
    // Any non-OK from the proxy (404 while unshipped, 5xx when down): fall
    // through to GitHub raw rather than surfacing a broken manifest.
  } catch {
    // Network error — offline, CORS, DNS failure. Fall through.
  }
  const res = await fetch(`${GH_RAW}/${path}`)
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(`HTTP ${res.status}${body ? `: ${body}` : ''}`)
  }
  return res.text()
}
