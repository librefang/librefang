// Fetch a raw manifest from the registry. Tries the stats.librefang.ai proxy
// first (commit info, CDN, click-tracking) and falls back to raw.githubusercontent
// when the proxy is unavailable or returns a client/server error. The proxy
// is best-effort: until its /api/registry/raw endpoint is live, every request
// reaches GitHub directly.

import type { RegistryCategory } from '../useRegistry'

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

// File-path candidates inside librefang-registry for a given (category, id).
// First element is the preferred layout; callers fall through to later ones
// on 404. MCP in particular supports both `mcp/<id>.toml` (flat) and
// `mcp/<id>/MCP.toml` (dir-backed) — matches `web/scripts/fetch-registry.ts`.
export function pathCandidatesFor(category: RegistryCategory, id: string): string[] {
  switch (category) {
    case 'hands':   return [`hands/${id}/HAND.toml`]
    case 'agents':  return [`agents/${id}/agent.toml`]
    case 'plugins': return [`plugins/${id}/plugin.toml`]
    case 'skills':  return [`skills/${id}/SKILL.md`]
    case 'mcp':     return [`mcp/${id}.toml`, `mcp/${id}/MCP.toml`]
    default:        return [`${category}/${id}.toml`]
  }
}

// Fetch the first candidate that exists. Returns the resolved content plus
// the path that actually succeeded so downstream UI (commit history,
// "View on GitHub" link) can point at the real file.
export async function fetchFirstAvailable(
  candidates: string[],
): Promise<{ content: string; path: string }> {
  let lastErr: unknown
  for (const p of candidates) {
    try {
      return { content: await fetchRegistryRaw(p), path: p }
    } catch (e) {
      lastErr = e
    }
  }
  throw lastErr ?? new Error('No registry path candidates succeeded')
}
