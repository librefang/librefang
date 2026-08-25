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

const CountSchema = z.number().nullable().transform(value => value ?? undefined).optional()

// All 8 category arrays are optional on the wire — a stale worker response
// (or the older registry.json shape) may be missing newer ones like skills/mcp.
// The hook normalizes missing arrays to [].
const RegistryDataSchema = z.object({
  hands: z.array(DetailSchema).optional().default([]),
  channels: z.array(DetailSchema).optional().default([]),
  providers: z.array(DetailSchema).optional().default([]),
  workflows: z.array(DetailSchema).optional().default([]),
  agents: z.array(DetailSchema).optional().default([]),
  plugins: z.array(DetailSchema).optional().default([]),
  skills: z.array(DetailSchema).optional().default([]),
  mcp: z.array(DetailSchema).optional().default([]),
  // Keep the count fields nullish (no `.default(0)`) so missing or explicit
  // unknown values in a partial API response do not invalidate its items.
  // The merge step below
  // uses `apiData.*Count ?? localData.*Count` to prefer local counts
  // when the API omits them — defaulting here to 0 would shadow that
  // fallback and clobber valid local counts with zeros.
  handsCount: CountSchema,
  channelsCount: CountSchema,
  providersCount: CountSchema,
  workflowsCount: CountSchema,
  agentsCount: CountSchema,
  pluginsCount: CountSchema,
  skillsCount: CountSchema,
  mcpCount: CountSchema,
})

export type Detail = z.infer<typeof DetailSchema>
export type HandDetail = Detail
export type ChannelDetail = Detail
export type RegistryData = z.infer<typeof RegistryDataSchema>

export type RegistryCategory =
  | 'hands' | 'channels' | 'providers'
  | 'workflows' | 'agents' | 'plugins' | 'skills' | 'mcp'

const COUNT_FIELDS = {
  hands: 'handsCount',
  channels: 'channelsCount',
  providers: 'providersCount',
  workflows: 'workflowsCount',
  agents: 'agentsCount',
  plugins: 'pluginsCount',
  skills: 'skillsCount',
  mcp: 'mcpCount',
} as const satisfies Record<RegistryCategory, `${RegistryCategory}Count`>

function languagePrefix(lang: string): string | undefined {
  return lang.split('-')[0] || undefined
}

/** Get localized description for a Detail item */
export function getLocalizedDesc(item: Detail, lang: string): string {
  if (lang === 'en') return item.description
  // Try exact match first (zh-TW), then prefix (zh)
  const prefix = languagePrefix(lang)
  const description = item.i18n?.[lang]?.description
    ?? (prefix ? item.i18n?.[prefix]?.description : undefined)
  return description || item.description
}

/** Get localized name for a Detail item — falls back to English if the
 * target locale has no translated name. Same lookup strategy as the
 * description helper. */
export function getLocalizedName(item: Detail, lang: string): string {
  if (lang === 'en') return item.name
  const prefix = languagePrefix(lang)
  const name = item.i18n?.[lang]?.name
    ?? (prefix ? item.i18n?.[prefix]?.name : undefined)
  return name || item.name
}

async function fetchRegistrySource(url: string, fetcher: typeof fetch): Promise<RegistryData | null> {
  try {
    const response = await fetcher(url)
    if (!response.ok) return null
    const parsed = RegistryDataSchema.safeParse(await response.json())
    return parsed.success ? parsed.data : null
  } catch {
    return null
  }
}

export async function fetchRegistryData(fetcher: typeof fetch = fetch): Promise<RegistryData> {
  const [localData, apiData] = await Promise.all([
    fetchRegistrySource(LOCAL_JSON, fetcher),
    fetchRegistrySource(REGISTRY_API, fetcher),
  ])

  // Prefer local details (have descriptions), append new items from API.
  if (localData && apiData) {
    return {
      hands: mergeDetails(localData.hands, apiData.hands),
      channels: mergeDetails(localData.channels, apiData.channels),
      providers: mergeDetails(localData.providers, apiData.providers),
      workflows: mergeDetails(localData.workflows, apiData.workflows),
      agents: mergeDetails(localData.agents, apiData.agents),
      plugins: mergeDetails(localData.plugins, apiData.plugins),
      skills: mergeDetails(localData.skills, apiData.skills),
      mcp: mergeDetails(localData.mcp, apiData.mcp),
      // Use API counts (most up to date). `??` preserves real zero
      // values (category legitimately empty) instead of silently
      // falling through to stale local counts.
      handsCount: apiData.handsCount ?? localData.handsCount,
      channelsCount: apiData.channelsCount ?? localData.channelsCount,
      providersCount: apiData.providersCount ?? localData.providersCount,
      workflowsCount: apiData.workflowsCount ?? localData.workflowsCount,
      agentsCount: apiData.agentsCount ?? localData.agentsCount,
      pluginsCount: apiData.pluginsCount ?? localData.pluginsCount,
      skillsCount: apiData.skillsCount ?? localData.skillsCount,
      mcpCount: apiData.mcpCount ?? localData.mcpCount,
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
    queryFn: () => fetchRegistryData(),
    staleTime: 1000 * 60 * 60,
    retry: 2,
  })
}

/** Return the items array and count for a given category. */
export function getCategoryItems(data: RegistryData | undefined, category: RegistryCategory): { items: Detail[]; count: number } {
  if (!data) return { items: [], count: 0 }
  const items = data[category] ?? []
  const count = data[COUNT_FIELDS[category]] ?? items.length
  return { items, count }
}
