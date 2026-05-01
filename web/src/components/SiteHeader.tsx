import { useEffect, useState } from 'react'
import {
  ChevronDown, ExternalLink, Globe, Menu, Moon,
  Search, Sun, X,
} from 'lucide-react'
import { Github } from './BrandIcons'
import { languages, translations } from '../i18n'
import type { Translation } from '../i18n'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

interface SiteHeaderProps {
  onOpenSearch?: () => void
  // True on non-homepage routes so we rewrite flat links to cross-page
  // navs and turn off scroll-spy. The *visual* layout doesn't change —
  // header is identical everywhere; breadcrumbs (if any) belong below it.
  isSubpage?: boolean
  // Optional "view source" link, e.g. the GitHub file URL of the current
  // registry item. Replaces the generic GitHub button on subpages.
  sourceUrl?: string
  // Fire GA-ish click events. Optional so non-homepage callers don't have
  // to wire gtag through.
  onTrackEvent?: (action: string, label: string) => void
}

// Site-wide header. Byte-for-byte identical on every page: same logo,
// same "LibreFang" brand text, same right cluster. `isSubpage` only
// tweaks link targets (cross-page anchors) and disables scroll-spy —
// nothing visual. Breadcrumbs live below the header in page content.
export default function SiteHeader({ onOpenSearch, isSubpage = false, sourceUrl, onTrackEvent }: SiteHeaderProps) {
  const lang = useAppStore((s) => s.lang)
  const switchLang = useAppStore((s) => s.switchLang)
  const theme = useAppStore((s) => s.theme)
  const toggleTheme = useAppStore((s) => s.toggleTheme)
  const t: Translation = translations[lang] || translations['en']!
  const [open, setOpen] = useState(false)
  const [langOpen, setLangOpen] = useState(false)
  const [featuresOpen, setFeaturesOpen] = useState(false)
  const [learnOpen, setLearnOpen] = useState(false)
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
      if (e.key === 'Escape') { setOpen(false); setLangOpen(false); setFeaturesOpen(false); setLearnOpen(false) }
    }
    const handleClickOutside = (e: MouseEvent) => {
      if (langOpen && !(e.target as HTMLElement).closest('[data-lang-menu]')) setLangOpen(false)
      if (featuresOpen && !(e.target as HTMLElement).closest('[data-features-menu]')) setFeaturesOpen(false)
      if (learnOpen && !(e.target as HTMLElement).closest('[data-learn-menu]')) setLearnOpen(false)
    }
    document.addEventListener('keydown', handleEscape)
    document.addEventListener('click', handleClickOutside)
    return () => {
      document.removeEventListener('keydown', handleEscape)
      document.removeEventListener('click', handleClickOutside)
    }
  }, [langOpen, featuresOpen, learnOpen])

  const langPrefix = lang === 'en' ? '' : `/${lang}`
  const homeHref = lang === 'en' ? '/' : `/${lang}/`

  interface NavLink { label: string; href: string; external?: boolean }

  const rc = t.registry?.categories
  // All 8 marketplace categories — no per-item highlight, they're peers.
  const featureLinks: NavLink[] = [
    { label: rc?.hands.title     || 'Hands',        href: `${langPrefix}/hands` },
    { label: rc?.agents.title    || 'Agents',       href: `${langPrefix}/agents` },
    { label: rc?.skills.title    || 'Skills',       href: `${langPrefix}/skills` },
    { label: rc?.mcp.title       || 'MCP Servers',  href: `${langPrefix}/mcp` },
    { label: rc?.plugins.title   || 'Plugins',      href: `${langPrefix}/plugins` },
    { label: rc?.providers.title || 'Providers',    href: `${langPrefix}/providers` },
    { label: rc?.workflows.title || 'Workflows',    href: `${langPrefix}/workflows` },
    { label: rc?.channels.title  || 'Channels',     href: `${langPrefix}/channels` },
  ]
  // Features dropdown: one anchor per homepage module, in scroll order.
  // Cross-page nav when viewed from a subpage; smooth-scroll on the
  // homepage itself. Hands / Workflows appear here too (even though the
  // Marketplace dropdown has items of the same name) because those are
  // the homepage teaser sections — different destination from the
  // /hands and /workflows catalog pages.
  const anchor = (id: string) => (isSubpage ? `${homeHref}#${id}` : `#${id}`)
  const anchorLinks: NavLink[] = [
    { label: t.nav.architecture,                                   href: anchor('architecture') },
    { label: t.nav.hands,                                          href: anchor('hands') },
    { label: t.nav.workflows || t.workflows?.label || 'Workflows', href: anchor('workflows') },
    { label: t.nav.evolution || t.evolution?.label || 'Evolution', href: anchor('evolution') },
    { label: t.nav.performance,                                    href: anchor('performance') },
    { label: t.nav.install,                                        href: anchor('install') },
    { label: t.nav.downloads  || 'Downloads',                      href: anchor('downloads') },
    { label: t.faq?.label     || 'FAQ',                            href: anchor('faq') },
    { label: t.githubStats?.label || 'Community',                  href: anchor('community') },
  ]
  // Only external "flat" link that remains.
  const flatLinks: NavLink[] = [
    { label: t.nav.docs, href: 'https://docs.librefang.ai', external: true },
  ]
  const featureActiveIds = ['hands', 'agents', 'skills', 'mcp', 'plugins', 'providers', 'workflows', 'channels']
  const isFeatureActive = featureActiveIds.includes(activeSection)
  const learnActiveIds = ['architecture', 'hands', 'evolution', 'workflows', 'performance', 'downloads', 'install', 'faq', 'community']
  const isLearnActive = learnActiveIds.includes(activeSection)

  const headerClass = cn(
    'fixed top-0 left-0 right-0 z-50 transition-all duration-300',
    (scrolled || isSubpage) && 'bg-surface/90 backdrop-blur-md border-b border-black/10 dark:border-white/5'
  )

  return (
    <nav className={headerClass}>
      <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between gap-4">
        <a href={homeHref} className="flex items-center gap-2.5">
          <img src="/logo.png" alt="" width="32" height="32" decoding="async" fetchPriority="high" className="w-8 h-8 rounded" />
          <span className="font-bold text-slate-900 dark:text-white tracking-tight">LibreFang</span>
        </a>

        <div className="hidden md:flex items-center gap-1">
          {/* Features dropdown — architecture, workflows, performance, install, downloads */}
          <div className="relative" data-learn-menu>
            <button
              className={cn(
                'flex items-center gap-1 px-3 py-1.5 text-sm transition-colors font-medium',
                isLearnActive || learnOpen ? 'text-cyan-600 dark:text-cyan-400' : 'text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400'
              )}
              onClick={() => { setLearnOpen(!learnOpen); setFeaturesOpen(false) }}
              aria-label={t.nav.learnMore || 'Features'}
              aria-expanded={learnOpen}
            >
              {t.nav.learnMore || 'Features'}
              <ChevronDown className={cn('w-3 h-3 transition-transform', learnOpen && 'rotate-180')} />
            </button>
            {learnOpen && (
              <div className="absolute left-0 mt-2 w-56 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50 py-1">
                {anchorLinks.map(link => (
                  <a
                    key={link.label}
                    href={link.href}
                    onClick={(e) => {
                      if (!isSubpage) {
                        const hash = link.href.split('#')[1]
                        if (hash) {
                          e.preventDefault()
                          const el = document.getElementById(hash)
                          if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
                        }
                      }
                      setLearnOpen(false)
                    }}
                    className="flex items-center justify-between px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:text-cyan-600 dark:hover:text-cyan-400 hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
                  >
                    <span>{link.label}</span>
                  </a>
                ))}
              </div>
            )}
          </div>

          {/* Marketplace dropdown — 8 registry categories only */}
          <div className="relative" data-features-menu>
            <button
              className={cn(
                'flex items-center gap-1 px-3 py-1.5 text-sm transition-colors font-medium',
                isFeatureActive || featuresOpen ? 'text-cyan-600 dark:text-cyan-400' : 'text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400'
              )}
              onClick={() => { setFeaturesOpen(!featuresOpen); setLearnOpen(false) }}
              aria-label={t.nav.features || 'Marketplace'}
              aria-expanded={featuresOpen}
            >
              {t.nav.features || 'Marketplace'}
              <ChevronDown className={cn('w-3 h-3 transition-transform', featuresOpen && 'rotate-180')} />
            </button>
            {featuresOpen && (
              <div className="absolute left-0 mt-2 w-56 bg-surface-200 border border-black/10 dark:border-white/10 rounded shadow-xl z-50 py-1">
                {featureLinks.map(link => (
                  <a
                    key={link.label}
                    href={link.href}
                    onClick={() => { setFeaturesOpen(false); onTrackEvent?.('click', `nav_feature_${link.href}`) }}
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
              aria-label={`Switch language (${currentLangName})`}
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
              aria-label={`${t.search?.title || 'Search'} ⌘K`}
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
              {t.nav.learnMore || 'Features'}
            </div>
            {anchorLinks.map(link => (
              <a
                key={link.label}
                href={link.href}
                onClick={(e) => {
                  if (!isSubpage) {
                    const hash = link.href.split('#')[1]
                    if (hash) {
                      e.preventDefault()
                      const el = document.getElementById(hash)
                      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
                    }
                  }
                  setOpen(false)
                }}
                className="block py-2 pl-3 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium"
              >
                {link.label}
              </a>
            ))}
            <div className="text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-widest py-1.5 mt-2">
              {t.nav.features || 'Marketplace'}
            </div>
            {featureLinks.map(link => (
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
              target={link.external ? '_blank' : undefined}
              rel={link.external ? 'noopener noreferrer' : undefined}
              onClick={() => setOpen(false)}
              className="block py-2.5 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium flex items-center gap-1"
            >
              {link.label}
              {link.external && <ExternalLink className="w-3 h-3" />}
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
