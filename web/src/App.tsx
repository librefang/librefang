import { useState, useEffect, useRef } from 'react'
import { motion } from 'framer-motion'
import {
  Terminal, Cpu, Shield, Zap, Network, ChevronRight, ChevronDown, ExternalLink,
  Copy, Check, Menu, X, Box, Layers, Radio, Eye,
  Scissors, Users, Globe, ArrowRight, Github,
  Star, GitFork, CircleDot, GitPullRequest, MessageSquare
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
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
  const currentLangName = languages.find(l => l.code === lang)?.name || 'English'

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
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
    { label: t.nav.docs, href: 'https://docs.librefang.ai', external: true },
  ]

  return (
    <nav className={cn('fixed top-0 left-0 right-0 z-50 transition-all duration-300', scrolled && 'bg-surface/90 backdrop-blur-md border-b border-white/5')}>
      <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between">
        <a href="/" className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded bg-cyan-500/10 border border-cyan-500/20 flex items-center justify-center">
            <Terminal className="w-4 h-4 text-cyan-400" />
          </div>
          <span className="font-bold text-white tracking-tight">LibreFang</span>
        </a>

        <div className="hidden md:flex items-center gap-1">
          {links.map(link => (
            <a
              key={link.label}
              href={link.href}
              target={link.external ? '_blank' : undefined}
              rel={link.external ? 'noopener noreferrer' : undefined}
              className="px-3 py-1.5 text-sm text-gray-400 hover:text-cyan-400 transition-colors font-medium flex items-center gap-1"
            >
              {link.label}
              {link.external && <ExternalLink className="w-3 h-3" />}
            </a>
          ))}

          {/* Language switcher */}
          <div className="relative ml-2" data-lang-menu>
            <button
              className="flex items-center gap-1.5 px-3 py-1.5 text-sm text-gray-400 hover:text-cyan-400 transition-colors font-medium"
              onClick={() => setLangOpen(!langOpen)}
              aria-label="Switch language"
              aria-expanded={langOpen}
            >
              <Globe className="w-3.5 h-3.5" />
              <span className="hidden lg:inline">{currentLangName}</span>
              <ChevronDown className={cn('w-3 h-3 transition-transform', langOpen && 'rotate-180')} />
            </button>
            {langOpen && (
              <div className="absolute right-0 mt-2 w-36 bg-surface-200 border border-white/10 rounded shadow-xl z-50">
                {languages.map(l => (
                  <button
                    key={l.code}
                    onClick={() => { onSwitchLang(l.code); setLangOpen(false) }}
                    className={cn('block w-full text-left px-4 py-2.5 text-sm transition-colors', l.code === lang ? 'text-cyan-400 bg-cyan-500/5' : 'text-gray-400 hover:text-white hover:bg-white/5')}
                  >
                    {l.name}
                  </button>
                ))}
              </div>
            )}
          </div>

          <a
            href="https://github.com/librefang/librefang"
            target="_blank"
            rel="noopener noreferrer"
            className="ml-3 px-4 py-1.5 text-sm font-semibold text-cyan-400 border border-cyan-500/30 rounded hover:bg-cyan-500/10 transition-all"
          >
            GitHub
          </a>
        </div>

        <button className="md:hidden text-gray-400" onClick={() => setOpen(!open)} aria-label="Toggle menu">
          {open ? <X className="w-5 h-5" /> : <Menu className="w-5 h-5" />}
        </button>
      </div>

      {open && (
        <div className="md:hidden bg-surface-100 border-t border-white/5 px-6 py-4 space-y-1">
          {links.map(link => (
            <a
              key={link.label}
              href={link.href}
              onClick={() => setOpen(false)}
              className="block py-2.5 text-sm text-gray-400 hover:text-cyan-400 transition-colors font-medium"
            >
              {link.label}
            </a>
          ))}
          <div className="pt-2 border-t border-white/5 mt-2 flex flex-wrap gap-2">
            {languages.map(l => (
              <button
                key={l.code}
                onClick={() => { onSwitchLang(l.code); setOpen(false) }}
                className={cn('px-3 py-1.5 text-xs rounded', l.code === lang ? 'text-cyan-400 bg-cyan-500/10' : 'text-gray-500 hover:text-white')}
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
              <div className="inline-flex items-center gap-2 px-3 py-1 rounded border border-cyan-500/20 bg-cyan-500/5 text-xs font-mono text-cyan-400 mb-8">
                <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-pulse" />
                v2026.3 &mdash; {t.hero.badge} &mdash; Rust
              </div>
            </FadeIn>

            <FadeIn delay={100}>
              <h1 className="text-5xl md:text-6xl lg:text-7xl font-black tracking-tight leading-[0.95] mb-6">
                <span className="text-white">{t.hero.title1}</span>
                <br />
                <span className="text-cyan-400">{t.hero.title2}</span>
              </h1>
            </FadeIn>

            <FadeIn delay={200}>
              <div className="flex items-center gap-2 text-base md:text-lg text-gray-500 font-mono mb-8 h-7">
                <span className="text-cyan-500">$</span>
                <span className="text-gray-300">{typed}</span>
                <span className="w-2 h-4 bg-cyan-400 cursor-blink" />
              </div>
            </FadeIn>

            <FadeIn delay={300}>
              <p className="text-gray-400 text-base leading-relaxed mb-8">{t.hero.desc}</p>
            </FadeIn>

            <FadeIn delay={400}>
              <div className="flex flex-col sm:flex-row gap-3">
                <a href="#install" className="inline-flex items-center justify-center gap-2 px-6 py-3 bg-cyan-500 hover:bg-cyan-400 text-surface font-bold rounded transition-all hover:shadow-lg hover:shadow-cyan-500/20">
                  {t.hero.getStarted}
                  <ArrowRight className="w-4 h-4" />
                </a>
                <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="inline-flex items-center justify-center gap-2 px-6 py-3 border border-white/10 hover:border-white/20 text-gray-300 font-semibold rounded transition-all hover:bg-white/5">
                  <Github className="w-4 h-4" />
                  {t.hero.viewGithub}
                </a>
              </div>
            </FadeIn>
          </div>

          {/* Right: system preview terminal */}
          <FadeIn delay={300}>
            <div className="hidden lg:block">
              <div className="border border-white/10 bg-surface-100 overflow-hidden glow-cyan">
                <div className="flex items-center gap-2 px-4 py-2.5 bg-surface-200 border-b border-white/5">
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
                    <div className="flex justify-between"><span className="text-gray-400">runtime</span><span className="text-cyan-400">running</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">uptime</span><span className="text-white">14d 7h 23m</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">memory</span><span className="text-white">38MB</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">agents</span><span className="text-white">4 active</span></div>
                  </div>
                  <div className="border-t border-white/5 pt-3 space-y-1.5">
                    <div className="text-amber-400/70">AGENTS</div>
                    <div className="flex justify-between"><span className="text-gray-400">clip</span><span className="text-cyan-400">● idle</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">lead</span><span className="text-green-400">● running</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">collector</span><span className="text-cyan-400">● idle</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">researcher</span><span className="text-green-400">● running</span></div>
                  </div>
                  <div className="border-t border-white/5 pt-3 space-y-1.5">
                    <div className="text-amber-400/70">CHANNELS</div>
                    <div className="flex justify-between"><span className="text-gray-400">telegram</span><span className="text-green-400">connected</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">slack</span><span className="text-green-400">connected</span></div>
                    <div className="flex justify-between"><span className="text-gray-400">discord</span><span className="text-gray-600">standby</span></div>
                  </div>
                </div>
              </div>
            </div>
          </FadeIn>
        </div>

        {/* Stats bar - full width below */}
        <FadeIn delay={500}>
          <div className="mt-16 grid grid-cols-2 md:grid-cols-4 gap-px bg-white/5 rounded overflow-hidden">
            {([
              { value: '180ms', label: t.stats.coldStart, icon: Zap },
              { value: '40MB', label: t.stats.memory, icon: Cpu },
              { value: String(registry?.handsCount ?? 15), label: t.stats.hands || 'Hands', icon: Box },
              { value: String(registry?.providersCount ?? 50), label: t.stats.providers || 'Providers', icon: Network },
            ] as const).map((stat, i) => (
              <div key={i} className="bg-surface-100 px-6 py-5 flex items-center gap-4">
                <stat.icon className="w-5 h-5 text-cyan-500/60 shrink-0" />
                <div>
                  <div className="text-2xl font-black text-white font-mono">{stat.value}</div>
                  <div className="text-xs text-gray-500 font-medium uppercase tracking-wider">{stat.label}</div>
                </div>
              </div>
            ))}
          </div>
        </FadeIn>
      </div>
    </header>
  )
}

// ─── Architecture ───
const layerIcons: LucideIcon[] = [Globe, Box, Cpu, Layers, Radio]
const layerColors: string[] = ['text-amber-400', 'text-cyan-400', 'text-purple-400', 'text-emerald-400', 'text-rose-400']

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
        <div key={title} className="px-3 py-2 bg-surface-200 border border-white/5">
          <div className="text-sm text-white font-semibold">{title}</div>
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
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.architecture.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.architecture.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{t.architecture.desc}</p>
        </FadeIn>

        <div className="space-y-px">
          {t.architecture.layers.map((layer, i) => {
            const Icon = layerIcons[i]!
            const isOpen = openLayer === i
            return (
              <FadeIn key={i} delay={i * 80}>
                <div className="border border-white/5 bg-surface-100 transition-all">
                  <button
                    onClick={() => setOpenLayer(isOpen ? null : i)}
                    className="w-full flex items-center gap-6 hover:bg-surface-200 px-6 md:px-8 py-6 transition-all text-left"
                  >
                    <div className="w-10 text-right font-mono text-sm text-gray-600 shrink-0">0{i + 1}</div>
                    <div className={cn('shrink-0', layerColors[i])}>
                      <Icon className="w-5 h-5" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="font-bold text-white text-lg">{layer.label}</div>
                      <div className="text-gray-500 text-sm mt-0.5">{layer.desc}</div>
                    </div>
                    <ChevronRight className={cn('w-4 h-4 text-gray-700 transition-transform shrink-0', isOpen && 'rotate-90 text-cyan-500')} />
                  </button>
                  {isOpen && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.3, ease: 'easeOut' }}
                      className="px-6 md:px-8 pb-6 border-t border-white/5"
                    >
                      <div className="pt-4">
                        {i === 0 && (
                          <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 gap-2">
                            {(registry?.channels && registry.channels.length > 0
                              ? registry.channels
                              : []
                            ).map(ch => (
                              <div key={ch.id} className="px-2 py-1.5 bg-surface-200 border border-white/5 text-xs text-gray-400 font-mono text-center truncate">{ch.name}</div>
                            ))}
                          </div>
                        )}
                        {i === 1 && (
                          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-2">
                            {(registry?.hands && registry.hands.length > 0
                              ? registry.hands
                              : []
                            ).map(h => (
                              <div key={h.id} className="px-3 py-2 bg-surface-200 border border-white/5">
                                <div className="text-sm text-white font-semibold">{h.name}</div>
                                <div className="text-[10px] text-gray-600 font-mono uppercase">{h.category}</div>
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

// ─── Hands (Features) — horizontal scroll carousel ───
const categoryColors: Record<string, string> = {
  content: 'text-amber-400 border-amber-400/20',
  data: 'text-cyan-400 border-cyan-400/20',
  productivity: 'text-emerald-400 border-emerald-400/20',
  communication: 'text-purple-400 border-purple-400/20',
  development: 'text-rose-400 border-rose-400/20',
  research: 'text-blue-400 border-blue-400/20',
}

function Hands({ t }: SectionProps) {
  const { data: registry } = useRegistry()
  const lang = useAppStore((s) => s.lang)
  const hands = registry?.hands && registry.hands.length > 0 ? registry.hands : []
  const scrollRef = useRef<HTMLDivElement>(null)

  return (
    <section id="hands" className="py-28 scroll-mt-20">
      <div className="max-w-6xl mx-auto px-6">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.hands.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.hands.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{t.hands.desc}</p>
        </FadeIn>
      </div>

      <div className="max-w-6xl mx-auto px-6">
        <FadeIn>
          <div
            ref={scrollRef}
            className="overflow-x-auto scrollbar-hide -mr-6 pr-6 pb-4"
          >
            <div className="grid grid-rows-2 grid-flow-col gap-3 w-max">
              {hands.map((hand) => {
                const colorClass = categoryColors[hand.category] ?? 'text-gray-400/60'
                return (
                  <div
                    key={hand.id}
                    className="group w-56 bg-surface-100 border border-white/5 hover:border-cyan-500/20 px-4 py-3 transition-all hover:bg-surface-200"
                  >
                    <div className="flex items-center gap-2 mb-1.5">
                      <h3 className="text-sm font-bold text-white truncate">{hand.name.replace(' Hand', '')}</h3>
                      <span className={cn('text-[10px] uppercase tracking-wide shrink-0', colorClass)}>
                        {hand.category}
                      </span>
                    </div>
                    <p className="text-xs text-gray-500 leading-relaxed line-clamp-2">{getLocalizedDesc(hand, lang)}</p>
                  </div>
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
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.performance.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.performance.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{t.performance.desc}</p>
        </FadeIn>

        <FadeIn delay={100}>
          <div className="hidden md:block border border-white/5 overflow-hidden">
            <table className="w-full text-left">
              <thead>
                <tr className="bg-surface-200 text-xs uppercase tracking-widest">
                  <th className="px-6 py-4 font-semibold text-gray-500">{t.performance.metric}</th>
                  <th className="px-6 py-4 font-semibold text-gray-500 text-center border-l border-white/5">{t.performance.others}</th>
                  <th className="px-6 py-4 font-semibold text-cyan-500 text-center border-l border-cyan-500/10 bg-cyan-500/5">LibreFang</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/5">
                {t.performance.rows.map((row, i) => (
                  <tr key={i} className="hover:bg-white/[0.02] transition-colors">
                    <td className="px-6 py-4 text-sm font-medium text-gray-300">{row.metric}</td>
                    <td className="px-6 py-4 text-sm text-center text-gray-500 font-mono border-l border-white/5">{row.others}</td>
                    <td className="px-6 py-4 text-sm text-center text-cyan-400 font-mono font-bold border-l border-cyan-500/10 bg-cyan-500/5">{row.librefang}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <div className="md:hidden space-y-3">
            {t.performance.rows.map((row, i) => (
              <div key={i} className="bg-surface-100 border border-white/5 p-4">
                <div className="text-xs text-gray-500 uppercase tracking-widest mb-3">{row.metric}</div>
                <div className="flex justify-between items-baseline">
                  <div className="text-sm text-gray-500">{t.performance.others}: <span className="font-mono">{row.others}</span></div>
                  <div className="text-lg font-bold font-mono text-cyan-400">{row.librefang}</div>
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
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.workflows.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.workflows.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{t.workflows.desc}</p>
        </FadeIn>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
          {t.workflows.items.map((item, i) => {
            const Icon = workflowIcons[i] || Box
            return (
              <FadeIn key={i} delay={i * 60}>
                <div className="group bg-surface-100 border border-white/5 hover:border-cyan-500/20 p-6 transition-all hover:bg-surface-200">
                  <Icon className="w-5 h-5 text-amber-400/60 group-hover:text-amber-400 transition-colors mb-4" />
                  <h3 className="text-lg font-bold text-white mb-2">{item.title}</h3>
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

// ─── Install ───
function Install({ t }: SectionProps) {
  const [copied, setCopied] = useState(false)
  const cmd = 'curl -fsSL https://librefang.sh/install | sh'

  const copy = () => {
    navigator.clipboard.writeText(cmd)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <section id="install" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-3xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.install.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.install.title}</h2>
          <p className="text-gray-400 text-lg mb-12">{t.install.desc}</p>
        </FadeIn>

        <FadeIn delay={100}>
          <div className="border border-white/10 bg-surface-100 overflow-hidden glow-cyan">
            <div className="flex items-center justify-between px-4 py-2.5 bg-surface-200 border-b border-white/5">
              <div className="flex gap-1.5">
                <div className="w-2.5 h-2.5 rounded-full bg-white/10" />
                <div className="w-2.5 h-2.5 rounded-full bg-white/10" />
                <div className="w-2.5 h-2.5 rounded-full bg-white/10" />
              </div>
              <span className="text-[10px] font-mono text-gray-600 uppercase tracking-widest">{t.install.terminal}</span>
              <button onClick={copy} className="text-gray-500 hover:text-cyan-400 transition-colors p-1" aria-label="Copy">
                {copied ? <Check className="w-3.5 h-3.5 text-cyan-400" /> : <Copy className="w-3.5 h-3.5" />}
              </button>
            </div>
            <div className="p-6 font-mono text-sm md:text-base space-y-4">
              <div className="flex gap-3">
                <span className="text-cyan-500 select-none">$</span>
                <span className="text-gray-200">curl -fsSL https://librefang.sh/install | sh</span>
              </div>
              <div className="flex gap-3">
                <span className="text-cyan-500 select-none">$</span>
                <span className="text-gray-200">librefang init</span>
              </div>
              <div className="flex gap-3">
                <span className="text-cyan-500 select-none">$</span>
                <span className="text-gray-200">librefang start</span>
              </div>
              <div className="text-gray-600 text-xs mt-2">
                <span className="text-amber-500/60">#</span> {t.install.comment}
              </div>
            </div>
          </div>
        </FadeIn>

        <FadeIn delay={200}>
          <div className="grid sm:grid-cols-2 gap-px mt-6 bg-white/5 overflow-hidden">
            <div className="bg-surface-100 p-5">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">{t.install.requires}</div>
              <ul className="space-y-2 text-sm text-gray-400">
                {t.install.reqItems.map((item, i) => (
                  <li key={i} className="flex items-center gap-2"><span className="w-1 h-1 bg-cyan-500 rounded-full" /> {item}</li>
                ))}
              </ul>
            </div>
            <div className="bg-surface-100 p-5">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">{t.install.includes}</div>
              <ul className="space-y-2 text-sm text-gray-400">
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
  return (
    <section id="faq" className="py-28 px-6 scroll-mt-20">
      <div className="max-w-3xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.faq.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-12">{t.faq.title}</h2>
        </FadeIn>

        <div className="space-y-px">
          {t.faq.items.map((item, i) => (
            <FadeIn key={i} delay={i * 60}>
              <details className="group border border-white/5 bg-surface-100 hover:bg-surface-200 transition-colors" open={i === 0}>
                <summary className="flex items-center justify-between px-6 py-5 cursor-pointer select-none list-none">
                  <span className="font-semibold text-white text-sm pr-4">{item.q}</span>
                  <ChevronRight className="w-4 h-4 text-gray-600 group-open:rotate-90 transition-transform shrink-0" />
                </summary>
                <div className="px-6 pb-5 text-sm text-gray-400 leading-relaxed border-t border-white/5 pt-4">
                  {item.a}
                </div>
              </details>
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
]
const communityIcons: LucideIcon[] = [GitPullRequest, CircleDot, MessageSquare]

function Community({ t }: SectionProps) {
  return (
    <section className="py-28 px-6 border-t border-white/5">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.community.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.community.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{t.community.desc}</p>
        </FadeIn>

        <div className="grid md:grid-cols-3 gap-4">
          {t.community.items.map((item, i) => {
            const Icon = communityIcons[i]!
            return (
              <FadeIn key={i} delay={i * 80}>
                <a
                  href={communityHrefs[i]}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="group block bg-surface-100 border border-white/5 hover:border-cyan-500/20 p-6 transition-all"
                >
                  <Icon className="w-5 h-5 text-cyan-500/60 group-hover:text-cyan-400 transition-colors mb-4" />
                  <h3 className="font-bold text-white mb-1">{item.label}</h3>
                  <p className="text-sm text-gray-500">{item.desc}</p>
                  <div className="mt-4 text-cyan-500 text-sm font-semibold flex items-center gap-1 group-hover:gap-2 transition-all">
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
    <section className="py-28 px-6 border-t border-white/5" id="github-stats">
      <div className="max-w-6xl mx-auto">
        <FadeIn>
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{gs.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{gs.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{gs.desc}</p>
        </FadeIn>

        <FadeIn delay={100}>
          <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-7 gap-px bg-white/5 rounded overflow-hidden mb-8">
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
                <div className="text-xl font-black text-white font-mono">
                  {loading ? <span className="inline-block w-10 h-5 bg-gray-700/50 rounded animate-pulse" /> : stat.value}
                </div>
                <div className="text-[10px] text-gray-500 uppercase tracking-widest mt-1">{stat.label}</div>
              </div>
            ))}
          </div>
        </FadeIn>

        {/* Star History Chart */}
        <FadeIn delay={200}>
          <div className="bg-surface-100 border border-white/5 p-6 mb-8">
            <div className="flex items-center justify-between mb-4">
              <span className="text-sm font-bold text-white">{gs.starHistory}</span>
              <a href="https://star-history.com/#librefang/librefang" target="_blank" rel="noopener noreferrer" className="text-xs text-gray-600 hover:text-cyan-400 transition-colors">View Full</a>
            </div>
            <div className="h-36 flex items-end gap-0.5">
              {starHistory.length >= 3 ? (
                Array.from({ length: Math.min(30, chartData.length) }, (_, i) => {
                  const idx = Math.floor((i / Math.min(30, chartData.length)) * chartData.length)
                  const value = chartData[idx] || 0
                  return <div key={i} className="flex-1 bg-cyan-500/30 hover:bg-cyan-500 transition-colors rounded-t min-w-0.5" style={{ height: `${Math.max(4, (value / chartMax) * 100)}%` }} />
                })
              ) : (
                <div className="w-full h-full flex flex-col items-center justify-center text-gray-500">
                  <span className="text-3xl font-black text-cyan-400 font-mono">{stars}</span>
                  <span className="text-xs mt-1">{gs.stars}</span>
                </div>
              )}
            </div>
          </div>
        </FadeIn>

        {/* Star History & Contributors */}
        <FadeIn delay={300}>
          <div className="grid md:grid-cols-2 gap-4 mb-12">
            <a href="https://star-history.com/#librefang/librefang&Date" target="_blank" rel="noopener noreferrer" className="block bg-surface-100 border border-white/5 hover:border-cyan-500/20 p-4 transition-all">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">{gs.starHistory}</div>
              <img src="https://api.star-history.com/svg?repos=librefang/librefang&type=Date&theme=dark" alt="Star History" className="w-full h-auto rounded" loading="lazy" />
            </a>
            <a href="https://github.com/librefang/librefang/graphs/contributors" target="_blank" rel="noopener noreferrer" className="block bg-surface-100 border border-white/5 hover:border-cyan-500/20 p-4 transition-all">
              <div className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-3">Contributors</div>
              <img src="https://contrib.rocks/image?repo=librefang/librefang&anon=0" alt="Contributors" className="w-full h-auto rounded" loading="lazy" />
            </a>
          </div>
        </FadeIn>

        <FadeIn delay={400}>
          <div className="flex flex-col sm:flex-row justify-center gap-3">
            <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="inline-flex items-center justify-center gap-2 px-6 py-3 border border-white/10 hover:border-cyan-500/30 hover:bg-cyan-500/10 text-white font-semibold rounded transition-all">
              <Star className="w-4 h-4" />
              {gs.starUs}
            </a>
            <a href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer" className="inline-flex items-center justify-center gap-2 px-6 py-3 border border-white/10 hover:border-white/20 text-gray-300 font-semibold rounded transition-all hover:bg-white/5">
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
          <div className="text-xs font-mono text-cyan-500 uppercase tracking-widest mb-3">{t.docs.label}</div>
          <h2 className="text-3xl md:text-5xl font-black text-white tracking-tight mb-4">{t.docs.title}</h2>
          <p className="text-gray-400 text-lg max-w-2xl mb-16">{t.docs.desc}</p>
        </FadeIn>

        <div className="grid md:grid-cols-3 gap-4 mb-8">
          {t.docs.categories.map((cat, i) => (
            <FadeIn key={i} delay={i * 80}>
              <div className="bg-surface-100 border border-white/5 hover:border-cyan-500/20 p-6 transition-all">
                <h3 className="font-bold text-white mb-2">{cat.title}</h3>
                <p className="text-sm text-gray-500">{cat.desc}</p>
              </div>
            </FadeIn>
          ))}
        </div>

        <FadeIn delay={300}>
          <div className="text-center">
            <a href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-2 text-cyan-500 font-semibold text-sm hover:text-cyan-400 transition-colors">
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
    <footer className="border-t border-white/5 py-12 px-6">
      <div className="max-w-6xl mx-auto flex flex-col md:flex-row items-center justify-between gap-6">
        <div className="flex items-center gap-2.5">
          <div className="w-6 h-6 rounded bg-cyan-500/10 border border-cyan-500/20 flex items-center justify-center">
            <Terminal className="w-3 h-3 text-cyan-400" />
          </div>
          <span className="text-sm font-semibold text-gray-400">LibreFang</span>
          <span className="text-xs text-gray-600 font-mono">Agent OS</span>
        </div>
        <div className="flex items-center gap-6 text-xs text-gray-600 font-medium">
          <a href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-400 transition-colors">{t.footer.docs}</a>
          <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-400 transition-colors">GitHub</a>
          <a href="https://github.com/librefang/librefang/blob/main/LICENSE" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-400 transition-colors">{t.footer.license}</a>
          <a href="/privacy/" className="hover:text-cyan-400 transition-colors">{t.footer.privacy}</a>
        </div>
        <div className="text-xs text-gray-700">&copy; {new Date().getFullYear()} LibreFang.ai</div>
      </div>
    </footer>
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
    <div className="min-h-screen">
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
      <Docs t={t} />
      <FAQ t={t} />
      <GitHubStats t={t} />
      <Community t={t} />
      <Footer t={t} />
    </div>
  )
}
