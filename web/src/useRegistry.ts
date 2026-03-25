import { useQuery } from '@tanstack/react-query'
import { z } from 'zod/v4'

const REGISTRY_API = 'https://stats.librefang.ai/api/registry'
const LOCAL_JSON = '/registry.json'

// ─── Zod schemas ───
const I18nEntrySchema = z.object({
  description: z.string(),
})

const DetailSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  category: z.string(),
  icon: z.string(),
  tags: z.array(z.string()).optional(),
  i18n: z.record(z.string(), I18nEntrySchema).optional(),
})

const RegistryDataSchema = z.object({
  hands: z.array(DetailSchema),
  channels: z.array(DetailSchema),
  handsCount: z.number(),
  channelsCount: z.number(),
  providersCount: z.number(),
  integrationsCount: z.number(),
  workflowsCount: z.number(),
  agentsCount: z.number(),
  pluginsCount: z.number(),
})

export type Detail = z.infer<typeof DetailSchema>
export type HandDetail = Detail
export type ChannelDetail = Detail

/** Get localized description for a Detail item */
export function getLocalizedDesc(item: Detail, lang: string): string {
  if (lang === 'en') return item.description
  // Try exact match first (zh-TW), then prefix (zh)
  const desc = item.i18n?.[lang]?.description ?? item.i18n?.[lang.split('-')[0]!]?.description
  return desc || item.description
}
export type RegistryData = z.infer<typeof RegistryDataSchema>

async function fetchRegistryData(): Promise<RegistryData> {
  // 1. Load local registry.json (has full descriptions from build time)
  const localRes = await fetch(LOCAL_JSON)
  const local = localRes.ok ? RegistryDataSchema.safeParse(await localRes.json()) : null
  const localData = local?.success ? local.data : null

  // 2. Load API for latest counts (descriptions may be empty)
  let apiData: RegistryData | null = null
  try {
    const apiRes = await fetch(REGISTRY_API)
    if (apiRes.ok) {
      const parsed = RegistryDataSchema.safeParse(await apiRes.json())
      if (parsed.success) apiData = parsed.data
    }
  } catch { /* API unavailable, use local only */ }

  // 3. Merge: use local details + API counts (API has latest numbers)
  if (localData && apiData) {
    return {
      // Use local details (have descriptions), but if API has more items, append them
      hands: mergeDetails(localData.hands, apiData.hands),
      channels: mergeDetails(localData.channels, apiData.channels),
      // Use API counts (most up to date)
      handsCount: apiData.handsCount,
      channelsCount: apiData.channelsCount,
      providersCount: apiData.providersCount,
      integrationsCount: apiData.integrationsCount,
      workflowsCount: apiData.workflowsCount,
      agentsCount: apiData.agentsCount,
      pluginsCount: apiData.pluginsCount,
    }
  }

  if (localData) return localData
  if (apiData) return apiData
  throw new Error('Both local and API registry data unavailable')
}

// Merge: prefer local (has descriptions), add any new items from API
function mergeDetails(local: Detail[], api: Detail[]): Detail[] {
  const localMap = new Map(local.map(d => [d.id, d]))
  for (const item of api) {
    if (!localMap.has(item.id)) {
      localMap.set(item.id, item)
    }
  }
  return Array.from(localMap.values())
}

export function useRegistry() {
  return useQuery<RegistryData>({
    queryKey: ['registry'],
    queryFn: fetchRegistryData,
    staleTime: 1000 * 60 * 60,
    retry: 2,
  })
}
