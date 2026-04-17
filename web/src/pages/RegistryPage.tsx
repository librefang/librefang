import { useEffect, useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowRight, Search, Loader2, AlertCircle, Sparkles, RotateCcw, Github, ExternalLink, ArrowUpDown, Star } from 'lucide-react'
import { useRegistry, getLocalizedDesc, getLocalizedName, getCategoryItems } from '../useRegistry'
import type { RegistryCategory, Detail } from '../useRegistry'
import { translations } from './../i18n'
import type { Translation } from './../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'
import { fetchRegistryRaw } from '../lib/registry-raw'
import SiteHeader from '../components/SiteHeader'
import Breadcrumbs from '../components/Breadcrumbs'
import RegistryIcon from '../components/RegistryIcon'
import { useFavorites } from '../lib/useFavorites'
// Fixed top header needs content to start below its 64px band.

interface RegistryPageProps {
  category: RegistryCategory
  onOpenSearch?: () => void
}

interface CategoryMeta {
  docsPath: string                        // docs.librefang.ai path
  registryPath: string                    // github.com/librefang/librefang-registry path
  fileNameFor: (id: string) => string     // TOML / directory pointer inside the registry
}

const CATEGORY_META: Record<RegistryCategory, CategoryMeta> = {
  skills:       { docsPath: '/agent/skills',            registryPath: '/tree/main/skills',    fileNameFor: id => `skills/${id}/SKILL.md` },
  mcp:          { docsPath: '/mcp',                     registryPath: '/tree/main/mcp',       fileNameFor: id => `mcp/${id}.toml` },
  plugins:      { docsPath: '/agent/plugins',           registryPath: '/tree/main/plugins',   fileNameFor: id => `plugins/${id}/plugin.toml` },
  hands:        { docsPath: '/agent/hands',             registryPath: '/tree/main/hands',     fileNameFor: id => `hands/${id}/HAND.toml` },
  agents:       { docsPath: '/agent/templates',         registryPath: '/tree/main/agents',    fileNameFor: id => `agents/${id}/agent.toml` },
  providers:    { docsPath: '/configuration/providers', registryPath: '/tree/main/providers', fileNameFor: id => `providers/${id}.toml` },
  workflows:    { docsPath: '/agent/workflows',         registryPath: '/tree/main/workflows', fileNameFor: id => `workflows/${id}.toml` },
  channels:     { docsPath: '/channels',                registryPath: '/tree/main/channels',  fileNameFor: id => `channels/${id}.toml` },
}

function getCategoryLabels(t: Translation, category: RegistryCategory): { title: string; desc: string } {
  const r = t.registry
  if (!r) return { title: category, desc: '' }
  const entry = r.categories[category]
  if (entry) return entry
  return { title: category, desc: '' }
}

function isPopular(item: Detail) {
  return item.tags?.includes('popular') ?? false
}

type SortKey = 'popular' | 'nameAsc' | 'nameDesc' | 'trending'

function sortItems(items: Detail[], key: SortKey, trendingIds: Map<string, number>): Detail[] {
  const arr = [...items]
  switch (key) {
    case 'nameAsc':
      return arr.sort((a, b) => a.name.localeCompare(b.name))
    case 'nameDesc':
      return arr.sort((a, b) => b.name.localeCompare(a.name))
    case 'trending':
      return arr.sort((a, b) => {
        const ac = trendingIds.get(a.id) ?? -1
        const bc = trendingIds.get(b.id) ?? -1
        if (ac !== bc) return bc - ac
        return a.name.localeCompare(b.name)
      })
    case 'popular':
    default:
      return arr.sort((a, b) => {
        const ap = isPopular(a) ? 0 : 1
        const bp = isPopular(b) ? 0 : 1
        if (ap !== bp) return ap - bp
        return a.name.localeCompare(b.name)
      })
  }
}

const TRENDING_API = 'https://stats.librefang.ai/api/registry/trending'

interface TrendingResp { category: string; top: { id: string; clicks: number }[] }

async function fetchTrending(category: string): Promise<TrendingResp> {
  const res = await fetch(`${TRENDING_API}?category=${encodeURIComponent(category)}`)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return res.json()
}

export default function RegistryPage({ category, onOpenSearch }: RegistryPageProps) {
  const lang = useAppStore(s => s.lang)
  const t = translations[lang] || translations['en']!
  const { data, isLoading, error, refetch, isFetching } = useRegistry()
  const queryClient = useQueryClient()
  const { isFavorite, toggle: toggleFavorite } = useFavorites()
  // Seed from ?category= so bookmarks / shared links preserve the filter.
  // The grid's filter treats query as a substring against id/name/desc/
  // category, so a category name in this slot filters to that chip.
  const [query, setQuery] = useState(() => {
    if (typeof window === 'undefined') return ''
    return new URLSearchParams(window.location.search).get('category') || ''
  })
  const [sortBy, setSortBy] = useState<SortKey>(() => {
    if (typeof window === 'undefined') return 'popular'
    const raw = new URLSearchParams(window.location.search).get('sort')
    return (['popular', 'nameAsc', 'nameDesc', 'trending'] as SortKey[]).includes(raw as SortKey)
      ? (raw as SortKey)
      : 'popular'
  })
  // Keep URL in sync when the filter matches a real category chip; skip
  // arbitrary search text so the URL bar doesn't fill with keystrokes.
  // Also mirror sortBy so bookmarks / shared links preserve the view.
  useEffect(() => {
    if (typeof window === 'undefined') return
    const cats = new Set<string>()
    for (const i of data ? (data[category] ?? []) : []) if (i.category) cats.add(i.category)
    const url = new URL(window.location.href)
    const q = query.trim()
    if (q && cats.has(q)) url.searchParams.set('category', q)
    else url.searchParams.delete('category')
    if (sortBy !== 'popular') url.searchParams.set('sort', sortBy)
    else url.searchParams.delete('sort')
    const next = url.pathname + (url.searchParams.toString() ? '?' + url.searchParams.toString() : '') + url.hash
    const curr = window.location.pathname + window.location.search + window.location.hash
    if (next !== curr) window.history.replaceState(null, '', next)
  }, [query, data, category, sortBy])
  const trendingQuery = useQuery<TrendingResp>({
    queryKey: ['registry-trending', category],
    queryFn: () => fetchTrending(category),
    staleTime: 1000 * 60 * 15,
    retry: 0,
  })

  const { items, count } = getCategoryItems(data, category)
  const labels = getCategoryLabels(t, category)
  const meta = CATEGORY_META[category]

  const trendingIds = useMemo(() => {
    const m = new Map<string, number>()
    for (const row of trendingQuery.data?.top ?? []) m.set(row.id, row.clicks)
    return m
  }, [trendingQuery.data])

  const filtered = useMemo(() => {
    const sorted = sortItems(items, sortBy, trendingIds)
    // Favorites always pin to the top within whatever sort the user picked.
    // Stable partition so relative order inside each group is preserved.
    const pinned: Detail[] = []
    const rest: Detail[] = []
    for (const i of sorted) {
      if (isFavorite(category, i.id)) pinned.push(i)
      else rest.push(i)
    }
    const combined = [...pinned, ...rest]
    if (!query.trim()) return combined
    const q = query.toLowerCase()
    return combined.filter(i => {
      const desc = getLocalizedDesc(i, lang).toLowerCase()
      return i.id.toLowerCase().includes(q)
          || i.name.toLowerCase().includes(q)
          || desc.includes(q)
          || (i.category || '').toLowerCase().includes(q)
          || (i.tags || []).some(tag => tag.toLowerCase().includes(q))
    })
  }, [items, query, lang, sortBy, trendingIds, isFavorite, category])

  const categories = useMemo(() => {
    const set = new Set<string>()
    for (const i of items) if (i.category) set.add(i.category)
    return Array.from(set).sort()
  }, [items])

  const langPrefix = lang === 'en' ? '' : `/${lang}`

  return (
    <main className="min-h-screen bg-surface pt-16">
      <SiteHeader
        isSubpage
        sourceUrl={`https://github.com/librefang/librefang-registry${meta.registryPath}`}
        onOpenSearch={onOpenSearch}
      />

      <section className="max-w-6xl mx-auto px-6 py-10">
        <Breadcrumbs crumbs={[{ label: labels.title }]} className="mb-6" />
        {/* Header */}
        <div className="mb-10">
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">
            {t.registry?.label || 'Registry'} · {count} {t.registry?.total || 'items'}
          </div>
          <h1 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">
            {labels.title}
          </h1>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-3xl">{labels.desc}</p>
        </div>

        {/* Search + Sort */}
        <div className="mb-10 flex flex-col sm:flex-row gap-3 sm:items-start">
          <div className="relative flex-1 max-w-xl">
            <Search className="w-4 h-4 text-gray-400 absolute left-4 top-1/2 -translate-y-1/2" />
            <input
              type="search"
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder={t.registry?.searchPlaceholder || 'Search...'}
              className="w-full pl-11 pr-4 py-3 bg-surface-100 border border-black/10 dark:border-white/10 rounded text-sm text-slate-900 dark:text-white placeholder-gray-400 focus:outline-none focus:border-cyan-500/40 transition-colors"
            />
            {query && (
              <div className="mt-2 text-xs text-gray-500">
                {filtered.length} {t.registry?.matching || 'matches'}
              </div>
            )}
          </div>
          <label className="relative inline-flex items-center gap-2">
            <ArrowUpDown className="w-4 h-4 text-gray-400 absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" />
            <span className="sr-only">{t.registry?.sort?.label || 'Sort'}</span>
            <select
              value={sortBy}
              onChange={e => setSortBy(e.target.value as SortKey)}
              className="pl-9 pr-8 py-3 bg-surface-100 border border-black/10 dark:border-white/10 rounded text-sm text-slate-900 dark:text-white focus:outline-none focus:border-cyan-500/40 transition-colors appearance-none cursor-pointer"
              aria-label={t.registry?.sort?.label || 'Sort'}
            >
              <option value="popular">{t.registry?.sort?.popular || 'Popular'}</option>
              <option value="nameAsc">{t.registry?.sort?.nameAsc || 'Name A–Z'}</option>
              <option value="nameDesc">{t.registry?.sort?.nameDesc || 'Name Z–A'}</option>
              <option value="trending" disabled={!trendingQuery.data || trendingIds.size === 0}>
                {t.registry?.sort?.trending || 'Most clicked'}
              </option>
            </select>
          </label>
        </div>

        {/* Trending strip — shows the top-clicked items in this category.
            Hidden when fewer than 3 clicks exist (noise). */}
        {trendingQuery.data && trendingQuery.data.top.length >= 3 && (() => {
          const idToItem = new Map(items.map(i => [i.id, i]))
          const trending = trendingQuery.data.top
            .map(t => idToItem.get(t.id))
            .filter((x): x is Detail => !!x)
            .slice(0, 5)
          if (trending.length < 3) return null
          return (
            <div className="mb-6 flex flex-wrap items-center gap-2">
              <span className="text-xs font-mono text-amber-500/80 uppercase tracking-widest flex items-center gap-1.5">
                <Sparkles className="w-3 h-3" /> {t.registry?.trending || 'Trending'}
              </span>
              {trending.map(item => {
                const href = `${langPrefix}/${category}/${item.id}`
                return (
                  <a
                    key={item.id}
                    href={href}
                    className="text-xs font-semibold text-cyan-600 dark:text-cyan-400 hover:underline"
                  >
                    {getLocalizedName(item, lang)}
                  </a>
                )
              })}
            </div>
          )
        })()}

        {/* Category chips (click to filter by category string) */}
        {categories.length > 0 && (
          <div className="flex flex-wrap gap-2 mb-8">
            <button
              onClick={() => setQuery('')}
              className={cn(
                'px-3 py-1 text-xs font-mono uppercase tracking-wider border transition-colors',
                query.trim() === '' ? 'border-cyan-500/40 text-cyan-600 dark:text-cyan-400 bg-cyan-500/5' : 'border-black/10 dark:border-white/10 text-gray-500 hover:text-gray-700 dark:hover:text-gray-300'
              )}
            >
              {t.registry?.all || 'All'}
            </button>
            {categories.map(cat => (
              <button
                key={cat}
                onClick={() => setQuery(cat)}
                className="px-3 py-1 text-xs font-mono uppercase tracking-wider border border-black/10 dark:border-white/10 text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 hover:border-cyan-500/30 transition-colors"
                title={cat}
              >
                {t.registry?.subcategories?.[cat] || cat}
              </button>
            ))}
          </div>
        )}

        {/* State: loading — skeleton cards so the grid layout doesn't jump */}
        {isLoading && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3" aria-busy="true" aria-label={t.registry?.loading || 'Loading'}>
            {Array.from({ length: 9 }).map((_, i) => (
              <div key={i} className="border border-black/10 dark:border-white/5 bg-surface-100 p-5 animate-pulse">
                <div className="flex items-center gap-2 mb-3">
                  <div className="h-5 w-5 rounded bg-black/10 dark:bg-white/10" />
                  <div className="h-4 w-32 rounded bg-black/10 dark:bg-white/10" />
                </div>
                <div className="h-2 w-16 rounded bg-black/10 dark:bg-white/10 mb-2" />
                <div className="space-y-1.5">
                  <div className="h-2 w-full rounded bg-black/10 dark:bg-white/10" />
                  <div className="h-2 w-11/12 rounded bg-black/10 dark:bg-white/10" />
                  <div className="h-2 w-3/4 rounded bg-black/10 dark:bg-white/10" />
                </div>
              </div>
            ))}
          </div>
        )}

        {/* State: error */}
        {error && !isLoading && (
          <div className="flex flex-col items-center justify-center py-24 text-center">
            <AlertCircle className="w-6 h-6 text-red-400 mb-3" />
            <div className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
              {t.registry?.errorTitle || 'Could not load registry'}
            </div>
            <div className="text-xs text-gray-500 max-w-sm mb-4">
              {t.registry?.errorDesc || 'GitHub rate limit hit or the proxy is down. Retry in a few seconds.'}
            </div>
            <button
              onClick={() => refetch()}
              disabled={isFetching}
              className="inline-flex items-center gap-2 px-4 py-2 text-sm font-semibold bg-cyan-500/10 hover:bg-cyan-500/20 border border-cyan-500/30 text-cyan-600 dark:text-cyan-400 rounded transition-colors disabled:opacity-50"
            >
              {isFetching
                ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                : <RotateCcw className="w-3.5 h-3.5" />}
              {t.registry?.retry || 'Retry'}
            </button>
          </div>
        )}

        {/* State: empty category */}
        {!isLoading && !error && items.length === 0 && (
          <div className="flex flex-col items-center justify-center py-24 text-center border border-dashed border-black/10 dark:border-white/10 rounded">
            <Sparkles className="w-6 h-6 text-amber-400/60 mb-3" />
            <div className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
              {t.registry?.emptyTitle || 'Nothing here yet'}
            </div>
            <div className="text-xs text-gray-500 max-w-sm mb-4">
              {t.registry?.emptyDesc || `The ${category} section of the registry is not populated yet. Check back soon or contribute one.`}
            </div>
            <a
              href={`https://github.com/librefang/librefang-registry${meta.registryPath}`}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 text-xs font-semibold text-cyan-600 dark:text-cyan-400 hover:text-cyan-500 transition-colors"
            >
              <Github className="w-3.5 h-3.5" />
              {t.registry?.contribute || 'Contribute on GitHub'}
            </a>
          </div>
        )}

        {/* State: empty search */}
        {!isLoading && !error && items.length > 0 && filtered.length === 0 && (
          <div className="flex flex-col items-center justify-center py-16 text-center">
            <Search className="w-5 h-5 text-gray-400 mb-2" />
            <div className="text-sm text-gray-500">{t.registry?.noMatches || 'No matches for'} "{query}"</div>
          </div>
        )}

        {/* Grid */}
        {!isLoading && !error && filtered.length > 0 && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
            {filtered.map(item => {
              const desc = getLocalizedDesc(item, lang)
              const popular = isPopular(item)
              const itemHref = `${langPrefix}/${category}/${item.id}`
              const rawPath = meta.fileNameFor(item.id)
              const prefetch = () => {
                // Warm the detail page's raw-TOML cache the moment the user
                // hovers a card. react-query dedupes if the query is already
                // in-flight or fresh, so it's safe to call on every hover.
                queryClient.prefetchQuery({
                  queryKey: ['registry-raw', rawPath],
                  queryFn: () => fetchRegistryRaw(rawPath),
                  staleTime: 1000 * 60 * 60,
                }).catch(() => { /* prefetch failure is silent */ })
              }
              const starred = isFavorite(category, item.id)
              return (
                <a
                  key={item.id}
                  href={itemHref}
                  onMouseEnter={prefetch}
                  onFocus={prefetch}
                  className={cn(
                    'group relative block border p-5 transition-all hover:-translate-y-0.5',
                    popular
                      ? 'border-amber-500/30 bg-amber-500/5 hover:border-amber-500/50'
                      : 'border-black/10 dark:border-white/5 bg-surface-100 hover:border-cyan-500/30'
                  )}
                >
                  <button
                    type="button"
                    onClick={(e) => { e.preventDefault(); e.stopPropagation(); toggleFavorite(category, item.id) }}
                    aria-label={starred ? 'Unstar' : 'Star'}
                    aria-pressed={starred}
                    className={cn(
                      'absolute top-3 right-3 p-1 transition-colors',
                      starred ? 'text-amber-500' : 'text-gray-300 dark:text-gray-600 hover:text-amber-500'
                    )}
                  >
                    <Star className="w-3.5 h-3.5" fill={starred ? 'currentColor' : 'none'} />
                  </button>
                  <div className="flex items-start justify-between gap-2 mb-3 pr-6">
                    <div className="flex items-center gap-2 min-w-0">
                      {item.icon && (
                        <span className="shrink-0 text-cyan-600 dark:text-cyan-400">
                          <RegistryIcon icon={item.icon} className="w-5 h-5" fallbackClassName="text-xl leading-none" />
                        </span>
                      )}
                      <h3 className="text-base font-bold text-slate-900 dark:text-white truncate">
                        {getLocalizedName(item, lang)}
                      </h3>
                      {popular && <Sparkles className="w-3.5 h-3.5 text-amber-500 shrink-0" />}
                    </div>
                    <ArrowRight className="w-3.5 h-3.5 text-gray-300 dark:text-gray-600 group-hover:text-cyan-500 transition-colors shrink-0 mt-1" />
                  </div>
                  {item.category && (
                    <div className="text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-wider mb-2">
                      {item.category}
                    </div>
                  )}
                  {desc && (
                    <p className="text-sm text-gray-500 dark:text-gray-400 leading-relaxed line-clamp-3">
                      {desc}
                    </p>
                  )}
                  {item.tags && item.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1 mt-3">
                      {item.tags.filter(tag => tag !== 'popular').slice(0, 4).map(tag => (
                        <span key={tag} className="text-[10px] font-mono text-gray-500 border border-black/5 dark:border-white/5 px-1.5 py-0.5">
                          {tag}
                        </span>
                      ))}
                    </div>
                  )}
                </a>
              )
            })}
          </div>
        )}

        {/* Docs link */}
        <div className="mt-12 pt-8 border-t border-black/10 dark:border-white/5 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3">
          <div className="text-xs text-gray-500">
            {t.registry?.sourceHint || 'Data proxied from the librefang-registry repo on GitHub.'}
          </div>
          <a
            href={`https://docs.librefang.ai${meta.docsPath}`}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 text-sm font-semibold text-cyan-600 dark:text-cyan-400 hover:text-cyan-500 transition-colors"
          >
            {t.registry?.readDocs || 'Read the docs'}
            <ExternalLink className="w-3.5 h-3.5" />
          </a>
        </div>
      </section>
    </main>
  )
}
