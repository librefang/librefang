import { useEffect, useMemo, useRef, useState } from 'react'
import { Search, X, Sparkles, Hash } from 'lucide-react'
import { useRegistry, getLocalizedDesc } from '../useRegistry'
import type { RegistryCategory, Detail } from '../useRegistry'
import { translations, type Translation } from '../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

interface SearchDialogProps {
  open: boolean
  onClose: () => void
}

type Hit =
  | { kind: 'item'; category: RegistryCategory; item: Detail }
  | { kind: 'anchor'; id: string; label: string; desc: string }

const CATEGORIES: RegistryCategory[] = [
  'skills', 'hands', 'agents', 'providers', 'workflows', 'channels', 'plugins', 'mcp', 'integrations',
]

const PER_CATEGORY_CAP = 5

function isPopular(d: Detail) {
  return d.tags?.includes('popular') ?? false
}

function scoreText(query: string, ...fields: string[]): number {
  const q = query.toLowerCase()
  const primary = (fields[0] || '').toLowerCase()
  if (primary === q) return 1000
  if (primary.startsWith(q)) return 500
  for (let i = 1; i < fields.length; i++) {
    const f = (fields[i] || '').toLowerCase()
    if (f.startsWith(q)) return 400 - i * 50
    if (f.includes(q)) return 150 - i * 20
  }
  if (primary.includes(q)) return 200
  return 0
}

function scoreHit(query: string, item: Detail, localizedDesc: string): number {
  const q = query.toLowerCase()
  const id = item.id.toLowerCase()
  const name = item.name.toLowerCase()
  const desc = localizedDesc.toLowerCase()
  const cat = (item.category || '').toLowerCase()
  if (id === q) return 1000
  if (id.startsWith(q)) return 500
  if (name.startsWith(q)) return 400
  if (id.includes(q)) return 200
  if (name.includes(q)) return 150
  if (desc.includes(q)) return 50
  if (cat.includes(q)) return 40
  if (item.tags?.some(tag => tag.toLowerCase().includes(q))) return 30
  return 0
}

export default function SearchDialog({ open, onClose }: SearchDialogProps) {
  const lang = useAppStore(s => s.lang)
  const t: Translation = translations[lang] || translations['en']!
  const { data } = useRegistry()
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  // Homepage section anchors that Cmd+K should also reach, even from a
  // registry subpage. Navigating to <homeHref>#<id> lets the browser resolve
  // same-page vs cross-page automatically.
  const anchorHits = useMemo<Hit[]>(() => [
    { kind: 'anchor', id: 'architecture', label: t.nav.architecture, desc: t.architecture?.title || '' },
    { kind: 'anchor', id: 'hands', label: t.nav.hands, desc: t.hands?.title || '' },
    { kind: 'anchor', id: 'workflows', label: t.nav.workflows || t.workflows?.label || 'Workflows', desc: t.workflows?.title || '' },
    { kind: 'anchor', id: 'evolution', label: t.nav.evolution || 'Skills Self-Evolution', desc: t.evolution?.title || '' },
    { kind: 'anchor', id: 'performance', label: t.nav.performance, desc: t.performance?.title || '' },
    { kind: 'anchor', id: 'install', label: t.nav.install, desc: t.install?.title || '' },
    { kind: 'anchor', id: 'downloads', label: t.nav.downloads || 'Downloads', desc: '' },
    { kind: 'anchor', id: 'faq', label: t.faq?.title || 'FAQ', desc: '' },
  ], [t])

  const itemHits = useMemo<Hit[]>(() => {
    if (!data) return []
    const out: Hit[] = []
    for (const cat of CATEGORIES) {
      for (const item of data[cat] ?? []) out.push({ kind: 'item', category: cat, item })
    }
    return out
  }, [data])

  const allHits = useMemo<Hit[]>(() => [...anchorHits, ...itemHits], [anchorHits, itemHits])

  const filtered = useMemo<Hit[]>(() => {
    const q = query.trim()
    if (!q) {
      // No query — show homepage anchors first (navigation shortcut), then a
      // sampling of popular items across categories.
      const perCat = new Map<RegistryCategory, number>()
      const items = itemHits
        .filter(h => h.kind === 'item' && isPopular(h.item))
        .filter(h => {
          if (h.kind !== 'item') return false
          const n = perCat.get(h.category) ?? 0
          if (n >= 3) return false
          perCat.set(h.category, n + 1)
          return true
        })
        .slice(0, 8)
      return [...anchorHits, ...items]
    }
    const scored: { hit: Hit; score: number }[] = []
    for (const h of allHits) {
      let s = 0
      if (h.kind === 'item') {
        const desc = getLocalizedDesc(h.item, lang)
        s = scoreHit(q, h.item, desc)
        if (isPopular(h.item)) s += 5
      } else {
        s = scoreText(q, h.label, h.desc, h.id)
      }
      if (s > 0) scored.push({ hit: h, score: s })
    }
    scored.sort((a, b) => b.score - a.score)
    const perCat = new Map<RegistryCategory, number>()
    const out: Hit[] = []
    for (const { hit } of scored) {
      if (hit.kind === 'item') {
        const n = perCat.get(hit.category) ?? 0
        if (n >= PER_CATEGORY_CAP) continue
        perCat.set(hit.category, n + 1)
      }
      out.push(hit)
      if (out.length >= 40) break
    }
    return out
  }, [allHits, itemHits, anchorHits, query, lang])

  useEffect(() => { setActiveIndex(0) }, [query])

  useEffect(() => {
    if (!open) return
    setQuery('')
    requestAnimationFrame(() => inputRef.current?.focus())
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); onClose() }
      else if (e.key === 'ArrowDown') {
        e.preventDefault()
        setActiveIndex(i => Math.min(filtered.length - 1, i + 1))
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        setActiveIndex(i => Math.max(0, i - 1))
      } else if (e.key === 'Enter') {
        const hit = filtered[activeIndex]
        if (hit) {
          e.preventDefault()
          navigate(hit)
        }
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, filtered, activeIndex])

  const navigate = (hit: Hit) => {
    const prefix = lang === 'en' ? '' : `/${lang}`
    if (hit.kind === 'item') {
      window.location.href = `${prefix}/${hit.category}/${hit.item.id}`
    } else {
      // Anchor: homepage + hash. Same-page case is handled by the browser.
      const home = lang === 'en' ? '/' : `/${lang}/`
      window.location.href = `${home}#${hit.id}`
    }
  }

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-[100] bg-black/40 backdrop-blur-sm flex items-start justify-center pt-[10vh] px-4"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label={t.search?.title || 'Search registry'}
    >
      <div
        className="w-full max-w-2xl bg-surface border border-black/10 dark:border-white/10 rounded-lg shadow-2xl overflow-hidden"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 px-4 py-3 border-b border-black/10 dark:border-white/10">
          <Search className="w-4 h-4 text-gray-400 shrink-0" />
          <input
            ref={inputRef}
            type="search"
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder={t.search?.placeholder || 'Search skills, hands, agents, providers...'}
            className="flex-1 bg-transparent outline-none text-slate-900 dark:text-white placeholder-gray-400 text-sm"
          />
          <button
            onClick={onClose}
            aria-label={t.search?.close || 'Close'}
            className="p-1 text-gray-400 hover:text-slate-900 dark:hover:text-white transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="max-h-[60vh] overflow-y-auto">
          {filtered.length === 0 && (
            <div className="px-4 py-12 text-center text-sm text-gray-500">
              {query.trim()
                ? (t.search?.noResults?.replace('{query}', query) || `No matches for "${query}"`)
                : (t.search?.hint || 'Type to search across all registry entries.')}
            </div>
          )}
          {filtered.map((hit, i) => {
            const isActive = i === activeIndex
            if (hit.kind === 'anchor') {
              return (
                <button
                  key={`anchor:${hit.id}`}
                  onClick={() => navigate(hit)}
                  onMouseEnter={() => setActiveIndex(i)}
                  className={cn(
                    'w-full text-left flex items-center gap-3 px-4 py-3 transition-colors border-l-2',
                    isActive
                      ? 'bg-cyan-500/5 border-cyan-500'
                      : 'border-transparent hover:bg-black/5 dark:hover:bg-white/5'
                  )}
                >
                  <Hash className="w-4 h-4 text-cyan-500/60 shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-0.5">
                      <span className="text-sm font-bold text-slate-900 dark:text-white truncate">{hit.label}</span>
                      <span className="ml-auto text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-wider shrink-0">
                        {t.nav.learnMore || 'Section'}
                      </span>
                    </div>
                    {hit.desc && <p className="text-xs text-gray-500 line-clamp-1">{hit.desc}</p>}
                  </div>
                </button>
              )
            }
            const desc = getLocalizedDesc(hit.item, lang)
            const catLabel = t.registry?.categories[hit.category]?.title || hit.category
            const popular = isPopular(hit.item)
            return (
              <button
                key={`item:${hit.category}:${hit.item.id}`}
                onClick={() => navigate(hit)}
                onMouseEnter={() => setActiveIndex(i)}
                className={cn(
                  'w-full text-left flex items-start gap-3 px-4 py-3 transition-colors border-l-2',
                  isActive
                    ? 'bg-cyan-500/5 border-cyan-500'
                    : 'border-transparent hover:bg-black/5 dark:hover:bg-white/5'
                )}
              >
                {hit.item.icon && (
                  <span className="text-xl leading-none shrink-0" aria-hidden>{hit.item.icon}</span>
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-0.5">
                    <span className="text-sm font-bold text-slate-900 dark:text-white truncate">
                      {hit.item.name}
                    </span>
                    {popular && <Sparkles className="w-3 h-3 text-amber-500 shrink-0" />}
                    <span className="ml-auto text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-wider shrink-0">
                      {catLabel}
                    </span>
                  </div>
                  {desc && (
                    <p className="text-xs text-gray-500 line-clamp-1">{desc}</p>
                  )}
                </div>
              </button>
            )
          })}
        </div>

        <div className="px-4 py-2 text-[10px] font-mono text-gray-400 dark:text-gray-600 border-t border-black/10 dark:border-white/10 flex items-center justify-between">
          <span>
            {t.search?.kbd || '↑↓ navigate · ↵ open · esc close'}
          </span>
          {data && (
            <span>
              {itemHits.length} {t.registry?.total || 'items'}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}
