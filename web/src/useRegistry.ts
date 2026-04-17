import { useQuery } from '@tanstack/react-query'
import { z } from 'zod/v4'

const REGISTRY_API = 'https://stats.librefang.ai/api/registry'
const LOCAL_JSON = '/registry.json'

// ─── Zod schemas ───
const I18nEntrySchema = z.object({
  name: z.string().optional(),
  description: z.string().optional(),
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

// All 8 category arrays are optional on the wire — a stale worker response
// (or the older registry.json shape) may be missing newer ones like skills/mcp.
// The hook normalizes missing arrays to [].
const RegistryDataSchema = z.object({
  hands: z.array(DetailSchema).optional().default([]),
  channels: z.array(DetailSchema).optional().default([]),
  providers: z.array(DetailSchema).optional().default([]),
  integrations: z.array(DetailSchema).optional().default([]),
  workflows: z.array(DetailSchema).optional().default([]),
  agents: z.array(DetailSchema).optional().default([]),
  plugins: z.array(DetailSchema).optional().default([]),
  skills: z.array(DetailSchema).optional().default([]),
  mcp: z.array(DetailSchema).optional().default([]),
  handsCount: z.number().optional().default(0),
  channelsCount: z.number().optional().default(0),
  providersCount: z.number().optional().default(0),
  integrationsCount: z.number().optional().default(0),
  workflowsCount: z.number().optional().default(0),
  agentsCount: z.number().optional().default(0),
  pluginsCount: z.number().optional().default(0),
  skillsCount: z.number().optional().default(0),
  mcpCount: z.number().optional().default(0),
})

export type Detail = z.infer<typeof DetailSchema>
export type HandDetail = Detail
export type ChannelDetail = Detail
export type RegistryData = z.infer<typeof RegistryDataSchema>

export type RegistryCategory =
  | 'hands' | 'channels' | 'providers' | 'integrations'
  | 'workflows' | 'agents' | 'plugins' | 'skills' | 'mcp'

/** Get localized description for a Detail item */
export function getLocalizedDesc(item: Detail, lang: string): string {
  if (lang === 'en') return item.description
  // Try exact match first (zh-TW), then prefix (zh)
  const desc = item.i18n?.[lang]?.description ?? item.i18n?.[lang.split('-')[0]!]?.description
  return desc || item.description
}

/** Get localized name for a Detail item — falls back to English if the
 * target locale has no translated name. Same lookup strategy as the
 * description helper. */
export function getLocalizedName(item: Detail, lang: string): string {
  if (lang === 'en') return item.name
  const name = item.i18n?.[lang]?.name ?? item.i18n?.[lang.split('-')[0]!]?.name
  return name || item.name
}

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

  // 3. Merge: prefer local details (have descriptions), append new items from API
  if (localData && apiData) {
    return {
      hands: mergeDetails(localData.hands, apiData.hands),
      channels: mergeDetails(localData.channels, apiData.channels),
      providers: mergeDetails(localData.providers, apiData.providers),
      integrations: mergeDetails(localData.integrations, apiData.integrations),
      workflows: mergeDetails(localData.workflows, apiData.workflows),
      agents: mergeDetails(localData.agents, apiData.agents),
      plugins: mergeDetails(localData.plugins, apiData.plugins),
      skills: mergeDetails(localData.skills, apiData.skills),
      mcp: mergeDetails(localData.mcp, apiData.mcp),
      // Use API counts (most up to date)
      handsCount: apiData.handsCount || localData.handsCount,
      channelsCount: apiData.channelsCount || localData.channelsCount,
      providersCount: apiData.providersCount || localData.providersCount,
      integrationsCount: apiData.integrationsCount || localData.integrationsCount,
      workflowsCount: apiData.workflowsCount || localData.workflowsCount,
      agentsCount: apiData.agentsCount || localData.agentsCount,
      pluginsCount: apiData.pluginsCount || localData.pluginsCount,
      skillsCount: apiData.skillsCount || localData.skillsCount,
      mcpCount: apiData.mcpCount || localData.mcpCount,
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

/** Return the items array and count for a given category. */
export function getCategoryItems(data: RegistryData | undefined, category: RegistryCategory): { items: Detail[]; count: number } {
  if (!data) return { items: [], count: 0 }
  // Legacy: the `integrations` category used to be a separate bucket
  // but is now the same data as `mcp`. If someone hits /integrations
  // (bookmark / old link), fall through so they see the real content.
  const effective = category === 'integrations' ? 'mcp' : category
  const items = data[effective] ?? []
  const count = (data[`${effective}Count` as keyof RegistryData] as number | undefined) ?? items.length
  return { items, count }
}
