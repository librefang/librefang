import { useEffect, useRef, useState } from 'react'
import { ArrowLeft, ChevronDown, ExternalLink, Github, Globe, Moon, Search, Sun, Sparkles } from 'lucide-react'
import { languages, translations } from '../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

interface BreadcrumbCrumb {
  label: string
  href?: string
}

interface SubpageHeaderProps {
  crumbs: BreadcrumbCrumb[]
  sourceUrl?: string   // Optional "Source" link — e.g. the GitHub file URL of the item
  onOpenSearch?: () => void
}

// Right-cluster widgets (search / theme / lang / GitHub) extracted so every
// subpage can share the exact same set of controls the homepage nav has.
// Crumbs land on the left, source link + right-cluster on the right.
export default function SubpageHeader({ crumbs, sourceUrl, onOpenSearch }: SubpageHeaderProps) {
  const lang = useAppStore(s => s.lang)
  const switchLang = useAppStore(s => s.switchLang)
  const theme = useAppStore(s => s.theme)
  const toggleTheme = useAppStore(s => s.toggleTheme)
  const t = translations[lang] || translations['en']!
  const [langOpen, setLangOpen] = useState(false)
  const [featuresOpen, setFeaturesOpen] = useState(false)
  const langMenuRef = useRef<HTMLDivElement>(null)
  const featuresRef = useRef<HTMLDivElement>(null)
  const currentLangName = languages.find(l => l.code === lang)?.name || 'English'
  const langPrefix = lang === 'en' ? '' : `/${lang}`
  const homeHref = lang === 'en' ? '/' : `/${lang}/`

  useEffect(() => {
    if (!langOpen && !featuresOpen) return
    const onDoc = (e: MouseEvent) => {
      if (langOpen && !langMenuRef.current?.contains(e.target as Node)) setLangOpen(false)
      if (featuresOpen && !featuresRef.current?.contains(e.target as Node)) setFeaturesOpen(false)
    }
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { setLangOpen(false); setFeaturesOpen(false) } }
    document.addEventListener('mousedown', onDoc)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDoc)
      document.removeEventListener('keydown', onKey)
    }
  }, [langOpen, featuresOpen])

  const rc = t.registry?.categories
  const featureLinks = [
    { label: rc?.hands.title || 'Hands', href: `${langPrefix}/hands` },
    { label: rc?.agents.title || 'Agents', href: `${langPrefix}/agents` },
    { label: rc?.skills.title || 'Skills', href: `${langPrefix}/skills`, highlight: true },
    { label: rc?.mcp.title || 'MCP', href: `${langPrefix}/mcp` },
    { label: rc?.plugins.title || 'Plugins', href: `${langPrefix}/plugins` },
    { label: rc?.providers.title || 'Providers', href: `${langPrefix}/providers` },
    { label: rc?.workflows.title || 'Workflows', href: `${langPrefix}/workflows` },
    { label: rc?.channels.title || 'Channels', href: `${langPrefix}/channels` },
  ]

  // Homepage on-page sections. From a subpage these are cross-page navs that
  // jump to the landing page + scroll-to hash.
  const anchorLinks = [
    { label: t.nav.architecture, href: `${homeHref}#architecture` },
    { label: t.nav.workflows || t.workflows?.label || 'Workflows', href: `${homeHref}#workflows` },
    { label: t.nav.performance, href: `${homeHref}#performance` },
  ]

  // Flat right-nav entries — match the homepage Nav so the cluster is
  // consistent whether you're on / or /skills.
  const flatLinks = [
    { label: t.nav.install, href: `${homeHref}#install` },
    { label: t.nav.downloads || 'Downloads', href: `${homeHref}#downloads` },
    { label: t.nav.docs, href: 'https://docs.librefang.ai', external: true },
  ]

  return (
    <div className="border-b border-black/10 dark:border-white/5 bg-surface-100 sticky top-0 z-40 backdrop-blur-md bg-surface-100/90">
      <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between gap-4">
        {/* Left: logo + breadcrumbs */}
        <nav className="flex items-center gap-2 min-w-0" aria-label="Breadcrumb">
          <a href={homeHref} className="flex items-center gap-2 shrink-0">
            <img src="/logo.png" alt="LibreFang" width="24" height="24" className="w-6 h-6 rounded" />
          </a>
          <span className="text-gray-300 dark:text-gray-700 text-sm">/</span>
          <div className="flex items-center gap-1.5 text-sm text-gray-500 min-w-0 overflow-hidden">
            {crumbs.map((c, i) => {
              const isLast = i === crumbs.length - 1
              return (
                <span key={i} className="flex items-center gap-1.5 min-w-0">
                  {i > 0 && <span className="text-gray-300 dark:text-gray-700 shrink-0">/</span>}
                  {isLast || !c.href ? (
                    <span className={cn('truncate', isLast ? 'text-slate-900 dark:text-white font-semibold' : '')}>{c.label}</span>
                  ) : (
                    <a href={c.href} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors truncate">
                      {c.label}
                    </a>
                  )}
                </span>
              )
            })}
          </div>
        </nav>

        {/* Right cluster: Features dropdown + search + theme + lang + source */}
        <div className="flex items-center gap-1 shrink-0">
          {/* Features dropdown — same set as homepage nav so visitors can
              pivot between categories without bouncing back to /. */}
          <div ref={featuresRef} className="relative hidden md:block">
            <button
              onClick={() => setFeaturesOpen(v => !v)}
              aria-expanded={featuresOpen}
              className="flex items-center gap-1 px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium"
            >
              {t.nav.features || 'Features'}
              <ChevronDown className={cn('w-3 h-3 transition-transform', featuresOpen && 'rotate-180')} />
            </button>
            {featuresOpen && (
              <div className="absolute right-0 mt-2 w-64 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50 py-1">
                <div className="px-4 pt-2 pb-1 text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest">
                  {t.nav.registry || 'Registry'}
                </div>
                {featureLinks.map(l => (
                  <a
                    key={l.label}
                    href={l.href}
                    className={cn(
                      'flex items-center justify-between px-4 py-2 text-sm transition-colors',
                      l.highlight ? 'text-amber-600 dark:text-amber-300 hover:bg-amber-500/10' : 'text-gray-700 dark:text-gray-300 hover:text-cyan-600 dark:hover:text-cyan-400 hover:bg-black/5 dark:hover:bg-white/5'
                    )}
                  >
                    <span>{l.label}</span>
                    {l.highlight && <Sparkles className="w-3.5 h-3.5" />}
                  </a>
                ))}
                <div className="border-t border-black/10 dark:border-white/10 mt-2 pt-2" />
                <div className="px-4 pb-1 text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest">
                  {t.nav.learnMore || 'Learn More'}
                </div>
                {anchorLinks.map(l => (
                  <a
                    key={l.label}
                    href={l.href}
                    className="flex items-center justify-between px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:text-cyan-600 dark:hover:text-cyan-400 hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
                  >
                    <span>{l.label}</span>
                  </a>
                ))}
              </div>
            )}
          </div>

          {/* Flat links — Install / Downloads / Docs. Install + Downloads
              are on-page anchors on the homepage, so from a subpage they
              are cross-page navigation that lands with a scroll-to hash. */}
          <div className="hidden lg:flex items-center gap-1">
            {flatLinks.map(l => (
              <a
                key={l.label}
                href={l.href}
                target={l.external ? '_blank' : undefined}
                rel={l.external ? 'noopener noreferrer' : undefined}
                className="px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium flex items-center gap-1"
              >
                {l.label}
                {l.external && <ExternalLink className="w-3 h-3" />}
              </a>
            ))}
          </div>

          {onOpenSearch && (
            <button
              onClick={onOpenSearch}
              aria-label={t.search?.title || 'Search'}
              className="flex items-center gap-1.5 px-2 py-1 text-xs text-gray-500 dark:text-gray-400 border border-black/10 dark:border-white/10 rounded hover:text-cyan-600 dark:hover:text-cyan-400 hover:border-cyan-500/30 transition-colors"
            >
              <Search className="w-3.5 h-3.5" />
              <kbd className="hidden sm:inline font-mono text-[10px] px-1 py-0.5 bg-surface-200 rounded">⌘K</kbd>
            </button>
          )}

          <button onClick={toggleTheme} aria-label="Toggle theme" className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">
            {theme === 'dark' ? <Sun className="w-4 h-4" /> : <Moon className="w-4 h-4" />}
          </button>

          <div ref={langMenuRef} className="relative">
            <button
              onClick={() => setLangOpen(v => !v)}
              aria-label="Switch language"
              aria-expanded={langOpen}
              className="flex items-center gap-1 px-2 py-1.5 text-xs text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            >
              <Globe className="w-3.5 h-3.5" />
              <span className="hidden sm:inline">{currentLangName}</span>
              <ChevronDown className={cn('w-3 h-3 transition-transform', langOpen && 'rotate-180')} />
            </button>
            {langOpen && (
              <div className="absolute right-0 mt-2 w-36 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50">
                {languages.map(l => (
                  <button
                    key={l.code}
                    onClick={() => { switchLang(l.code); setLangOpen(false) }}
                    className={cn('block w-full text-left px-4 py-2 text-sm transition-colors', l.code === lang ? 'text-cyan-600 dark:text-cyan-400 bg-cyan-500/5' : 'text-gray-600 dark:text-gray-400 hover:text-slate-900 dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/5')}
                  >
                    {l.name}
                  </button>
                ))}
              </div>
            )}
          </div>

          {sourceUrl ? (
            <a
              href={sourceUrl}
              target="_blank"
              rel="noopener noreferrer"
              aria-label="View source on GitHub"
              className="flex items-center gap-1 px-2 py-1 text-xs text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            >
              <Github className="w-3.5 h-3.5" />
              <ExternalLink className="w-3 h-3" />
            </a>
          ) : (
            <a
              href="https://github.com/librefang/librefang"
              target="_blank"
              rel="noopener noreferrer"
              aria-label="GitHub"
              className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            >
              <Github className="w-4 h-4" />
            </a>
          )}
        </div>
      </div>

      {/* Breadcrumbs overflow: on mobile the logo+crumbs row keeps right cluster tight. */}
      {crumbs.length > 0 && (
        <div className="md:hidden max-w-6xl mx-auto px-6 pb-2 flex items-center gap-1.5 text-xs text-gray-500 overflow-x-auto whitespace-nowrap">
          <a href={homeHref} className="flex items-center gap-1 text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors shrink-0">
            <ArrowLeft className="w-3 h-3" />
          </a>
          {crumbs.map((c, i) => (
            <span key={i} className="flex items-center gap-1.5">
              <span className="text-gray-300 dark:text-gray-700">/</span>
              {c.href && i < crumbs.length - 1 ? (
                <a href={c.href} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{c.label}</a>
              ) : (
                <span className="text-slate-900 dark:text-white font-semibold">{c.label}</span>
              )}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}
