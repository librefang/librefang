import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { RegistryCategory } from '../useRegistry'

const MARKETPLACE_API = import.meta.env.VITE_MARKETPLACE_API_URL ?? '/v1/packages'
const PAGE_SIZE = 100

// Maps registry categories to marketplace `kind` values.
// Categories with no marketplace coverage return null.
const CATEGORY_KIND: Partial<Record<RegistryCategory, string>> = {
  skills:  'skill',
  hands:   'hand',
  mcp:     'mcp',
  plugins: 'extension',
}

export interface MarketplacePkg {
  id: string
  total_downloads: number
  weekly_downloads: number
  stars: number
  latest_version: string | null
}

export async function fetchMarketplace(kind: string): Promise<MarketplacePkg[]> {
  const packages: MarketplacePkg[] = []
  let total: number | null = null

  do {
    const params = new URLSearchParams({
      kind,
      limit: String(PAGE_SIZE),
      offset: String(packages.length),
    })
    const res = await fetch(`${MARKETPLACE_API}?${params}`)
    if (!res.ok) throw new Error(`marketplace HTTP ${res.status}`)

    const json = await res.json() as { packages?: unknown; total?: unknown }
    if (!Array.isArray(json.packages) || !Number.isSafeInteger(json.total) || (json.total as number) < 0) {
      throw new Error('marketplace returned an invalid package page')
    }
    total ??= json.total as number
    if (json.packages.length === 0 && packages.length < total) {
      throw new Error('marketplace returned an incomplete package list')
    }
    packages.push(...json.packages as MarketplacePkg[])
  } while (packages.length < total)

  return packages.slice(0, total)
}

export function marketplaceQueryOptions(category: RegistryCategory) {
  const kind = CATEGORY_KIND[category] ?? null
  return {
    queryKey: ['marketplace', kind],
    queryFn: () => kind ? fetchMarketplace(kind) : Promise.resolve([]),
    enabled: !!kind,
    staleTime: 1000 * 60 * 15,
    retry: 0,
  }
}

export function useMarketplace(category: RegistryCategory): Map<string, MarketplacePkg> {
  const { data } = useQuery<MarketplacePkg[]>(marketplaceQueryOptions(category))
  return useMemo(() => {
    const map = new Map<string, MarketplacePkg>()
    for (const pkg of data ?? []) map.set(pkg.id, pkg)
    return map
  }, [data])
}
