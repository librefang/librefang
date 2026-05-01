import { useMemo, useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft, ArrowRight, Loader2, AlertCircle, ExternalLink, Sparkles, Copy, Check, Terminal, FileText, RotateCcw, Link as LinkIcon, Download, Star, TrendingUp } from 'lucide-react'
import { useState } from 'react'
import { useRegistry, getLocalizedDesc, getLocalizedName, getCategoryItems } from '../useRegistry'
import type { RegistryCategory, Detail } from '../useRegistry'
import { translations } from '../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'
import { highlightToml } from '../lib/toml-highlight'
import { renderMarkdown } from '../lib/minimal-markdown'
import SiteHeader from '../components/SiteHeader'
import Breadcrumbs from '../components/Breadcrumbs'
import RegistryIcon from '../components/RegistryIcon'
import { fetchRegistryRaw, pathCandidatesFor, fetchFirstAvailable } from '../lib/registry-raw'
import { useMarketplace } from '../lib/useMarketplace'

interface RegistryDetailPageProps {
  category: RegistryCategory
  id: string
  onOpenSearch?: () => void
}

const COMMIT_API = 'https://stats.librefang.ai/api/registry/commit'
const CLICK_API = 'https://stats.librefang.ai/api/registry/click'

interface CommitInfo { sha: string | null; date: string | null; message: string | null }

// Compact relative time: "3d ago", "2mo ago", "1y ago". Falls back to absolute
// date for anything older than ~a year or if parsing fails.
function relTime(iso: string | null): string {
  if (!iso) return ''
  const then = new Date(iso).getTime()
  if (Number.isNaN(then)) return ''
  const diff = Date.now() - then
  const sec = Math.round(diff / 1000)
  if (sec < 60) return `${sec}s ago`
  const min = Math.round(sec / 60)
  if (min < 60) return `${min}m ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  if (day < 30) return `${day}d ago`
  const mo = Math.round(day / 30)
  if (mo < 12) return `${mo}mo ago`
  return new Date(iso).toISOString().slice(0, 10)
}


// README-ish path candidates by category. skills also ship a SKILL.md with the
// prompt body (that's the canonical doc for them); everything else uses a
// README.md if present.
function readmePathsFor(category: RegistryCategory, id: string): string[] {
  switch (category) {
    case 'skills': return [`skills/${id}/SKILL.md`, `skills/${id}/README.md`]
    case 'hands':  return [`hands/${id}/README.md`]
    case 'agents': return [`agents/${id}/README.md`]
    default:       return []
  }
}

// Commands the CLI actually exposes (verified against librefang-cli/src/main.rs).
// Categories without an install-by-id subcommand get a different hint.
const COMMAND_TEMPLATE: Partial<Record<RegistryCategory, string>> = {
  skills:       'librefang skill install {id}',
  hands:        'librefang hand activate {id}',
  agents:       'librefang agent new {id}',
  channels:     'librefang channel setup {id}',
  // `librefang mcp add <name>` is the one-click MCP-server installer.
  mcp:          'librefang mcp add {id}',
}

function isPopular(item: Detail | undefined) {
  return item?.tags?.includes('popular') ?? false
}

function AnchorLink({ id, title }: { id: string; title: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <a
      href={`#${id}`}
      onClick={(e) => {
        e.preventDefault()
        const url = `${window.location.origin}${window.location.pathname}#${id}`
        navigator.clipboard.writeText(url)
        // Also update history so the hash is visible in the URL bar.
        history.replaceState(null, '', `#${id}`)
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      }}
      aria-label={title}
      title={title}
      className="opacity-0 group-hover:opacity-70 hover:!opacity-100 ml-1 inline-flex items-center text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-opacity"
    >
      {copied ? <Check className="w-3 h-3" /> : <LinkIcon className="w-3 h-3" />}
    </a>
  )
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <button
      onClick={() => {
        navigator.clipboard.writeText(text)
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      }}
      className="inline-flex items-center gap-1.5 px-3 py-1 text-xs font-mono text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors border border-black/10 dark:border-white/10 rounded"
    >
      {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
      {label}
    </button>
  )
}

export default function RegistryDetailPage({ category, id, onOpenSearch }: RegistryDetailPageProps) {
  const lang = useAppStore(s => s.lang)
  const t = translations[lang] || translations['en']!
  const { data: registry } = useRegistry()

  const { items } = getCategoryItems(registry, category)
  const item = useMemo(() => items.find(x => x.id === id), [items, id])
  // Related = same category, excluding self. Popular first, then alphabetical,
  // cap at 6 so the section is a browse surface not a wall of text.
  const related = useMemo(() => {
    const rest = items.filter(x => x.id !== id)
    rest.sort((a, b) => {
      const ap = a.tags?.includes('popular') ? 0 : 1
      const bp = b.tags?.includes('popular') ? 0 : 1
      if (ap !== bp) return ap - bp
      return a.name.localeCompare(b.name)
    })
    return rest.slice(0, 6)
  }, [items, id])

  // Prev/next for the bottom-of-page navigation strip. Same sort as the list
  // page so "next" matches what the visitor would expect from the grid.
  const sortedCategory = useMemo(() => {
    const sorted = [...items]
    sorted.sort((a, b) => {
      const ap = a.tags?.includes('popular') ? 0 : 1
      const bp = b.tags?.includes('popular') ? 0 : 1
      if (ap !== bp) return ap - bp
      return a.name.localeCompare(b.name)
    })
    return sorted
  }, [items])
  const currentIdx = sortedCategory.findIndex(x => x.id === id)
  const prevItem = currentIdx > 0 ? sortedCategory[currentIdx - 1] : undefined
  const nextItem = currentIdx >= 0 && currentIdx < sortedCategory.length - 1 ? sortedCategory[currentIdx + 1] : undefined

  const pathCandidates = pathCandidatesFor(category, id)
  // Cache key is the primary path so the hover-prefetch on the list page
  // (which only knows the preferred layout) warms the same slot.
  const primaryPath = pathCandidates[0]!
  const rawQuery = useQuery({
    queryKey: ['registry-raw', primaryPath],
    queryFn: () => fetchFirstAvailable(pathCandidates),
    staleTime: 1000 * 60 * 60,
    retry: 1,
  })
  // Resolved path — the candidate that actually returned content, or the
  // primary guess if the query hasn't succeeded yet. GitHub/commit URLs
  // use this so they point at the file the user is actually viewing.
  const rawPath = rawQuery.data?.path ?? primaryPath
  // Fire-and-forget click tracking so trending can surface on list pages.
  // navigator.sendBeacon is queued by the browser even on unload, and doesn't
  // block the page at all. Some browsers fall back to fetch keepalive.
  useEffect(() => {
    const body = JSON.stringify({ category, id })
    try {
      if ('sendBeacon' in navigator) {
        const blob = new Blob([body], { type: 'application/json' })
        navigator.sendBeacon(CLICK_API, blob)
      } else {
        fetch(CLICK_API, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body,
          keepalive: true,
        }).catch(() => { /* ignore */ })
      }
    } catch { /* ignore */ }
  }, [category, id])

  // Try each candidate README path in order until one 200s. Skills have
  // SKILL.md as canonical, other categories may or may not have a README.
  const readmeQuery = useQuery<string | null>({
    queryKey: ['registry-readme', category, id],
    queryFn: async () => {
      for (const p of readmePathsFor(category, id)) {
        try {
          const md = await fetchRegistryRaw(p)
          if (md && md.trim()) return md
        } catch { /* try next */ }
      }
      return null
    },
    staleTime: 1000 * 60 * 60,
    retry: 0,
  })

  const commitQuery = useQuery<CommitInfo>({
    // Wait until raw resolves so we only look up commits for the path
    // that actually exists. Otherwise we'd race two requests for MCP
    // entries — one against the file-backed path, one against the
    // dir-backed path — and one would always 404.
    enabled: !!rawQuery.data?.path,
    queryKey: ['registry-commit', rawPath],
    queryFn: async () => {
      const res = await fetch(`${COMMIT_API}?path=${encodeURIComponent(rawPath)}`)
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      return res.json()
    },
    staleTime: 1000 * 60 * 60 * 6,
    retry: 1,
  })

  const marketplace = useMarketplace(category)
  const mktPkg = marketplace.get(id)

  const catHref = lang === 'en' ? `/${category}` : `/${lang}/${category}`
  const categoryLabel = t.registry?.categories[category]?.title || category
  const desc = item ? getLocalizedDesc(item, lang) : ''
  const displayName = item ? getLocalizedName(item, lang) : id
  const popular = isPopular(item)

  return (
    <main className="min-h-screen bg-surface pt-16">
      <SiteHeader
        isSubpage
        sourceUrl={`https://github.com/librefang/librefang-registry/blob/main/${rawPath}`}
        onOpenSearch={onOpenSearch}
      />

      <div className="max-w-6xl mx-auto px-6 py-8 lg:grid lg:grid-cols-[200px_1fr] lg:gap-12">
        <div className="lg:col-span-2 mb-6">
          <Breadcrumbs crumbs={[
            { label: categoryLabel, href: catHref },
            { label: displayName },
          ]} />
        </div>
        {/* Sticky TOC — hidden below lg, otherwise pinned in the left gutter. */}
        <aside className="hidden lg:block">
          <nav className="sticky top-24 text-xs" aria-label={t.registry?.onThisPage || 'On this page'}>
            <div className="font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest mb-3">
              {t.registry?.onThisPage || 'On this page'}
            </div>
            <ul className="space-y-2">
              <li><a href="#use-it" className="text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{t.registry?.useIt || 'Use it'}</a></li>
              <li><a href="#manifest" className="text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{t.registry?.manifest || 'Manifest'}</a></li>
              <li><a href="#related" className="text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{(t.registry?.relatedIn || 'More {category}').replace('{category}', categoryLabel)}</a></li>
            </ul>
          </nav>
        </aside>

      <section className="max-w-4xl mx-auto px-0 py-0">
        {/* Header card */}
        <div className={cn(
          'border p-6 md:p-8 mb-8',
          popular ? 'border-amber-500/30 bg-amber-500/5' : 'border-black/10 dark:border-white/5 bg-surface-100'
        )}>
          <div className="flex items-start gap-4 mb-4">
            {item?.icon && (
              <div className="shrink-0 text-cyan-600 dark:text-cyan-400">
                <RegistryIcon icon={item.icon} className="w-10 h-10" fallbackClassName="text-4xl leading-none" />
              </div>
            )}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 mb-2">
                <h1 className="text-2xl md:text-3xl font-black text-slate-900 dark:text-white tracking-tight truncate">
                  {displayName}
                </h1>
                {popular && <Sparkles className="w-4 h-4 text-amber-500 shrink-0" />}
              </div>
              <div className="flex flex-wrap items-center gap-2 text-xs font-mono">
                <code className="text-gray-500 dark:text-gray-400">{id}</code>
                {item?.category && (
                  <>
                    <span className="text-gray-300 dark:text-gray-700">·</span>
                    <span className="text-gray-400 dark:text-gray-600 uppercase tracking-wider">{item.category}</span>
                  </>
                )}
              </div>
            </div>
          </div>

          {desc && (
            <p className="text-gray-600 dark:text-gray-400 text-base leading-relaxed mb-4">
              {desc}
            </p>
          )}

          {item?.tags && item.tags.length > 0 && (
            <div className="flex flex-wrap gap-2 mb-3">
              {item.tags.filter(tag => tag !== 'popular').map(tag => (
                <span key={tag} className="text-xs font-mono text-gray-500 border border-black/10 dark:border-white/10 px-2 py-0.5">
                  {tag}
                </span>
              ))}
            </div>
          )}
          {mktPkg && (mktPkg.total_downloads > 0 || mktPkg.stars > 0 || mktPkg.latest_version) && (
            <div className="flex flex-wrap items-center gap-3 mb-4 p-3 bg-black/3 dark:bg-white/3 border border-black/8 dark:border-white/8 rounded">
              {mktPkg.latest_version && (
                <span className="flex items-center gap-1.5 text-xs font-mono font-semibold text-cyan-600 dark:text-cyan-400 bg-cyan-500/10 px-2 py-1 rounded">
                  v{mktPkg.latest_version}
                </span>
              )}
              {mktPkg.total_downloads > 0 && (
                <span className="flex items-center gap-1.5 text-xs font-mono text-gray-600 dark:text-gray-300">
                  <Download className="w-3.5 h-3.5 text-cyan-500/70" />
                  <strong>{mktPkg.total_downloads >= 1000 ? `${(mktPkg.total_downloads / 1000).toFixed(1)}k` : mktPkg.total_downloads}</strong>
                  <span className="text-gray-400">{t.registry?.downloads || 'downloads'}</span>
                </span>
              )}
              {mktPkg.weekly_downloads > 0 && (
                <span className="flex items-center gap-1.5 text-xs font-mono text-green-600 dark:text-green-400">
                  <TrendingUp className="w-3.5 h-3.5" />
                  <strong>{mktPkg.weekly_downloads >= 1000 ? `${(mktPkg.weekly_downloads / 1000).toFixed(1)}k` : mktPkg.weekly_downloads}</strong>
                  <span className="text-gray-400">{t.registry?.thisWeek || 'this week'}</span>
                </span>
              )}
              {mktPkg.stars > 0 && (
                <span className="flex items-center gap-1.5 text-xs font-mono text-amber-500">
                  <Star className="w-3.5 h-3.5" fill="currentColor" />
                  <strong>{mktPkg.stars}</strong>
                </span>
              )}
            </div>
          )}
          {commitQuery.data?.date && (
            <div
              className="text-[11px] font-mono text-gray-400 dark:text-gray-600 flex items-center gap-3"
              title={commitQuery.data.message ? `${commitQuery.data.message} — ${commitQuery.data.date}` : commitQuery.data.date}
            >
              <span>{t.registry?.lastUpdated || 'Updated'} {relTime(commitQuery.data.date)}</span>
              <a
                href={`https://github.com/librefang/librefang-registry/commits/main/${rawPath}`}
                target="_blank"
                rel="noopener noreferrer"
                className="text-cyan-600 dark:text-cyan-400 hover:underline"
              >
                {category === 'agents'
                  ? (t.registry?.templateDiff || 'Template diff')
                  : (t.registry?.viewHistory || 'History')}
              </a>
            </div>
          )}
        </div>

        {/* Install / use command */}
        {COMMAND_TEMPLATE[category] ? (
          <div id="use-it" className="mb-8 group scroll-mt-20">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-xs font-mono text-gray-500 uppercase tracking-widest flex items-center gap-2">
                <Terminal className="w-3.5 h-3.5" />
                {t.registry?.useIt || 'Use it'}
                <AnchorLink id="use-it" title={t.registry?.copyLink || 'Copy link'} />
              </h2>
              <CopyButton text={COMMAND_TEMPLATE[category]!.replace('{id}', id)} label={t.registry?.copy || 'Copy'} />
            </div>
            <pre className="overflow-x-auto text-sm font-mono leading-relaxed p-4 bg-slate-950/90 dark:bg-black text-gray-100 border border-cyan-500/20">
              <code>
                <span className="text-cyan-400 select-none">$ </span>
                {COMMAND_TEMPLATE[category]!.replace('{id}', id)}
              </code>
            </pre>
            {/* Secondary: open the local dashboard if one is running. We don't
                try to POST from the website (mixed-content + CORS friction);
                we just deep-link. Clicking errors cleanly if no daemon is up. */}
            <a
              href={`http://127.0.0.1:4545/${
                category === 'hands' ? 'hands'
                : category === 'agents' ? 'agents'
                : category === 'channels' ? 'channels'
                : category === 'mcp' ? 'mcp-servers'
                : 'skills'
              }`}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-3 inline-flex items-center gap-1.5 text-xs text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            >
              {t.registry?.openInDashboard || 'Or install via local dashboard'}
              <ExternalLink className="w-3 h-3" />
            </a>
          </div>
        ) : (
          <div className="mb-8 flex items-start gap-3 p-4 border border-black/10 dark:border-white/5 bg-surface-100">
            <FileText className="w-4 h-4 text-gray-400 shrink-0 mt-0.5" />
            <p className="text-sm text-gray-500 leading-relaxed">
              {t.registry?.configOnly?.replace('{category}', categoryLabel) ||
                `${categoryLabel} entries are configured through ~/.librefang/config.toml rather than a CLI install command. Copy the manifest below and paste it into the matching section of your config.`}
            </p>
          </div>
        )}

        {/* Manifest */}
        <div id="manifest" className="mb-6 flex items-center justify-between group scroll-mt-20">
          <h2 className="text-xs font-mono text-gray-500 uppercase tracking-widest flex items-center">
            {t.registry?.manifest || 'Manifest'}
            <AnchorLink id="manifest" title={t.registry?.copyLink || 'Copy link'} />
          </h2>
          {rawQuery.data && (
            <CopyButton text={rawQuery.data.content} label={t.registry?.copy || 'Copy'} />
          )}
        </div>

        {rawQuery.isLoading && (
          <div className="flex items-center justify-center py-16 text-gray-400">
            <Loader2 className="w-5 h-5 animate-spin mr-2" />
            <span className="text-sm">{t.registry?.loading || 'Loading…'}</span>
          </div>
        )}

        {rawQuery.error && !rawQuery.isLoading && (
          <div className="flex flex-col items-center justify-center py-16 text-center border border-red-500/20 bg-red-500/5">
            <AlertCircle className="w-5 h-5 text-red-400 mb-2" />
            <div className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-1">
              {t.registry?.manifestErrorTitle || 'Could not load manifest'}
            </div>
            <div className="text-xs text-gray-500 max-w-md mb-4">
              {(rawQuery.error as Error).message}
            </div>
            <button
              onClick={() => rawQuery.refetch()}
              disabled={rawQuery.isFetching}
              className="inline-flex items-center gap-2 px-4 py-1.5 text-xs font-semibold bg-cyan-500/10 hover:bg-cyan-500/20 border border-cyan-500/30 text-cyan-600 dark:text-cyan-400 rounded transition-colors disabled:opacity-50"
            >
              {rawQuery.isFetching
                ? <Loader2 className="w-3 h-3 animate-spin" />
                : <RotateCcw className="w-3 h-3" />}
              {t.registry?.retry || 'Retry'}
            </button>
          </div>
        )}

        {rawQuery.data && (
          <pre className="overflow-x-auto text-xs md:text-sm font-mono leading-relaxed p-5 bg-surface-100 border border-black/10 dark:border-white/5 text-gray-700 dark:text-gray-300 whitespace-pre toml-highlight">
            <code>{highlightToml(rawQuery.data.content)}</code>
          </pre>
        )}

        {/* README — rendered inline when an adjacent README/SKILL.md exists. */}
        {readmeQuery.data && (
          <div id="readme" className="mt-12 pt-8 border-t border-black/10 dark:border-white/5 group scroll-mt-20">
            <h2 className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-4 flex items-center">
              {t.registry?.readme || 'README'}
              <AnchorLink id="readme" title={t.registry?.copyLink || 'Copy link'} />
            </h2>
            <div className="max-w-none">
              {renderMarkdown(readmeQuery.data)}
            </div>
          </div>
        )}

        {/* Related items in the same category */}
        {related.length > 0 && (
          <div id="related" className="mt-12 pt-8 border-t border-black/10 dark:border-white/5 group scroll-mt-20">
            <h2 className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-4 flex items-center">
              {t.registry?.relatedIn?.replace('{category}', categoryLabel) || `More ${categoryLabel}`}
              <AnchorLink id="related" title={t.registry?.copyLink || 'Copy link'} />
            </h2>
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
              {related.map(rel => {
                const relDesc = getLocalizedDesc(rel, lang)
                const relPopular = rel.tags?.includes('popular')
                const relHref = `${lang === 'en' ? '' : `/${lang}`}/${category}/${rel.id}`
                return (
                  <a
                    key={rel.id}
                    href={relHref}
                    className={cn(
                      'group block border p-4 transition-all hover:-translate-y-0.5',
                      relPopular
                        ? 'border-amber-500/30 bg-amber-500/5 hover:border-amber-500/50'
                        : 'border-black/10 dark:border-white/5 bg-surface-100 hover:border-cyan-500/30'
                    )}
                  >
                    <div className="flex items-center gap-2 mb-1.5 min-w-0">
                      {rel.icon && (
                        <span className="shrink-0 text-cyan-600 dark:text-cyan-400">
                          <RegistryIcon icon={rel.icon} className="w-4 h-4" fallbackClassName="text-lg leading-none" />
                        </span>
                      )}
                      <h3 className="text-sm font-bold text-slate-900 dark:text-white truncate">{getLocalizedName(rel, lang)}</h3>
                      {relPopular && <Sparkles className="w-3 h-3 text-amber-500 shrink-0" />}
                    </div>
                    {relDesc && (
                      <p className="text-xs text-gray-500 leading-relaxed line-clamp-2">{relDesc}</p>
                    )}
                  </a>
                )
              })}
            </div>
          </div>
        )}

        {/* Prev / next within the current category. Matches the sorted
            order used by the list page so "next" feels predictable. */}
        {(prevItem || nextItem) && (
          <nav className="mt-10 pt-6 border-t border-black/10 dark:border-white/5 grid grid-cols-2 gap-3" aria-label={t.registry?.prevNext || 'Previous / next in category'}>
            {prevItem ? (
              <a
                href={`${lang === 'en' ? '' : `/${lang}`}/${category}/${prevItem.id}`}
                className="group flex flex-col items-start gap-1 p-3 border border-black/10 dark:border-white/5 hover:border-cyan-500/30 bg-surface-100 transition-colors min-w-0"
              >
                <span className="text-[10px] font-mono uppercase tracking-wider text-gray-400 flex items-center gap-1">
                  <ArrowLeft className="w-3 h-3" /> {t.registry?.previous || 'Previous'}
                </span>
                <span className="text-sm font-bold text-slate-900 dark:text-white group-hover:text-cyan-600 dark:group-hover:text-cyan-400 truncate max-w-full">
                  {prevItem.name}
                </span>
              </a>
            ) : <div />}
            {nextItem ? (
              <a
                href={`${lang === 'en' ? '' : `/${lang}`}/${category}/${nextItem.id}`}
                className="group flex flex-col items-end gap-1 p-3 border border-black/10 dark:border-white/5 hover:border-cyan-500/30 bg-surface-100 transition-colors text-right min-w-0"
              >
                <span className="text-[10px] font-mono uppercase tracking-wider text-gray-400 flex items-center gap-1">
                  {t.registry?.next || 'Next'} <ArrowRight className="w-3 h-3" />
                </span>
                <span className="text-sm font-bold text-slate-900 dark:text-white group-hover:text-cyan-600 dark:group-hover:text-cyan-400 truncate max-w-full">
                  {nextItem.name}
                </span>
              </a>
            ) : <div />}
          </nav>
        )}

        {/* Back to category */}
        <div className="mt-6 pt-4">
          <a
            href={catHref}
            className="inline-flex items-center gap-2 text-sm font-semibold text-cyan-600 dark:text-cyan-400 hover:text-cyan-500 transition-colors"
          >
            <ArrowLeft className="w-3.5 h-3.5" />
            {t.registry?.allIn?.replace('{category}', categoryLabel) || `All ${categoryLabel}`}
          </a>
        </div>
      </section>
      </div>
    </main>
  )
}
