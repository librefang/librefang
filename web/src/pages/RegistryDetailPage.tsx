import { useMemo, useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft, Loader2, AlertCircle, ExternalLink, Sparkles, Github, Copy, Check, Terminal, FileText, RotateCcw, Link as LinkIcon } from 'lucide-react'
import { useState } from 'react'
import { useRegistry, getLocalizedDesc, getCategoryItems } from '../useRegistry'
import type { RegistryCategory, Detail } from '../useRegistry'
import { translations } from '../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'
import { highlightToml } from '../lib/toml-highlight'
import { fetchRegistryRaw } from '../lib/registry-raw'

interface RegistryDetailPageProps {
  category: RegistryCategory
  id: string
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

// How to resolve a category + id to a file path inside librefang-registry.
// Directory-backed categories (hands/agents/skills) use <UPPER>.toml inside
// <id>/ ; file-backed categories just use <id>.toml at the top level.
function pathFor(category: RegistryCategory, id: string): string {
  switch (category) {
    case 'hands':  return `hands/${id}/HAND.toml`
    case 'agents': return `agents/${id}/AGENT.toml`
    case 'skills': return `skills/${id}/SKILL.toml`
    case 'mcp':    return `mcp/${id}.toml`
    default:       return `${category}/${id}.toml`
  }
}

// Commands the CLI actually exposes (verified against librefang-cli/src/main.rs).
// Categories without an install-by-id subcommand get a different hint.
const COMMAND_TEMPLATE: Partial<Record<RegistryCategory, string>> = {
  skills:   'librefang skill install {id}',
  hands:    'librefang hand activate {id}',
  agents:   'librefang agent new {id}',
  channels: 'librefang channel setup {id}',
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

export default function RegistryDetailPage({ category, id }: RegistryDetailPageProps) {
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

  const rawPath = pathFor(category, id)
  const rawQuery = useQuery({
    queryKey: ['registry-raw', rawPath],
    queryFn: () => fetchRegistryRaw(rawPath),
    staleTime: 1000 * 60 * 60,
    retry: 1,
  })
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

  const commitQuery = useQuery<CommitInfo>({
    queryKey: ['registry-commit', rawPath],
    queryFn: async () => {
      const res = await fetch(`${COMMIT_API}?path=${encodeURIComponent(rawPath)}`)
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      return res.json()
    },
    staleTime: 1000 * 60 * 60 * 6,
    retry: 1,
  })

  const baseHref = lang === 'en' ? '/' : `/${lang}/`
  const catHref = lang === 'en' ? `/${category}` : `/${lang}/${category}`
  const categoryLabel = t.registry?.categories[category]?.title || category
  const desc = item ? getLocalizedDesc(item, lang) : ''
  const popular = isPopular(item)

  return (
    <main className="min-h-screen bg-surface">
      <div className="border-b border-black/10 dark:border-white/5 bg-surface-100">
        <div className="max-w-4xl mx-auto px-6 h-16 flex items-center justify-between">
          <nav className="flex items-center gap-1.5 text-sm text-gray-500">
            <a href={baseHref} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors inline-flex items-center gap-1">
              <ArrowLeft className="w-3.5 h-3.5" />
              {t.registry?.backHome || 'Home'}
            </a>
            <span className="text-gray-300 dark:text-gray-700">/</span>
            <a href={catHref} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">
              {categoryLabel}
            </a>
            <span className="text-gray-300 dark:text-gray-700">/</span>
            <span className="text-slate-900 dark:text-white font-semibold truncate max-w-[180px] md:max-w-none">{item?.name || id}</span>
          </nav>
          <a
            href={`https://github.com/librefang/librefang-registry/blob/main/${rawPath}`}
            target="_blank"
            rel="noopener noreferrer"
            className="flex items-center gap-2 text-xs text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-mono"
          >
            <Github className="w-3.5 h-3.5" />
            <span className="hidden sm:inline">Source</span>
            <ExternalLink className="w-3 h-3" />
          </a>
        </div>
      </div>

      <div className="max-w-6xl mx-auto px-6 py-12 lg:grid lg:grid-cols-[200px_1fr] lg:gap-12">
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
              <div className="text-4xl leading-none shrink-0" aria-hidden>{item.icon}</div>
            )}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 mb-2">
                <h1 className="text-2xl md:text-3xl font-black text-slate-900 dark:text-white tracking-tight truncate">
                  {item?.name || id}
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
          {commitQuery.data?.date && (
            <div
              className="text-[11px] font-mono text-gray-400 dark:text-gray-600"
              title={commitQuery.data.message ? `${commitQuery.data.message} — ${commitQuery.data.date}` : commitQuery.data.date}
            >
              {t.registry?.lastUpdated || 'Updated'} {relTime(commitQuery.data.date)}
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
              href={`http://127.0.0.1:4545/${category === 'hands' ? 'hands' : category === 'agents' ? 'agents' : 'skills'}`}
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
            <CopyButton text={rawQuery.data} label={t.registry?.copy || 'Copy'} />
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
            <code>{highlightToml(rawQuery.data)}</code>
          </pre>
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
                      {rel.icon && <span className="text-lg leading-none shrink-0" aria-hidden>{rel.icon}</span>}
                      <h3 className="text-sm font-bold text-slate-900 dark:text-white truncate">{rel.name}</h3>
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

        {/* Back to category */}
        <div className="mt-10 pt-6 border-t border-black/10 dark:border-white/5">
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
