import { useState, useEffect, useRef } from 'react'
import { motion } from 'framer-motion'
import {
  Terminal, Cpu, Shield, Zap, Network, ChevronRight, ChevronDown, ExternalLink,
  Copy, Check, Menu, X, Box, Layers, Radio, Eye,
  Scissors, Users, Globe, ArrowRight, Github, Monitor,
  Star, GitFork, CircleDot, GitPullRequest, MessageSquare,
  Sun, Moon
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { translations, languages } from './i18n'
import type { Translation } from './i18n'
import { useRegistry, getLocalizedDesc } from './useRegistry'
import { useAppStore } from './store'
import { cn } from './lib/utils'


// ─── Language detection ───
function getCurrentLang(): string {
  if (typeof window === 'undefined') return 'en'
  const path = window.location.pathname
  if (path.startsWith('/zh-TW')) return 'zh-TW'
  if (path.startsWith('/zh')) return 'zh'
  if (path.startsWith('/de')) return 'de'
  if (path.startsWith('/ja')) return 'ja'
  if (path.startsWith('/ko')) return 'ko'
  if (path.startsWith('/es')) return 'es'
  return 'en'
}

// ─── Typing animation hook ───
function useTyping(texts: string[], speed = 60, pause = 2000): string {
  const [display, setDisplay] = useState('')
  const [idx, setIdx] = useState(0)
  const [charIdx, setCharIdx] = useState(0)
  const [deleting, setDeleting] = useState(false)

  useEffect(() => {
    const current = texts[idx]!
    if (!deleting && charIdx < current.length) {
      const t = setTimeout(() => {
        setDisplay(current.slice(0, charIdx + 1))
        setCharIdx(c => c + 1)
      }, speed)
      return () => clearTimeout(t)
    }
    if (!deleting && charIdx === current.length) {
      const t = setTimeout(() => setDeleting(true), pause)
      return () => clearTimeout(t)
    }
    if (deleting && charIdx > 0) {
      const t = setTimeout(() => {
        setDisplay(current.slice(0, charIdx - 1))
        setCharIdx(c => c - 1)
      }, speed / 2)
      return () => clearTimeout(t)
    }
    if (deleting && charIdx === 0) {
      setDeleting(false)
      setIdx(i => (i + 1) % texts.length)
    }
  }, [charIdx, deleting, idx, texts, speed, pause])

  return display
}

// ─── Framer Motion fade-in ───
interface FadeInProps {
  children: React.ReactNode
  className?: string
  delay?: number
}

function FadeIn({ children, className = '', delay = 0 }: FadeInProps) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 24 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, amount: 0.1 }}
      transition={{ duration: 0.6, delay: delay / 1000, ease: 'easeOut' }}
      className={className}
    >
      {children}
    </motion.div>
  )
}

// ─── Nav ───
interface NavProps {
  t: Translation
  lang: string
  onSwitchLang: (code: string) => void
}

function Nav({ t, lang, onSwitchLang }: NavProps) {
  const [open, setOpen] = useState(false)
  const [langOpen, setLangOpen] = useState(false)
  const [scrolled, setScrolled] = useState(false)
  const [activeSection, setActiveSection] = useState('')
  const theme = useAppStore((s) => s.theme)
  const toggleTheme = useAppStore((s) => s.toggleTheme)
  const currentLangName = languages.find(l => l.code === lang)?.name || 'English'

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  useEffect(() => {
    const sections = document.querySelectorAll('section[id]')
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            setActiveSection(entry.target.id)
          }
        }
      },
      { threshold: 0.3, rootMargin: '-80px 0px -50% 0px' }
    )
    sections.forEach(s => observer.observe(s))
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { setOpen(false); setLangOpen(false) }
    }
    const handleClickOutside = (e: MouseEvent) => {
      if (langOpen && !(e.target as HTMLElement).closest('[data-lang-menu]')) setLangOpen(false)
    }
    document.addEventListener('keydown', handleEscape)
    document.addEventListener('click', handleClickOutside)
    return () => {
      document.removeEventListener('keydown', handleEscape)
      document.removeEventListener('click', handleClickOutside)
    }
  }, [langOpen])

  interface NavLink {
    label: string
    href: string
    external?: boolean
  }

  const links: NavLink[] = [
    { label: t.nav.architecture, href: '#architecture' },
    { label: t.nav.hands, href: '#hands' },
    { label: t.workflows?.label || 'Workflows', href: '#workflows' },
    { label: t.nav.performance, href: '#performance' },
    { label: t.nav.install, href: '#install' },
    { label: t.nav.downloads || 'Downloads', href: '#downloads' },
    { label: t.nav.docs, href: 'https://docs.librefang.ai', external: true },
  ]

  return (
    <nav className={cn('fixed top-0 left-0 right-0 z-50 transition-all duration-300', scrolled && 'bg-surface/90 backdrop-blur-md border-b border-black/10 dark:border-white/5')}>
      <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between">
        <a href="/" className="flex items-center gap-2.5">
          <img src="/logo.png" alt="LibreFang" className="w-8 h-8 rounded" />
          <span className="font-bold text-slate-900 dark:text-white tracking-tight">LibreFang</span>
        </a>

        <div className="hidden md:flex items-center gap-1">
          {links.map(link => (
            <a
              key={link.label}
              href={link.href}
              target={link.external ? '_blank' : undefined}
              rel={link.external ? 'noopener noreferrer' : undefined}
              aria-current={activeSection === link.href.replace('#', '') ? 'page' : undefined}
              onClick={(e) => {
                if (link.href.startsWith('#')) {
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
                    onClick={() => { onSwitchLang(l.code); setLangOpen(false) }}
                    className={cn('block w-full text-left px-4 py-2.5 text-sm transition-colors', l.code === lang ? 'text-cyan-600 dark:text-cyan-400 bg-cyan-500/5' : 'text-gray-600 dark:text-gray-400 hover:text-slate-900 dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/5')}
                  >
                    {l.name}
                  </button>
                ))}
              </div>
            )}
          </div>

          <button
            onClick={toggleTheme}
            className="p-2 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            aria-label="Toggle theme"
          >
            {theme === 'dark' ? <Sun className="w-4 h-4" /> : <Moon className="w-4 h-4" />}
          </button>

          <a
            href="https://github.com/librefang/librefang"
            target="_blank"
            rel="noopener noreferrer"
            className="ml-3 px-4 py-1.5 text-sm font-semibold text-cyan-600 dark:text-cyan-400 border border-cyan-500/30 rounded hover:bg-cyan-500/10 transition-all"
          >
            GitHub
          </a>
        </div>

        <div className="flex md:hidden items-center gap-1">
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
                  <button key={l.code} onClick={() => { onSwitchLang(l.code); setLangOpen(false) }} className={cn('block w-full text-left px-4 py-2.5 text-sm transition-colors', l.code === lang ? 'text-cyan-600 dark:text-cyan-400 bg-cyan-500/5' : 'text-gray-600 dark:text-gray-400 hover:text-slate-900 dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/5')}>{l.name}</button>
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
          {links.map(link => (
            <a
              key={link.label}
              href={link.href}
              onClick={(e) => {
                if (link.href.startsWith('#')) {
                  e.preventDefault()
                  const el = document.querySelector(link.href)
                  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
                }
                setOpen(false)
              }}
              aria-current={activeSection === link.href.replace('#', '') ? 'page' : undefined}
              className="block py-2.5 text-sm text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors font-medium"
            >
              {link.label}
            </a>
          ))}
          <div className="pt-2 border-t border-black/10 dark:border-white/5 mt-2 flex flex-wrap gap-2">
            {languages.map(l => (
              <button
                key={l.code}
                onClick={() => { onSwitchLang(l.code); setOpen(false) }}
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

// ─── Hero ───
interface SectionProps {
  t: Translation
}

function Hero({ t, registry }: SectionProps & { registry?: import('./useRegistry').RegistryData }) {
  const typed = useTyping(t.hero.typing)

  return (
    <header className="relative min-h-screen grid-bg overflow-hidden">
      <div className="absolute top-1/4 left-1/3 -translate-x-1/2 -translate-y-1/2 w-[600px] h-[600px] bg-cyan-500/5 rounded-full blur-[120px] pointer-events-none" />

      <div className="relative z-10 max-w-6xl mx-auto px-6 pt-32 pb-20">
        <div className="grid lg:grid-cols-2 gap-16 items-center">
          {/* Left: text content */}
          <div>
            <FadeIn>
              <div className="inline-flex items-center gap-2 px-3 py-1 rounded border border-cyan-500/20 bg-cyan-500/5 text-xs font-mono text-cyan-600 dark:text-cyan-400 mb-8">
                <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-pulse" />
                v2026.3 &mdash; {t.hero.badge} &mdash; Rust
              </div>
            </FadeIn>

            <FadeIn delay={100}>
              <h1 className="text-4xl sm:text-5xl md:text-6xl lg:text-7xl font-black tracking-tight leading-[0.95] mb-6">
                <span className="text-slate-900 dark:text-white">{t.hero.title1}</span>
                <br />
                <span className="text-cyan-600 dark:text-cyan-400">{t.hero.title2}</span>
              </h1>
            </FadeIn>

            <FadeIn delay={200}>
              <div className="flex items-center gap-2 text-base md:text-lg text-gray-500 font-mono mb-8 min-h-[1.75rem] overflow-hidden">
                <span className="text-cyan-600 dark:text-cyan-500">$</span>
                <span className="text-gray-700 dark:text-gray-300">{typed}</span>
                <span className="w-2 h-4 bg-cyan-400 cursor-blink" />
              </div>
            </FadeIn>

            <FadeIn delay={300}>
              <p className="text-gray-600 dark:text-gray-400 text-base leading-relaxed mb-8">{t.hero.desc}</p>
            </FadeIn>

            <FadeIn delay={400}>
              <div className="flex flex-col sm:flex-row gap-3">
                <a href="#install" onClick={() => trackEvent('click', 'hero_get_started')} className="inline-flex items-center justify-center gap-2 px-6 py-3 bg-cyan-500 hover:bg-cyan-400 text-surface font-bold rounded transition-all hover:shadow-lg hover:shadow-cyan-500/20">
                  {t.hero.getStarted}
                  <ArrowRight className="w-4 h-4" />
                </a>
                <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" onClick={() => trackEvent('click', 'hero_github')} className="inline-flex items-center justify-center gap-2 px-6 py-3 border border-black/10 dark:border-white/10 hover:border-black/20 dark:hover:border-white/20 text-gray-700 dark:text-gray-300 font-semibold rounded transition-all hover:bg-black/5 dark:hover:bg-white/5">
                  <Github className="w-4 h-4" />
                  {t.hero.viewGithub}
                </a>
              </div>
            </FadeIn>

            <FadeIn delay={450}>
              <div className="lg:hidden mt-8 inline-flex items-center gap-3 px-4 py-2 bg-surface-100 border border-black/10 dark:border-white/5 rounded font-mono text-xs">
                <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse" />
                <span className="text-gray-500 dark:text-gray-400">4 agents running</span>
                <span className="text-gray-300 dark:text-gray-600">·</span>
                <span className="text-gray-500 dark:text-gray-400">38MB</span>
              </div>
            </FadeIn>
          </div>

          {/* Right: system preview terminal */}
          <FadeIn delay={300}>
            <div className="hidden lg:block">
              <div className="border border-black/10 dark:border-white/10 bg-surface-100 overflow-hidden glow-cyan">
                <div className="flex items-center gap-2 px-4 py-2.5 bg-surface-200 border-b border-black/10 dark:border-white/5">
                  <div className="flex gap-1.5">
                    <div className="w-2 h-2 rounded-full bg-red-500/40" />
                    <div className="w-2 h-2 rounded-full bg-yellow-500/40" />
                    <div className="w-2 h-2 rounded-full bg-green-500/40" />
                  </div>
                  <span className="text-[10px] font-mono text-gray-600 uppercase tracking-widest ml-2">librefang agent os</span>
                </div>
                <div className="p-5 font-mono text-xs leading-relaxed space-y-3">
                  <div className="text-gray-500">$ librefang status</div>
                  <div className="space-y-1.5">
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">runtime</span><span className="text-cyan-600 dark:text-cyan-400">running</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">uptime</span><span className="text-slate-900 dark:text-white">14d 7h 23m</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">memory</span><span className="text-slate-900 dark:text-white">38MB</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">agents</span><span className="text-slate-900 dark:text-white">4 active</span></div>
                  </div>
                  <div className="border-t border-black/10 dark:border-white/5 pt-3 space-y-1.5">
                    <div className="text-amber-400/70">AGENTS</div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">clip</span><span className="text-cyan-600 dark:text-cyan-400">● idle</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">lead</span><span className="text-green-400">● running</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">collector</span><span className="text-cyan-600 dark:text-cyan-400">● idle</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">researcher</span><span className="text-green-400">● running</span></div>
                  </div>
                  <div className="border-t border-black/10 dark:border-white/5 pt-3 space-y-1.5">
                    <div className="text-amber-400/70">CHANNELS</div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">telegram</span><span className="text-green-400">connected</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">slack</span><span className="text-green-400">connected</span></div>
                    <div className="flex justify-between"><span className="text-gray-600 dark:text-gray-400">discord</span><span className="text-gray-600">standby</span></div>
                  </div>
                </div>
              </div>
            </div>
          </FadeIn>
        </div>

        {/* Stats bar - full width below */}
        <FadeIn delay={500}>
          <div className="mt-16 grid grid-cols-2 md:grid-cols-4 gap-px bg-black/5 dark:bg-white/5 rounded overflow-hidden">
            {([
              { value: '180ms', label: t.stats.coldStart, icon: Zap },
              { value: '40MB', label: t.stats.memory, icon: Cpu },
              { value: String(registry?.handsCount ?? 15), label: t.stats.hands || 'Hands', icon: Box },
              { value: String(registry?.providersCount ?? 50), label: t.stats.providers || 'Providers', icon: Network },
            ] as const).map((stat, i) => (
              <motion.div key={i} initial={{ opacity: 0, y: 10 }} whileInView={{ opacity: 1, y: 0 }} viewport={{ once: true }} transition={{ delay: 0.5 + i * 0.1, duration: 0.4 }}>
                <div className="bg-surface-100 px-6 py-5 flex items-center gap-4">
                  <stat.icon className="w-5 h-5 text-cyan-600/60 dark:text-cyan-500/60 shrink-0" />
                  <div>
                    <div className="text-2xl font-black text-slate-900 dark:text-white font-mono">{stat.value}</div>
                    <div className="text-xs text-gray-500 font-medium uppercase tracking-wider">{stat.label}</div>
                  </div>
                </div>
              </motion.div>
            ))}
          </div>
        </FadeIn>
      </div>
    </header>
  )
}

// ─── Architecture ───
const layerIcons: LucideIcon[] = [Globe, Box, Cpu, Layers, Radio]
const layerColors: string[] = ['text-amber-400', 'text-cyan-600 dark:text-cyan-400', 'text-purple-400', 'text-emerald-400', 'text-rose-400']

// Layer detail titles (not translated, technical terms)
const layerTitles = {
  kernel: ['Agent Lifecycle', 'Workflow Engine', 'Budget Control', 'Scheduler', 'Memory System', 'Skill System', 'MCP + A2A', 'OFP Wire'],
  runtime: ['Tokio Async', 'WASM Sandbox', 'Merkle Audit', 'SSRF Protection', 'Taint Tracking', 'GCRA Rate Limiter', 'Prompt Injection', 'RBAC'],
  hardware: ['Single Binary', 'Linux / macOS / Windows', 'Raspberry Pi', 'Android (Termux)', 'VPS / Cloud', 'Bare Metal', 'Tauri Desktop'],
}

function DetailGrid({ titles, descs }: { titles: string[]; descs: string[] }) {
  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
      {titles.map((title, i) => (
        <div key={title} className="px-3 py-2 bg-surface-200 border border-black/10 dark:border-white/5">
          <div className="text-sm text-slate-900 dark:text-white font-semibold">{title}</div>
          <div className="text-xs text-gray-500 mt-0.5">{descs[i] ?? ''}</div>
        </div>
      ))}
    </div>
  )
}

function Architecture({ t }: SectionProps) {
  const [openLayer, setOpenLayer] = useState<number | null>(null)
  const { data: registry } = useRegistry()

  return (
    <section id="architecture" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.architecture.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.architecture.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{t.architecture.desc}</p>
        </FadeIn>

        <div className="space-y-px">
          {t.architecture.layers.map((layer, i) => {
            const Icon = layerIcons[i]!
            const isOpen = openLayer === i
            return (
              <FadeIn key={i} delay={i * 80}>
                <div className="border border-black/10 dark:border-white/5 bg-surface-100 transition-all">
                  <button
                    onClick={() => setOpenLayer(isOpen ? null : i)}
                    className="w-full flex items-center gap-6 hover:bg-surface-200 px-6 md:px-8 py-6 transition-all text-left"
                  >
                    <div className="w-10 text-right font-mono text-sm text-gray-400 dark:text-gray-600 shrink-0">0{i + 1}</div>
                    <div className={cn('shrink-0', layerColors[i])}>
                      <Icon className="w-5 h-5" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="font-bold text-slate-900 dark:text-white text-lg">{layer.label}</div>
                      <div className="text-gray-500 text-sm mt-0.5">{layer.desc}</div>
                    </div>
                    <ChevronRight className={cn('w-4 h-4 text-gray-300 dark:text-gray-700 transition-transform shrink-0', isOpen && 'rotate-90 text-cyan-600 dark:text-cyan-500')} />
                  </button>
                  {isOpen && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.3, ease: 'easeOut' }}
                      className="overflow-hidden px-6 md:px-8 pb-6 border-t border-black/10 dark:border-white/5"
                    >
                      <div className="pt-4">
                        {i === 0 && (
                          <div className="grid grid-cols-2 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 gap-2">
                            {sortByPopularity(
                              registry?.channels && registry.channels.length > 0 ? registry.channels : []
                            ).map(ch => (
                              <div key={ch.id} className={cn(
                                'px-2 py-1.5 border text-xs font-mono text-center truncate',
                                isPopular(ch) ? 'bg-amber-500/10 border-amber-500/30 text-amber-600 dark:text-amber-300' : 'bg-surface-200 border-black/10 dark:border-white/5 text-gray-600 dark:text-gray-400'
                              )}>
                                {ch.name}{isPopular(ch) && ' 🔥'}
                              </div>
                            ))}
                          </div>
                        )}
                        {i === 1 && (
                          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-2">
                            {sortByPopularity(
                              registry?.hands && registry.hands.length > 0 ? registry.hands : []
                            ).map(h => (
                              <div key={h.id} className={cn(
                                'px-3 py-2 border',
                                isPopular(h) ? 'bg-amber-500/10 border-amber-500/30' : 'bg-surface-200 border-black/10 dark:border-white/5'
                              )}>
                                <div className="text-sm text-slate-900 dark:text-white font-semibold">
                                  {h.name}{isPopular(h) && ' 🔥'}
                                </div>
                                <div className="text-[10px] text-gray-400 dark:text-gray-600 font-mono uppercase">{h.category}</div>
                              </div>
                            ))}
                          </div>
                        )}
                        {i === 2 && <DetailGrid titles={layerTitles.kernel} descs={t.architecture.kernelDescs ?? []} />}
                        {i === 3 && <DetailGrid titles={layerTitles.runtime} descs={t.architecture.runtimeDescs ?? []} />}
                        {i === 4 && <DetailGrid titles={layerTitles.hardware} descs={t.architecture.hardwareDescs ?? []} />}
                      </div>
                    </motion.div>
                  )}
                </div>
              </FadeIn>
            )
          })}
        </div>
      </div>
    </section>
  )
}

// ─── Popular items (read from tags field) ───
function isPopular(item: { tags?: string[] }): boolean {
  return item.tags?.includes('popular') ?? false
}

function sortByPopularity<T extends { tags?: string[] }>(items: T[]): T[] {
  return [...items].sort((a, b) => {
    const ap = isPopular(a) ? 0 : 1
    const bp = isPopular(b) ? 0 : 1
    return ap - bp
  })
}

// ─── Hands (Features) — horizontal scroll carousel ───
const categoryColors: Record<string, string> = {
  content: 'text-amber-400 border-amber-400/20',
  data: 'text-cyan-600 dark:text-cyan-400 border-cyan-400/20',
  productivity: 'text-emerald-400 border-emerald-400/20',
  communication: 'text-purple-400 border-purple-400/20',
  development: 'text-rose-400 border-rose-400/20',
  research: 'text-blue-400 border-blue-400/20',
}

function Hands({ t }: SectionProps) {
  const { data: registry } = useRegistry()
  const lang = useAppStore((s) => s.lang)
  const rawHands = registry?.hands && registry.hands.length > 0 ? registry.hands : []
  const hands = sortByPopularity(rawHands)
  const scrollRef = useRef<HTMLDivElement>(null)

  return (
    <section id="hands" className="py-28 scroll-mt-20">
      <div className="max-w-6xl mx-auto px-6">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.hands.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.hands.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{t.hands.desc}</p>
        </FadeIn>
      </div>

      <div className="max-w-6xl mx-auto px-6">
        <FadeIn>
          <div
            ref={scrollRef}
            className="overflow-x-auto scrollbar-hide -mr-6 pr-6 pb-4 touch-pan-x"
          >
            <div className="grid grid-rows-2 grid-flow-col gap-3 w-max">
              {hands.map((hand) => {
                const colorClass = categoryColors[hand.category] ?? 'text-gray-600 dark:text-gray-400/60'
                return (
                  <a
                    key={hand.id}
                    href={`https://docs.librefang.ai/hands#${hand.id}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="group w-56 bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 px-4 py-3 hover:bg-surface-200 hover:scale-[1.02] transition-all duration-200"
                  >
                    <div className="flex items-center gap-2 mb-1.5">
                      <h3 className="text-sm font-bold text-slate-900 dark:text-white truncate">{hand.name.replace(' Hand', '')}</h3>
                      <span className={cn('text-[10px] uppercase tracking-wide shrink-0', colorClass)}>
                        {hand.category}
                      </span>
                    </div>
                    <p className="text-xs text-gray-500 leading-relaxed line-clamp-2">{getLocalizedDesc(hand, lang)}</p>
                  </a>
                )
              })}
            </div>
          </div>
        </FadeIn>
      </div>
    </section>
  )
}

// ─── Performance Comparison ───
function Performance({ t }: SectionProps) {
  return (
    <section id="performance" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.performance.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.performance.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{t.performance.desc}</p>
        </FadeIn>

        <FadeIn delay={100}>
          <div className="hidden md:block border border-black/10 dark:border-white/5 overflow-hidden">
            <table className="w-full text-left">
              <thead>
                <tr className="bg-surface-200 text-xs uppercase tracking-widest">
                  <th className="px-6 py-4 font-semibold text-gray-500">{t.performance.metric}</th>
                  <th className="px-6 py-4 font-semibold text-gray-500 text-center border-l border-black/10 dark:border-white/5">{t.performance.others}</th>
                  <th className="px-6 py-4 font-semibold text-cyan-600 dark:text-cyan-500 text-center border-l border-cyan-500/10 bg-cyan-500/5">LibreFang</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-black/10 dark:divide-white/5">
                {t.performance.rows.map((row, i) => (
                  <tr key={i} className="hover:bg-black/[0.02] dark:hover:bg-white/[0.02] transition-colors">
                    <td className="px-6 py-4 text-sm font-medium text-gray-700 dark:text-gray-300">{row.metric}</td>
                    <td className="px-6 py-4 text-sm text-center text-gray-500 font-mono border-l border-black/10 dark:border-white/5">{row.others}</td>
                    <td className="px-6 py-4 text-sm text-center text-cyan-600 dark:text-cyan-400 font-mono font-bold border-l border-cyan-500/10 bg-cyan-500/5">{row.librefang}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <div className="md:hidden space-y-3">
            {t.performance.rows.map((row, i) => (
              <div key={i} className="bg-surface-100 border border-black/10 dark:border-white/5 p-4">
                <div className="text-xs text-gray-500 uppercase tracking-widest mb-3">{row.metric}</div>
                <div className="flex justify-between items-baseline">
                  <div className="text-sm text-gray-500">{t.performance.others}: <span className="font-mono">{row.others}</span></div>
                  <div className="text-xl font-black font-mono text-cyan-600 dark:text-cyan-400">{row.librefang}</div>
                </div>
              </div>
            ))}
          </div>
        </FadeIn>
      </div>
    </section>
  )
}

// ─── Workflows ───
const workflowIcons: LucideIcon[] = [Scissors, Users, Eye, Network, ArrowRight, Shield]

function Workflows({ t }: SectionProps) {
  if (!t.workflows) return null
  return (
    <section id="workflows" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.workflows.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.workflows.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{t.workflows.desc}</p>
        </FadeIn>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
          {t.workflows.items.map((item, i) => {
            const Icon = workflowIcons[i] || Box
            return (
              <FadeIn key={i} delay={i * 60}>
                <div className="group bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 p-6 transition-all hover:bg-surface-200">
                  <Icon className="w-5 h-5 text-amber-400/60 group-hover:text-amber-400 transition-colors mb-4" />
                  <h3 className="text-lg font-bold text-slate-900 dark:text-white mb-2">{item.title}</h3>
                  <p className="text-sm text-gray-500 leading-relaxed">{item.desc}</p>
                </div>
              </FadeIn>
            )
          })}
        </div>
      </div>
    </section>
  )
}

// ─── Downloads ───

interface ReleaseAsset {
  name: string
  browser_download_url: string
  size: number
}

interface DownloadItem {
  label: string
  desc: string
  icon: LucideIcon
  assets: { name: string; url: string; size: string }[]
}

function formatSize(bytes: number): string {
  if (bytes > 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(0)}MB`
  return `${(bytes / 1024).toFixed(0)}KB`
}

function categorizeAssets(assets: ReleaseAsset[]): DownloadItem[] {
  const desktop: DownloadItem = {
    label: 'Desktop App',
    desc: 'Tauri 2.0 native app',
    icon: Monitor,
    assets: [],
  }
  const cli: DownloadItem = {
    label: 'CLI',
    desc: 'Command-line binary',
    icon: Terminal,
    assets: [],
  }

  // Desktop patterns
  const desktopPatterns = [
    { pattern: /x64\.dmg$/, name: 'macOS (Intel) .dmg' },
    { pattern: /aarch64\.dmg$/, name: 'macOS (Apple Silicon) .dmg' },
    { pattern: /x64-setup\.exe$/, name: 'Windows (x64) .exe' },
    { pattern: /arm64-setup\.exe$/, name: 'Windows (ARM) .exe' },
    { pattern: /amd64\.AppImage$/, name: 'Linux (x64) .AppImage' },
    { pattern: /amd64\.deb$/, name: 'Linux (x64) .deb' },
    { pattern: /x86_64\.rpm$/, name: 'Linux (x64) .rpm' },
  ]

  // CLI patterns
  const cliPatterns = [
    { pattern: /x86_64-apple-darwin\.tar\.gz$/, name: 'macOS (Intel)' },
    { pattern: /aarch64-apple-darwin\.tar\.gz$/, name: 'macOS (Apple Silicon)' },
    { pattern: /x86_64-unknown-linux-gnu\.tar\.gz$/, name: 'Linux (x64 glibc)' },
    { pattern: /x86_64-unknown-linux-musl\.tar\.gz$/, name: 'Linux (x64 musl)' },
    { pattern: /aarch64-unknown-linux-gnu\.tar\.gz$/, name: 'Linux (ARM64)' },
    { pattern: /x86_64-pc-windows-msvc\.zip$/, name: 'Windows (x64)' },
    { pattern: /aarch64-pc-windows-msvc\.zip$/, name: 'Windows (ARM)' },
    { pattern: /aarch64-linux-android\.tar\.gz$/, name: 'Android (Termux)' },
  ]

  for (const asset of assets) {
    if (asset.name.endsWith('.sha256')) continue
    for (const p of desktopPatterns) {
      if (p.pattern.test(asset.name)) {
        desktop.assets.push({ name: p.name, url: asset.browser_download_url, size: formatSize(asset.size) })
        break
      }
    }
    for (const p of cliPatterns) {
      if (p.pattern.test(asset.name)) {
        cli.assets.push({ name: p.name, url: asset.browser_download_url, size: formatSize(asset.size) })
        break
      }
    }
  }

  return [desktop, cli]
}

// Need to import Monitor at the top - it's used in Downloads
// Adding it via the LucideIcon type already imported

function Downloads(_props: SectionProps) {
  const lang = useAppStore((s) => s.lang)
  const { data: release, isLoading } = useQuery({
    queryKey: ['latestRelease'],
    queryFn: async () => {
      const res = await fetch('https://api.github.com/repos/librefang/librefang/releases/latest', {
        headers: { Accept: 'application/vnd.github.v3+json' },
      })
      if (!res.ok) throw new Error('Failed')
      return res.json() as Promise<{ tag_name: string; assets: ReleaseAsset[]; html_url: string }>
    },
    staleTime: 1000 * 60 * 60,
    retry: 2,
  })

  const categories = release ? categorizeAssets(release.assets) : []
  const version = release?.tag_name ?? ''

  const labels: Record<string, Record<string, string>> = {
    title: { en: 'Downloads', zh: '下载', 'zh-TW': '下載', ja: 'ダウンロード', ko: '다운로드', de: 'Downloads', es: 'Descargas' },
    desc: {
      en: 'Desktop app, CLI binaries, and deployment options.',
      zh: '桌面应用、CLI 二进制文件和部署选项。',
      'zh-TW': '桌面應用、CLI 二進位檔和部署選項。',
      ja: 'デスクトップアプリ、CLIバイナリ、デプロイオプション。',
      ko: '데스크톱 앱, CLI 바이너리, 배포 옵션.',
      de: 'Desktop-App, CLI-Binaries und Deployment-Optionen.',
      es: 'App de escritorio, binarios CLI y opciones de despliegue.',
    },
    onlineDeply: { en: 'One-Click Deploy', zh: '一键部署', 'zh-TW': '一鍵部署', ja: 'ワンクリックデプロイ', ko: '원클릭 배포', de: 'Ein-Klick-Deploy', es: 'Despliegue en un clic' },
    allReleases: { en: 'All Releases', zh: '所有版本', 'zh-TW': '所有版本', ja: '全リリース', ko: '모든 릴리스', de: 'Alle Releases', es: 'Todas las versiones' },
    sdk: { en: 'SDK & Packages', zh: 'SDK 和包', 'zh-TW': 'SDK 和套件', ja: 'SDK & パッケージ', ko: 'SDK & 패키지', de: 'SDK & Pakete', es: 'SDK y paquetes' },
  }

  const l = (key: string) => labels[key]?.[lang] ?? labels[key]?.['en'] ?? key

  return (
    <section id="downloads" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">
            {version && <span className="text-gray-400 dark:text-gray-600 mr-2">{version}</span>}
            {l('title')}
          </div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{l('title')}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{l('desc')}</p>
        </FadeIn>

        {/* Desktop & CLI */}
        {isLoading ? (
          <div className="text-gray-400 dark:text-gray-600 text-center py-12">Loading releases...</div>
        ) : (
          <div className="grid md:grid-cols-2 gap-6 mb-8">
            {categories.map((cat) => (
              <FadeIn key={cat.label}>
                <div className="bg-surface-100 border border-black/10 dark:border-white/5 p-6 hover:-translate-y-0.5 transition-transform h-full">
                  <div className="flex items-center gap-3 mb-4">
                    <cat.icon className="w-5 h-5 text-cyan-600 dark:text-cyan-500" />
                    <div>
                      <h3 className="text-base font-bold text-slate-900 dark:text-white">{cat.label}</h3>
                      <span className="text-xs text-gray-500">{cat.desc}</span>
                    </div>
                  </div>
                  <div className="space-y-1.5">
                    {cat.assets.map((a) => (
                      <a
                        key={a.name}
                        href={a.url}
                        className="flex items-center justify-between px-3 py-2 hover:bg-surface-200 transition-colors group"
                      >
                        <span className="text-sm text-gray-700 dark:text-gray-300 group-hover:text-cyan-600 dark:group-hover:text-cyan-400 transition-colors">{a.name}</span>
                        <span className="text-xs text-gray-400 dark:text-gray-600 font-mono">{a.size}</span>
                      </a>
                    ))}
                  </div>
                </div>
              </FadeIn>
            ))}
          </div>
        )}

        {/* SDK, Deploy, All Releases */}
        <FadeIn delay={200}>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
            <a href="https://deploy.librefang.ai/" target="_blank" rel="noopener noreferrer" className="flex items-center gap-3 bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 px-5 py-4 transition-all group">
              <Globe className="w-5 h-5 text-cyan-500/60 group-hover:text-cyan-600 dark:group-hover:text-cyan-400 shrink-0" />
              <div>
                <div className="text-sm font-bold text-slate-900 dark:text-white">{l('onlineDeply')}</div>
                <div className="text-xs text-gray-500">deploy.librefang.ai</div>
              </div>
            </a>
            <a href="https://github.com/librefang/librefang/releases" target="_blank" rel="noopener noreferrer" className="flex items-center gap-3 bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 px-5 py-4 transition-all group">
              <Github className="w-5 h-5 text-cyan-500/60 group-hover:text-cyan-600 dark:group-hover:text-cyan-400 shrink-0" />
              <div>
                <div className="text-sm font-bold text-slate-900 dark:text-white">{l('allReleases')}</div>
                <div className="text-xs text-gray-500">GitHub Releases</div>
              </div>
            </a>
            <a href="https://docs.librefang.ai/sdk" target="_blank" rel="noopener noreferrer" className="flex items-center gap-3 bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 px-5 py-4 transition-all group">
              <Box className="w-5 h-5 text-cyan-500/60 group-hover:text-cyan-600 dark:group-hover:text-cyan-400 shrink-0" />
              <div>
                <div className="text-sm font-bold text-slate-900 dark:text-white">{l('sdk')}</div>
                <div className="text-xs text-gray-500">Python / Node.js / Rust</div>
              </div>
            </a>
          </div>
        </FadeIn>
      </div>
    </section>
  )
}

// ─── Install ───
function Install({ t }: SectionProps) {
  const [copied, setCopied] = useState(false)
  const [os, setOs] = useState<'mac' | 'windows' | 'linux' | 'unknown'>('unknown')
  const cmd = os === 'windows' ? 'irm https://librefang.sh/install.ps1 | iex' : 'curl -fsSL https://librefang.sh/install | sh'

  useEffect(() => {
    const ua = navigator.userAgent.toLowerCase()
    if (ua.includes('mac')) setOs('mac')
    else if (ua.includes('win')) setOs('windows')
    else if (ua.includes('linux')) setOs('linux')
  }, [])

  const copy = () => {
    navigator.clipboard.writeText(cmd)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <section id="install" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-3xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.install.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.install.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg mb-12">{t.install.desc}</p>
        </FadeIn>

        <FadeIn delay={100}>
          <div className="border border-black/10 dark:border-white/10 bg-surface-100 overflow-hidden glow-cyan">
            <div className="flex items-center justify-between px-4 py-2.5 bg-surface-200 border-b border-black/10 dark:border-white/5">
              <div className="flex gap-1.5">
                <div className="w-2.5 h-2.5 rounded-full bg-black/10 dark:bg-white/10" />
                <div className="w-2.5 h-2.5 rounded-full bg-black/10 dark:bg-white/10" />
                <div className="w-2.5 h-2.5 rounded-full bg-black/10 dark:bg-white/10" />
              </div>
              <span className="text-[10px] font-mono text-gray-600 uppercase tracking-widest">{t.install.terminal}</span>
              <button onClick={() => { copy(); trackEvent('click', 'install_copy') }} className="text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors p-1" aria-label="Copy">
                {copied ? <Check className="w-3.5 h-3.5 text-cyan-600 dark:text-cyan-400" /> : <Copy className="w-3.5 h-3.5" />}
              </button>
            </div>
            <div className="p-6 font-mono text-sm md:text-base space-y-4">
              <div className="flex gap-3">
                <span className="text-cyan-600 dark:text-cyan-500 select-none">$</span>
                <span className="text-gray-800 dark:text-gray-200">curl -fsSL https://librefang.sh/install | sh</span>
              </div>
              <div className="flex gap-3">
                <span className="text-cyan-600 dark:text-cyan-500 select-none">$</span>
                <span className="text-gray-800 dark:text-gray-200">librefang init</span>
              </div>
              <div className="flex gap-3">
                <span className="text-cyan-600 dark:text-cyan-500 select-none">$</span>
                <span className="text-gray-800 dark:text-gray-200">librefang start</span>
              </div>
              <div className="text-gray-600 text-xs mt-2">
                <span className="text-amber-500/60">#</span> {t.install.comment}
              </div>
              <div className="flex gap-2 mt-3 pt-3 border-t border-black/10 dark:border-white/5">
                {(['mac', 'windows', 'linux'] as const).map(p => (
                  <span key={p} className={cn(
                    'text-[10px] font-mono px-2 py-0.5 rounded',
                    os === p ? 'bg-cyan-500/20 text-cyan-600 dark:text-cyan-400' : 'text-gray-600'
                  )}>
                    {p === 'mac' ? 'macOS' : p === 'windows' ? 'Windows' : 'Linux'}
                    {os === p && ' \u2713'}
                  </span>
                ))}
              </div>
            </div>
          </div>
        </FadeIn>

        <FadeIn delay={200}>
          <div className="grid sm:grid-cols-2 gap-px mt-6 bg-black/5 dark:bg-white/5 overflow-hidden">
            <div className="bg-surface-100 p-5">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">{t.install.requires}</div>
              <ul className="space-y-2 text-sm text-gray-600 dark:text-gray-400">
                {t.install.reqItems.map((item, i) => (
                  <li key={i} className="flex items-center gap-2"><span className="w-1 h-1 bg-cyan-500 rounded-full" /> {item}</li>
                ))}
              </ul>
            </div>
            <div className="bg-surface-100 p-5">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">{t.install.includes}</div>
              <ul className="space-y-2 text-sm text-gray-600 dark:text-gray-400">
                {t.install.incItems.map((item, i) => (
                  <li key={i} className="flex items-center gap-2"><span className="w-1 h-1 bg-amber-400 rounded-full" /> {item}</li>
                ))}
              </ul>
            </div>
          </div>
        </FadeIn>
      </div>
    </section>
  )
}

// ─── FAQ ───
function FAQ({ t }: SectionProps) {
  const [openIndex, setOpenIndex] = useState<number | null>(0)

  return (
    <section id="faq" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.faq.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-12">{t.faq.title}</h2>
        </FadeIn>

        <div className="space-y-px">
          {t.faq.items.map((item, i) => (
            <FadeIn key={i} delay={i * 60}>
              <div className="border border-black/10 dark:border-white/5 bg-surface-100 transition-colors">
                <button
                  onClick={() => setOpenIndex(openIndex === i ? null : i)}
                  className="w-full flex items-center justify-between px-6 py-5 text-left"
                >
                  <span className="font-semibold text-slate-900 dark:text-white text-sm pr-4">{item.q}</span>
                  <ChevronRight className={cn('w-4 h-4 text-gray-300 dark:text-gray-600 transition-transform shrink-0', openIndex === i && 'rotate-90 text-cyan-600 dark:text-cyan-500')} />
                </button>
                {openIndex === i && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: 'auto', opacity: 1 }}
                    transition={{ duration: 0.3, ease: 'easeOut' }}
                    className="overflow-hidden"
                  >
                    <div className="px-6 pb-5 text-sm text-gray-500 dark:text-gray-400 leading-relaxed border-t border-black/10 dark:border-white/5 pt-4">
                      {item.a}
                    </div>
                  </motion.div>
                )}
              </div>
            </FadeIn>
          ))}
        </div>
      </div>
    </section>
  )
}

// ─── Community ───
const communityHrefs: string[] = [
  'https://github.com/librefang/librefang/pulls',
  'https://github.com/librefang/librefang/issues',
  'https://github.com/librefang/librefang/discussions',
  'https://discord.gg/DzTYqAZZmc',
]
const communityIcons: LucideIcon[] = [GitPullRequest, CircleDot, MessageSquare, MessageSquare]

function Community({ t }: SectionProps) {
  return (
    <section className="py-28 px-6 border-t border-black/10 dark:border-white/5">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.community.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.community.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{t.community.desc}</p>
        </FadeIn>

        <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-4">
          {t.community.items.map((item, i) => {
            const Icon = communityIcons[i]!
            return (
              <FadeIn key={i} delay={i * 80}>
                <a
                  href={communityHrefs[i]}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="group flex flex-col bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 p-6 transition-all hover:-translate-y-1 h-full"
                >
                  <Icon className="w-5 h-5 text-cyan-500/60 group-hover:text-cyan-600 dark:group-hover:text-cyan-400 transition-colors mb-4" />
                  <h3 className="font-bold text-slate-900 dark:text-white mb-1">{item.label}</h3>
                  <p className="text-sm text-gray-500 line-clamp-1">{item.desc}</p>
                  <div className="mt-4 text-cyan-600 dark:text-cyan-500 text-sm font-semibold flex items-center gap-1 group-hover:gap-2 transition-all">
                    {t.community.open} <ArrowRight className="w-3.5 h-3.5" />
                  </div>
                </a>
              </FadeIn>
            )
          })}
        </div>
      </div>
    </section>
  )
}

// ─── GitHub Stats ───
function formatNumber(num: number | null | undefined): string {
  if (num === null || num === undefined) return '-'
  if (num >= 1000) return `${(num / 1000).toFixed(1)}k`
  return String(num)
}

interface GitHubStatsData {
  stars?: number
  forks?: number
  issues?: number
  prs?: number
  downloads?: number
  lastUpdate?: string
  starHistory?: { stars: number }[]
}

function GitHubStats({ t }: SectionProps) {
  const gs = t.githubStats
  if (!gs) return null

  /* eslint-disable react-hooks/rules-of-hooks */
  const [data, setData] = useState<GitHubStatsData | null>(null)
  const [docsVisits, setDocsVisits] = useState(0)
  const [loading, setLoading] = useState(true)
  /* eslint-enable react-hooks/rules-of-hooks */

  useEffect(() => {
    Promise.all([
      fetch('https://stats.librefang.ai/api/github').then(r => r.ok ? r.json() as Promise<GitHubStatsData> : null).catch(() => null),
      fetch('https://counter.librefang.ai/api').then(r => r.ok ? r.json() as Promise<{ total: number }> : { total: 0 }).catch(() => ({ total: 0 })),
    ]).then(([gh, docs]) => {
      setData(gh)
      setDocsVisits(docs?.total || 0)
      setLoading(false)
    })
  }, [])

  const stars = data?.stars ?? 0
  const forks = data?.forks ?? 0
  const issues = data?.issues ?? 0
  const prs = data?.prs ?? 0
  const downloads = data?.downloads ?? 0
  const lastUpdate = data?.lastUpdate ? new Date(data.lastUpdate).toLocaleDateString() : '-'
  const starHistory = data?.starHistory || []

  const chartData = starHistory.length > 0 ? starHistory.map(d => d.stars) : (stars > 0 ? [stars] : [0])
  const chartMax = Math.max(...chartData, 1)

  return (
    <section className="py-28 px-6 border-t border-black/10 dark:border-white/5" id="github-stats">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{gs.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{gs.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{gs.desc}</p>
        </FadeIn>

        <FadeIn delay={100}>
          <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-7 gap-px bg-black/5 dark:bg-white/5 rounded overflow-hidden mb-8">
            {[
              { icon: <Star className="w-4 h-4" />, value: formatNumber(stars), label: gs.stars },
              { icon: <GitFork className="w-4 h-4" />, value: formatNumber(forks), label: gs.forks },
              { icon: <CircleDot className="w-4 h-4" />, value: formatNumber(issues), label: gs.issues },
              { icon: <GitPullRequest className="w-4 h-4" />, value: formatNumber(prs), label: gs.prs },
              { icon: <ArrowRight className="w-4 h-4" />, value: formatNumber(downloads), label: gs.downloads },
              { icon: <Eye className="w-4 h-4" />, value: formatNumber(docsVisits), label: gs.docsVisits },
              { icon: <Zap className="w-4 h-4" />, value: lastUpdate, label: gs.lastUpdate },
            ].map((stat, i) => (
              <div key={i} className="bg-surface-100 p-4 text-center">
                <div className="flex justify-center mb-1.5 text-cyan-500/50">{stat.icon}</div>
                <div className="text-xl font-black text-slate-900 dark:text-white font-mono">
                  {loading ? <span className="inline-block w-10 h-5 bg-gray-300/50 dark:bg-gray-700/50 rounded animate-pulse" /> : stat.value}
                </div>
                <div className="text-[10px] text-gray-500 uppercase tracking-widest mt-1">{stat.label}</div>
              </div>
            ))}
          </div>
        </FadeIn>

        {/* Star History + Contributors - side by side */}
        <FadeIn delay={200}>
          <div className="grid md:grid-cols-2 gap-4 mb-12">
            <div className="bg-surface-100 border border-black/10 dark:border-white/5 p-5">
              <div className="flex items-center justify-between mb-4">
                <span className="text-sm font-bold text-slate-900 dark:text-white">{gs.starHistory}</span>
                <a href="https://star-history.com/#librefang/librefang" target="_blank" rel="noopener noreferrer" className="text-xs text-gray-400 dark:text-gray-600 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">View Full</a>
              </div>
              <div className="h-32 flex items-end gap-0.5">
                {starHistory.length >= 3 ? (
                  Array.from({ length: Math.min(30, chartData.length) }, (_, i) => {
                    const idx = Math.floor((i / Math.min(30, chartData.length)) * chartData.length)
                    const value = chartData[idx] || 0
                    return <div key={i} className="flex-1 bg-cyan-500/30 hover:bg-cyan-500 transition-colors rounded-t min-w-0.5" style={{ height: `${Math.max(4, (value / chartMax) * 100)}%` }} />
                  })
                ) : (
                  <div className="w-full h-full flex flex-col items-center justify-center text-gray-500">
                    <span className="text-3xl font-black text-cyan-600 dark:text-cyan-400 font-mono">{stars}</span>
                    <span className="text-xs mt-1">{gs.stars}</span>
                  </div>
                )}
              </div>
            </div>
            <a href="https://github.com/librefang/librefang/graphs/contributors" target="_blank" rel="noopener noreferrer" className="block bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 p-5 transition-all">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">Contributors</div>
              <img src="https://contrib.rocks/image?repo=librefang/librefang&anon=0" alt="Contributors" className="w-full h-auto rounded" loading="lazy" />
            </a>
          </div>
        </FadeIn>

        <FadeIn delay={400}>
          <div className="flex flex-col sm:flex-row justify-center gap-3">
            <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="inline-flex items-center justify-center gap-2 px-6 py-3 border border-black/10 dark:border-white/10 hover:border-cyan-500/30 hover:bg-cyan-500/10 text-slate-900 dark:text-white font-semibold rounded transition-all">
              <Star className="w-4 h-4" />
              {gs.starUs}
            </a>
            <a href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer" className="inline-flex items-center justify-center gap-2 px-6 py-3 border border-black/10 dark:border-white/10 hover:border-black/20 dark:hover:border-white/20 text-gray-700 dark:text-gray-300 font-semibold rounded transition-all hover:bg-black/5 dark:hover:bg-white/5">
              <MessageSquare className="w-4 h-4" />
              {gs.discuss}
            </a>
          </div>
        </FadeIn>
      </div>
    </section>
  )
}

// ─── Docs ───
function Docs({ t }: SectionProps) {
  if (!t.docs) return null
  return (
    <section id="docs" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3">{t.docs.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-4">{t.docs.title}</h2>
          <p className="text-gray-600 dark:text-gray-400 text-lg max-w-2xl mb-16">{t.docs.desc}</p>
        </FadeIn>

        <div className="grid md:grid-cols-3 gap-4 mb-8">
          {t.docs.categories.map((cat, i) => (
            <FadeIn key={i} delay={i * 80}>
              <div className="bg-surface-100 border border-black/10 dark:border-white/5 hover:border-cyan-500/20 p-6 transition-all">
                <h3 className="font-bold text-slate-900 dark:text-white mb-2">{cat.title}</h3>
                <p className="text-sm text-gray-500">{cat.desc}</p>
              </div>
            </FadeIn>
          ))}
        </div>

        <FadeIn delay={300}>
          <div className="text-center">
            <a href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-2 text-cyan-500 font-semibold text-sm hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">
              {t.docs.viewAll} <ExternalLink className="w-3.5 h-3.5" />
            </a>
          </div>
        </FadeIn>
      </div>
    </section>
  )
}

// ─── Footer ───
function Footer({ t }: SectionProps) {
  return (
    <footer className="border-t border-black/10 dark:border-white/5 py-12 px-6">
      <div className="max-w-6xl mx-auto flex flex-col md:flex-row items-center justify-between gap-6">
        <div className="flex items-center gap-2.5">
          <img src="/logo.png" alt="LibreFang" className="w-6 h-6 rounded" />
          <span className="text-sm font-semibold text-gray-600 dark:text-gray-400">LibreFang</span>
          <span className="text-xs text-gray-400 dark:text-gray-600 font-mono">Agent OS</span>
        </div>
        <div className="flex flex-wrap items-center justify-center gap-x-6 gap-y-2 text-xs text-gray-500 dark:text-gray-600 font-medium">
          <a href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{t.footer.docs}</a>
          <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">GitHub</a>
          <a href="https://github.com/librefang/librefang/blob/main/LICENSE" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{t.footer.license}</a>
          <a href="/changelog/" className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{t.footer.changelog}</a>
          <a href="/privacy/" className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{t.footer.privacy}</a>
        </div>
        <div className="text-xs text-gray-300 dark:text-gray-700">&copy; {new Date().getFullYear()} LibreFang.ai</div>
      </div>
    </footer>
  )
}

// ─── GA event tracking ───
function trackEvent(action: string, label: string) {
  if (typeof window !== 'undefined' && 'gtag' in window) {
    (window as any).gtag('event', action, { event_category: 'engagement', event_label: label })
  }
}

// ─── Back to top ───
function BackToTop() {
  const [show, setShow] = useState(false)
  useEffect(() => {
    const onScroll = () => setShow(window.scrollY > window.innerHeight)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])
  if (!show) return null
  return (
    <button
      onClick={() => window.scrollTo({ top: 0, behavior: 'smooth' })}
      className="fixed bottom-6 right-6 z-40 p-3 bg-surface-200 border border-black/10 dark:border-white/10 hover:border-cyan-500/30 hover:bg-cyan-500/10 text-gray-600 dark:text-gray-400 hover:text-cyan-600 dark:hover:text-cyan-400 transition-all rounded"
      aria-label="Back to top"
    >
      <ArrowRight className="w-4 h-4 -rotate-90" />
    </button>
  )
}

// ─── App ───
export default function App() {
  const lang = useAppStore((s) => s.lang)
  const switchLang = useAppStore((s) => s.switchLang)

  useEffect(() => {
    document.documentElement.lang = lang
  }, [lang])

  useEffect(() => {
    const onPopState = () => useAppStore.setState({ lang: getCurrentLang() })
    window.addEventListener('popstate', onPopState)
    return () => window.removeEventListener('popstate', onPopState)
  }, [])

  const t = translations[lang] || translations['en']!
  const { data: registry } = useRegistry()

  // Update meta tags on language change
  useEffect(() => {
    if (t.meta) {
      document.title = t.meta.title
      const descMeta = document.querySelector('meta[name="description"]')
      if (descMeta) descMeta.setAttribute('content', t.meta.description)
      const ogTitle = document.querySelector('meta[property="og:title"]')
      if (ogTitle) ogTitle.setAttribute('content', t.meta.title)
      const ogDesc = document.querySelector('meta[property="og:description"]')
      if (ogDesc) ogDesc.setAttribute('content', t.meta.description)
    }
  }, [lang, t])

  return (
    <main className="min-h-screen">
      <a href="#architecture" className="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-[60] focus:px-4 focus:py-2 focus:bg-cyan-500 focus:text-surface focus:font-bold focus:rounded">
        Skip to content
      </a>
      <Nav t={t} lang={lang} onSwitchLang={switchLang} />
      <Hero t={t} registry={registry} />
      <div className="glow-line" />
      <Architecture t={t} />
      <div className="glow-line" />
      <Hands t={t} />
      <Workflows t={t} />
      <Performance t={t} />
      <div className="glow-line" />
      <Install t={t} />
      <Downloads t={t} />
      <Docs t={t} />
      <FAQ t={t} />
      <GitHubStats t={t} />
      <Community t={t} />
      <Footer t={t} />
      <BackToTop />
    </main>
  )
}
