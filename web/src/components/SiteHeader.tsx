import { useEffect, useState } from 'react'
import {
  ChevronDown, ExternalLink, Globe, Menu, Moon,
  Search, Sun, Sparkles, X, Github,
} from 'lucide-react'
import { languages, translations } from '../i18n'
import type { Translation } from '../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

export interface Crumb {
  label: string
  href?: string
}

interface SiteHeaderProps {
  onOpenSearch?: () => void
  // When present, renders subpage layout: small logo + crumbs instead of
  // logo + "LibreFang" brand. Omitted on the homepage.
  crumbs?: Crumb[]
  // Optional "view source" link, e.g. the GitHub file URL of the current
  // registry item. Replaces the generic GitHub button on subpages.
  sourceUrl?: string
  // Fire GA-ish click events. Optional so non-homepage callers don't have
  // to wire gtag through.
  onTrackEvent?: (action: string, label: string) => void
}

// Site-wide header component. One implementation used by the homepage AND
// every subpage, so the right cluster (Features dropdown, search, theme,
// language, Install/Downloads/Docs) is identical. The only thing that
// varies is the left side:
//   * Homepage — big logo + brand text
//   * Subpage — small logo + breadcrumb nav derived from the current route
export default function SiteHeader({ onOpenSearch, crumbs, sourceUrl, onTrackEvent }: SiteHeaderProps) {
  const lang = useAppStore((s) => s.lang)
  const switchLang = useAppStore((s) => s.switchLang)
  const theme = useAppStore((s) => s.theme)
  const toggleTheme = useAppStore((s) => s.toggleTheme)
  const t: Translation = translations[lang] || translations['en']!
  const isSubpage = !!crumbs && crumbs.length > 0
  const [open, setOpen] = useState(false)
  const [langOpen, setLangOpen] = useState(false)
  const [featuresOpen, setFeaturesOpen] = useState(false)
  const [scrolled, setScrolled] = useState(false)
  const [activeSection, setActiveSection] = useState('')
  const currentLangName = languages.find(l => l.code === lang)?.name || 'English'

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  // Scroll-spy only when we have real homepage sections on the page.
  useEffect(() => {
    if (isSubpage) return
    const sections = document.querySelectorAll('section[id]')
    if (sections.length === 0) return
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) setActiveSection(entry.target.id)
        }
      },
      { threshold: 0.3, rootMargin: '-80px 0px -50% 0px' }
    )
    sections.forEach(s => observer.observe(s))
    return () => observer.disconnect()
  }, [isSubpage])

  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { setOpen(false); setLangOpen(false); setFeaturesOpen(false) }
    }
    const handleClickOutside = (e: MouseEvent) => {
      if (langOpen && !(e.target as HTMLElement).closest('[data-lang-menu]')) setLangOpen(false)
      if (featuresOpen && !(e.target as HTMLElement).closest('[data-features-menu]')) setFeaturesOpen(false)
    }
    document.addEventListener('keydown', handleEscape)
    document.addEventListener('click', handleClickOutside)
    return () => {
      document.removeEventListener('keydown', handleEscape)
      document.removeEventListener('click', handleClickOutside)
    }
  }, [langOpen, featuresOpen])

  const langPrefix = lang === 'en' ? '' : `/${lang}`
  const homeHref = lang === 'en' ? '/' : `/${lang}/`

  interface NavLink { label: string; href: string; external?: boolean; highlight?: boolean }

  const rc = t.registry?.categories
  const featureLinks: NavLink[] = [
    { label: rc?.hands.title     || 'Hands',        href: `${langPrefix}/hands` },
    { label: rc?.agents.title    || 'Agents',       href: `${langPrefix}/agents` },
    { label: rc?.skills.title    || 'Skills',       href: `${langPrefix}/skills`, highlight: true },
    { label: rc?.mcp.title       || 'MCP Servers',  href: `${langPrefix}/mcp` },
    { label: rc?.plugins.title   || 'Plugins',      href: `${langPrefix}/plugins` },
    { label: rc?.providers.title || 'Providers',    href: `${langPrefix}/providers` },
    { label: rc?.workflows.title || 'Workflows',    href: `${langPrefix}/workflows` },
    { label: rc?.channels.title  || 'Channels',     href: `${langPrefix}/channels` },
  ]
  // Homepage on-page anchors. From a subpage these are cross-page navs.
  const anchorLinks: NavLink[] = [
    { label: t.nav.architecture, href: `${homeHref}#architecture` },
    { label: t.nav.workflows || t.workflows?.label || 'Workflows', href: `${homeHref}#workflows` },
    { label: t.nav.performance, href: `${homeHref}#performance` },
  ]
  // Flat nav entries. On the homepage these can be in-page anchors; on
  // subpages we rewrite them to cross-page navs with hash.
  const flatLinks: NavLink[] = [
    { label: t.nav.install, href: isSubpage ? `${homeHref}#install` : '#install' },
    { label: t.nav.downloads || 'Downloads', href: isSubpage ? `${homeHref}#downloads` : '#downloads' },
    { label: t.nav.docs, href: 'https://docs.librefang.ai', external: true },
  ]
  const featureActiveIds = ['architecture', 'hands', 'workflows', 'performance', 'evolution']
  const isFeatureActive = featureActiveIds.includes(activeSection)

  const headerClass = cn(
    'fixed top-0 left-0 right-0 z-50 transition-all duration-300',
    (scrolled || isSubpage) && 'bg-surface/90 backdrop-blur-md border-b border-black/10 dark:border-white/5'
  )

  return (
    <nav className={headerClass}>
      <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between gap-4">
        {/* Left: homepage → logo + brand; subpage → small logo + crumbs */}
        {isSubpage ? (
          <nav className="flex items-center gap-2 min-w-0" aria-label="Breadcrumb">
            <a href={homeHref} className="flex items-center gap-2 shrink-0">
              <img src="/logo.png" alt="LibreFang" width="24" height="24" decoding="async" className="w-6 h-6 rounded" />
            </a>
            <span className="text-gray-300 dark:text-gray-700 text-sm">/</span>
            <div className="flex items-center gap-1.5 text-sm text-gray-500 min-w-0 overflow-hidden">
              {crumbs!.map((c, i) => {
                const isLast = i === crumbs!.length - 1
                return (
                  <span key={i} className="flex items-center gap-1.5 min-w-0">
                    {i > 0 && <span className="text-gray-300 dark:text-gray-700 shrink-0">/</span>}
                    {isLast || !c.href ? (
                      <span className={cn('truncate', isLast ? 'text-slate-900 dark:text-white font-semibold' : '')}>{c.label}</span>
                    ) : (
                      <a href={c.href} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors truncate">{c.label}</a>
                    )}
                  </span>
                )
              })}
            </div>
          </nav>
        ) : (
          <a href="/" className="flex items-center gap-2.5">
            <img src="/logo.png" alt="LibreFang" width="32" height="32" decoding="async" fetchPriority="high" className="w-8 h-8 rounded" />
            <span className="font-bold text-slate-900 dark:text-white tracking-tight">LibreFang</span>
          </a>
        )}

        <div className="hidden md:flex items-center gap-1">
          {/* Features dropdown with two groups: Registry pages + Learn More anchors */}
          <div className="relative" data-features-menu>
            <button
              className={cn(
                'flex items-center gap-1 px-3 py-1.5 text-sm transition-colors font-medium',
                isFeatureActive || featuresOpen ? 'text-cyan-600 dark:text-cyan-400' : 'text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400'
              )}
              onClick={() => setFeaturesOpen(!featuresOpen)}
              aria-label={t.nav.features || 'Features'}
              aria-expanded={featuresOpen}
            >
              {t.nav.features || 'Features'}
              <ChevronDown className={cn('w-3 h-3 transition-transform', featuresOpen && 'rotate-180')} />
            </button>
            {featuresOpen && (
              <div className="absolute left-0 mt-2 w-64 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50 py-1">
                <div className="px-4 pt-2 pb-1 text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest">
                  {t.nav.registry || 'Registry'}
                </div>
                {featureLinks.map(link => (
                  <a
                    key={link.label}
                    href={link.href}
                    onClick={() => { setFeaturesOpen(false); onTrackEvent?.('click', `nav_feature_${link.href}`) }}
                    className={cn(
                      'flex items-center justify-between px-4 py-2 text-sm transition-colors',
                      link.highlight ? 'text-amber-600 dark:text-amber-300 hover:bg-amber-500/10' : 'text-gray-700 dark:text-gray-300 hover:text-cyan-600 dark:hover:text-cyan-400 hover:bg-black/5 dark:hover:bg-white/5'
                    )}
                  >
                    <span>{link.label}</span>
                    {link.highlight && <Sparkles className="w-3.5 h-3.5" />}
                  </a>
                ))}
                <div className="border-t border-black/10 dark:border-white/10 mt-2 pt-2" />
                <div className="px-4 pb-1 text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest">
                  {t.nav.learnMore || 'Learn More'}
                </div>
                {anchorLinks.map(link => (
                  <a
                    key={link.label}
                    href={link.href}
                    onClick={(e) => {
                      // Same-page anchor jump if on homepage; cross-page nav otherwise.
                      if (!isSubpage) {
                        const hash = link.href.split('#')[1]
                        if (hash) {
                          e.preventDefault()
                          const el = document.getElementById(hash)
                          if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
                        }
                      }
                      setFeaturesOpen(false)
                    }}
                    className="flex items-center justify-between px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:text-cyan-600 dark:hover:text-cyan-400 hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
                  >
                    <span>{link.label}</span>
                  </a>
                ))}
              </div>
            )}
          </div>

          {flatLinks.map(link => (
            <a
              key={link.label}
              href={link.href}
              target={link.external ? '_blank' : undefined}
              rel={link.external ? 'noopener noreferrer' : undefined}
              aria-current={activeSection === link.href.replace('#', '') ? 'page' : undefined}
              onClick={(e) => {
                // Smooth-scroll for the homepage flat anchors.
                if (!isSubpage && link.href.startsWith('#')) {
                  e.preventDefault()
                  const el = document.querySelector(link.href)
                  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
                }
              }}
              className={cn(
                'px-3 py-1.5 text-sm transition-colors font-medium flex items-center gap-1',
                activeSection === link.href.replace('#', '') ? 'text-cyan-600 dark:text-cyan-400' : 'text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400'
              )}
            >
              {link.label}
              {link.external && <ExternalLink className="w-3 h-3" />}
            </a>
          ))}

          {/* Language switcher */}
          <div className="relative ml-2" data-lang-menu>
            <button
              className="flex items-center gap-1.5 px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium"
              onClick={() => setLangOpen(!langOpen)}
              aria-label="Switch language"
              aria-expanded={langOpen}
            >
              <Globe className="w-3.5 h-3.5" />
              <span className="hidden lg:inline">{currentLangName}</span>
              <ChevronDown className={cn('w-3 h-3 transition-transform', langOpen && 'rotate-180')} />
            </button>
            {langOpen && (
              <div className="absolute right-0 mt-2 w-36 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50">
                {languages.map(l => (
                  <button
                    key={l.code}
                    onClick={() => { switchLang(l.code); setLangOpen(false) }}
                    className={cn('block w-full text-left px-4 py-2.5 text-sm transition-colors', l.code === lang ? 'text-cyan-600 dark:text-cyan-400 bg-cyan-500/5' : 'text-gray-600 dark:text-gray-400 hover:text-slate-900 dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/5')}
                  >
                    {l.name}
                  </button>
                ))}
              </div>
            )}
          </div>

          {onOpenSearch && (
            <button
              onClick={onOpenSearch}
              className="ml-1 flex items-center gap-1.5 px-2 py-1 text-xs text-gray-500 dark:text-gray-400 border border-black/10 dark:border-white/10 rounded hover:text-cyan-600 dark:hover:text-cyan-400 hover:border-cyan-500/30 transition-colors"
              aria-label={t.search?.title || 'Search'}
            >
              <Search className="w-3.5 h-3.5" />
              <kbd className="font-mono text-[10px] px-1 py-0.5 bg-surface-200 rounded">⌘K</kbd>
            </button>
          )}

          <button
            onClick={toggleTheme}
            className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            aria-label="Toggle theme"
          >
            {theme === 'dark' ? <Sun className="w-4 h-4" /> : <Moon className="w-4 h-4" />}
          </button>

          {sourceUrl ? (
            <a
              href={sourceUrl}
              target="_blank"
              rel="noopener noreferrer"
              aria-label="View source on GitHub"
              className="ml-3 flex items-center gap-1 px-3 py-1.5 text-sm font-semibold text-cyan-600 dark:text-cyan-400 border border-cyan-500/30 rounded hover:bg-cyan-500/10 transition-all"
            >
              <Github className="w-3.5 h-3.5" />
              <span className="hidden lg:inline">Source</span>
              <ExternalLink className="w-3 h-3" />
            </a>
          ) : (
            <a
              href="https://github.com/librefang/librefang"
              target="_blank"
              rel="noopener noreferrer"
              className="ml-3 px-4 py-1.5 text-sm font-semibold text-cyan-600 dark:text-cyan-400 border border-cyan-500/30 rounded hover:bg-cyan-500/10 transition-all"
            >
              GitHub
            </a>
          )}
        </div>

        {/* Mobile */}
        <div className="flex md:hidden items-center gap-1">
          {onOpenSearch && (
            <button onClick={onOpenSearch} aria-label={t.search?.title || 'Search'} className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">
              <Search className="w-4 h-4" />
            </button>
          )}
          <button onClick={toggleTheme} className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors" aria-label="Toggle theme">
            {theme === 'dark' ? <Sun className="w-4 h-4" /> : <Moon className="w-4 h-4" />}
          </button>
          <div className="relative" data-lang-menu>
            <button className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors" onClick={() => setLangOpen(!langOpen)} aria-label="Switch language">
              <Globe className="w-4 h-4" />
            </button>
            {langOpen && (
              <div className="absolute right-0 mt-2 w-36 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50">
                {languages.map(l => (
                  <button key={l.code} onClick={() => { switchLang(l.code); setLangOpen(false) }} className={cn('block w-full text-left px-4 py-2.5 text-sm transition-colors', l.code === lang ? 'text-cyan-600 dark:text-cyan-400 bg-cyan-500/5' : 'text-gray-600 dark:text-gray-400 hover:text-slate-900 dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/5')}>{l.name}</button>
                ))}
              </div>
            )}
          </div>
          <button className="p-2 text-gray-600 dark:text-gray-400" onClick={() => setOpen(!open)} aria-label="Toggle menu">
            {open ? <X className="w-5 h-5" /> : <Menu className="w-5 h-5" />}
          </button>
        </div>
      </div>

      {open && (
        <div className="md:hidden bg-surface-100 border-t border-black/10 dark:border-white/5 px-6 py-4 space-y-1">
          <div className="pb-1">
            <div className="text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest py-1.5">
              {t.nav.registry || 'Registry'}
            </div>
            {featureLinks.map(link => (
              <a
                key={link.label}
                href={link.href}
                onClick={() => setOpen(false)}
                className={cn(
                  'flex items-center justify-between py-2 pl-3 text-sm transition-colors font-medium',
                  link.highlight ? 'text-amber-600 dark:text-amber-300' : 'text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400'
                )}
              >
                <span>{link.label}</span>
                {link.highlight && <Sparkles className="w-3.5 h-3.5" />}
              </a>
            ))}
            <div className="text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest py-1.5 mt-2">
              {t.nav.learnMore || 'Learn More'}
            </div>
            {anchorLinks.map(link => (
              <a
                key={link.label}
                href={link.href}
                onClick={() => setOpen(false)}
                className="block py-2 pl-3 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium"
              >
                {link.label}
              </a>
            ))}
          </div>
          {flatLinks.map(link => (
            <a
              key={link.label}
              href={link.href}
              onClick={(e) => {
                if (!isSubpage && link.href.startsWith('#')) {
                  e.preventDefault()
                  const el = document.querySelector(link.href)
                  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
                }
                setOpen(false)
              }}
              className="block py-2.5 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium"
            >
              {link.label}
            </a>
          ))}
          <div className="pt-2 border-t border-black/10 dark:border-white/5 mt-2 flex flex-wrap gap-2">
            {languages.map(l => (
              <button
                key={l.code}
                onClick={() => { switchLang(l.code); setOpen(false) }}
                className={cn('px-3 py-1.5 text-xs rounded', l.code === lang ? 'text-cyan-600 dark:text-cyan-400 bg-cyan-500/10' : 'text-gray-500 hover:text-slate-900 dark:hover:text-white')}
              >
                {l.name}
              </button>
            ))}
          </div>
        </div>
      )}
    </nav>
  )
}
