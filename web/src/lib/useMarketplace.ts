import { useQuery } from '@tanstack/react-query'
import type { RegistryCategory } from '../useRegistry'

const MARKETPLACE_API = 'https://marketplace.librefang.ai/v1/packages'

// Maps registry categories to marketplace `kind` values.
// Categories with no marketplace coverage return null.
const CATEGORY_KIND: Partial<Record<RegistryCategory, string>> = {
  skills: 'skill',
  hands:  'hand',
  mcp:    'mcp',
  plugins: 'extension',
}

export interface MarketplacePkg {
  id: string
  total_downloads: number
  weekly_downloads: number
  stars: number
  latest_version: string | null
  is_verified: boolean
}

async function fetchMarketplace(kind: string): Promise<MarketplacePkg[]> {
  const res = await fetch(`${MARKETPLACE_API}?kind=${encodeURIComponent(kind)}&limit=500`)
  if (!res.ok) throw new Error(`marketplace HTTP ${res.status}`)
  const json = await res.json() as { packages: MarketplacePkg[] }
  return json.packages ?? []
}

export function useMarketplace(category: RegistryCategory): Map<string, MarketplacePkg> {
  const kind = CATEGORY_KIND[category] ?? null
  const { data } = useQuery<MarketplacePkg[]>({
    queryKey: ['marketplace', category],
    queryFn: () => fetchMarketplace(kind!),
    enabled: !!kind,
    staleTime: 1000 * 60 * 15,
    retry: 0,
  })
  const map = new Map<string, MarketplacePkg>()
  for (const pkg of data ?? []) map.set(pkg.id, pkg)
  return map
}
