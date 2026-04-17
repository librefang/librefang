import { useMemo, useState } from 'react'
import { ArrowLeft, Search, Loader2, AlertCircle, ExternalLink, Sparkles, Github } from 'lucide-react'
import { useRegistry, getLocalizedDesc, getCategoryItems } from '../useRegistry'
import type { RegistryCategory, Detail } from '../useRegistry'
import { translations } from './../i18n'
import type { Translation } from './../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

interface RegistryPageProps {
  category: RegistryCategory
}

interface CategoryMeta {
  docsPath: string                        // docs.librefang.ai path
  registryPath: string                    // github.com/librefang/librefang-registry path
  fileNameFor: (id: string) => string     // TOML / directory pointer inside the registry
}

const CATEGORY_META: Record<RegistryCategory, CategoryMeta> = {
  skills:       { docsPath: '/agent/skills',            registryPath: '/tree/main/skills',       fileNameFor: id => `skills/${id}/SKILL.toml` },
  mcp:          { docsPath: '/integrations/mcp-a2a',    registryPath: '/tree/main/mcp',          fileNameFor: id => `mcp/${id}.toml` },
  plugins:      { docsPath: '/agent/plugins',           registryPath: '/tree/main/plugins',      fileNameFor: id => `plugins/${id}.toml` },
  hands:        { docsPath: '/agent/hands',             registryPath: '/tree/main/hands',        fileNameFor: id => `hands/${id}/HAND.toml` },
  agents:       { docsPath: '/agent/templates',         registryPath: '/tree/main/agents',       fileNameFor: id => `agents/${id}/AGENT.toml` },
  providers:    { docsPath: '/configuration/providers', registryPath: '/tree/main/providers',    fileNameFor: id => `providers/${id}.toml` },
  workflows:    { docsPath: '/agent/workflows',         registryPath: '/tree/main/workflows',    fileNameFor: id => `workflows/${id}.toml` },
  channels:     { docsPath: '/integrations/channels',   registryPath: '/tree/main/channels',     fileNameFor: id => `channels/${id}.toml` },
  integrations: { docsPath: '/integrations',            registryPath: '/tree/main/integrations', fileNameFor: id => `integrations/${id}.toml` },
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

function sortItems(items: Detail[]): Detail[] {
  return [...items].sort((a, b) => {
    const ap = isPopular(a) ? 0 : 1
    const bp = isPopular(b) ? 0 : 1
    if (ap !== bp) return ap - bp
    return a.name.localeCompare(b.name)
  })
}

export default function RegistryPage({ category }: RegistryPageProps) {
  const lang = useAppStore(s => s.lang)
  const t = translations[lang] || translations['en']!
  const { data, isLoading, error } = useRegistry()
  const [query, setQuery] = useState('')

  const { items, count } = getCategoryItems(data, category)
  const labels = getCategoryLabels(t, category)
  const meta = CATEGORY_META[category]

  const filtered = useMemo(() => {
    const sorted = sortItems(items)
    if (!query.trim()) return sorted
    const q = query.toLowerCase()
    return sorted.filter(i => {
      const desc = getLocalizedDesc(i, lang).toLowerCase()
      return i.id.toLowerCase().includes(q)
          || i.name.toLowerCase().includes(q)
          || desc.includes(q)
          || (i.category || '').toLowerCase().includes(q)
          || (i.tags || []).some(tag => tag.toLowerCase().includes(q))
    })
  }, [items, query, lang])

  const categories = useMemo(() => {
    const set = new Set<string>()
    for (const i of items) if (i.category) set.add(i.category)
    return Array.from(set).sort()
  }, [items])

  const baseHref = lang === 'en' ? '/' : `/${lang}/`

  return (
    <main className="min-h-screen bg-surface">
      {/* Top bar */}
      <div className="border-b border-black/10 dark:border-white/5 bg-surface-100">
        <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between">
          <a href={baseHref} className="flex items-center gap-2 text-sm text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">
            <ArrowLeft className="w-4 h-4" />
            <span>{t.registry?.backHome || 'Home'}</span>
          </a>
          <a href="https://github.com/librefang/librefang-registry" target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 text-xs text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-mono">
            <Github className="w-3.5 h-3.5" />
            <span>librefang/librefang-registry</span>
            <ExternalLink className="w-3 h-3" />
          </a>
        </div>
      </div>

      <section className="max-w-6xl mx-auto px-6 py-14">
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

        {/* Search */}
        <div className="relative mb-10 max-w-xl">
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
              >
                {cat}
              </button>
            ))}
          </div>
        )}

        {/* State: loading */}
        {isLoading && (
          <div className="flex flex-col items-center justify-center py-24 text-gray-400">
            <Loader2 className="w-6 h-6 animate-spin mb-3" />
            <span className="text-sm">{t.registry?.loading || 'Loading registry...'}</span>
          </div>
        )}

        {/* State: error */}
        {error && !isLoading && (
          <div className="flex flex-col items-center justify-center py-24 text-center">
            <AlertCircle className="w-6 h-6 text-red-400 mb-3" />
            <div className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
              {t.registry?.errorTitle || 'Could not load registry'}
            </div>
            <div className="text-xs text-gray-500 max-w-sm">
              {t.registry?.errorDesc || 'GitHub rate limit hit or the proxy is down. Retry in a few seconds.'}
            </div>
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
              const sourcePath = meta.fileNameFor(item.id)
              return (
                <a
                  key={item.id}
                  href={`https://github.com/librefang/librefang-registry/blob/main/${sourcePath}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={cn(
                    'group block border p-5 transition-all hover:-translate-y-0.5',
                    popular
                      ? 'border-amber-500/30 bg-amber-500/5 hover:border-amber-500/50'
                      : 'border-black/10 dark:border-white/5 bg-surface-100 hover:border-cyan-500/30'
                  )}
                >
                  <div className="flex items-start justify-between gap-2 mb-3">
                    <div className="flex items-center gap-2 min-w-0">
                      {item.icon && (
                        <span className="text-xl leading-none shrink-0" aria-hidden>
                          {item.icon}
                        </span>
                      )}
                      <h3 className="text-base font-bold text-slate-900 dark:text-white truncate">
                        {item.name}
                      </h3>
                      {popular && <Sparkles className="w-3.5 h-3.5 text-amber-500 shrink-0" />}
                    </div>
                    <ExternalLink className="w-3.5 h-3.5 text-gray-300 dark:text-gray-600 group-hover:text-cyan-500 transition-colors shrink-0 mt-1" />
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
