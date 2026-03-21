import { useState, useEffect } from 'react'
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'
import { translations, languages } from './i18n'
import { ExternalLink, Globe, ChevronDown, Menu, X, ClipboardCheck, Settings, BadgeCheck, Code, Bug, MessageCircle, Copy, Check, Video, UserSearch, Radar, TrendingUp, Activity, Filter, Network, RefreshCw, Shield, Library, Monitor, SearchCheck, Rss, Star, GitFork, CircleDot, GitPullRequest } from 'lucide-react'

const queryClient = new QueryClient()

const comparisonData = [
  { metric: 'Cold Start', openclaw: '2.5s+', zeroclaw: '4s+', librefang: '180ms' },
  { metric: 'Idle Memory', openclaw: '180MB+', zeroclaw: '250MB+', librefang: '40MB' },
  { metric: 'Binary Size', openclaw: '100MB+', zeroclaw: '200MB+', librefang: '32MB' },
  { metric: 'Security Layers', openclaw: '3', zeroclaw: '2', librefang: '16' },
  { metric: 'Channel Adapters', openclaw: '15', zeroclaw: '8', librefang: '40' },
  { metric: 'Built-in Hands', openclaw: '0', zeroclaw: '0', librefang: '7' },
]

function MaterialIcon({ name, className = '' }) {
  const icons = {
    open_in_new: ExternalLink,
    language: Globe,
    expand_more: ChevronDown,
    menu: Menu,
    close: X,
    checklist: ClipboardCheck,
    settings_suggest: Settings,
    verified: BadgeCheck,
    code: Code,
    bug_report: Bug,
    forum: MessageCircle,
    content_copy: Copy,
    check: Check,
    movie_edit: Video,
    person_search: UserSearch,
    radar: Radar,
    trending_up: TrendingUp,
    monitor: Monitor,
    filter_alt: Filter,
    hub: Network,
    sync_alt: RefreshCw,
    security: Shield,
    video_library: Library,
    monitoring: Activity,
    manage_search: SearchCheck,
    rss_feed: Rss,
  }
  const IconComponent = icons[name]
  if (!IconComponent) return null
  return <IconComponent className={className} />
}

function getCurrentLang() {
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

function Header({ t }) {
  const currentLang = getCurrentLang()
  const currentLangName = languages.find(l => l.code === currentLang)?.name || 'English'
  const [langOpen, setLangOpen] = useState(false)
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)

  useEffect(() => {
    const handleEscape = (e) => {
      if (e.key === 'Escape') {
        setMobileMenuOpen(false)
        setLangOpen(false)
      }
    }
    const handleClickOutside = (e) => {
      if (langOpen && !e.target.closest('[data-lang-menu]')) {
        setLangOpen(false)
      }
    }
    document.addEventListener('keydown', handleEscape)
    document.addEventListener('click', handleClickOutside)
    return () => {
      document.removeEventListener('keydown', handleEscape)
      document.removeEventListener('click', handleClickOutside)
    }
  }, [langOpen])

  return (
    <nav className="fixed top-0 left-0 right-0 z-40 px-4 md:px-12 py-4 glass-effect" role="navigation" aria-label="Main navigation">
      <div className="max-w-7xl mx-auto flex items-center justify-between">
        <a href="/" className="flex items-center gap-3">
          <div className="flex items-center justify-center p-1">
            <img src="/logo.png" alt="LibreFang Logo" width="32" height="32" className="rounded-md" loading="lazy" decoding="async" />
          </div>
          <span className="font-extrabold text-2xl tracking-tight text-white">LibreFang</span>
        </a>
        <div className="hidden md:flex items-center gap-8 text-sm font-semibold text-gray-400">
          <a className="hover:text-primary transition-colors" href="#features">{t.nav.features}</a>
          <a className="hover:text-primary transition-colors" href="#comparison">{t.nav.comparison}</a>
          <a className="flex items-center gap-1 hover:text-primary transition-colors" href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer">
            <span>{t.nav.docs}</span>
            <MaterialIcon name="open_in_new" className="w-3.5 h-3.5" />
          </a>
          <a className="flex items-center gap-1 hover:text-primary transition-colors" href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer">
            <span>{t.nav.github}</span>
            <MaterialIcon name="open_in_new" className="w-3.5 h-3.5" />
          </a>
        </div>
        <div className="flex items-center gap-4">
          <div className="relative" data-lang-menu>
            <button className="flex items-center gap-2 p-2 text-gray-400 hover:text-primary transition-colors rounded-lg hover:bg-white/10" aria-label="Switch language" aria-expanded={langOpen} onClick={() => setLangOpen(!langOpen)}>
              <MaterialIcon name="language" className="w-5 h-5" />
              <span className="hidden sm:inline text-sm font-bold">{currentLangName}</span>
              <MaterialIcon name="expand_more" className={`w-4 h-4 transition-transform ${langOpen ? 'rotate-180' : ''}`} />
            </button>
            <div className={`absolute right-0 mt-2 w-40 bg-[#161b22] border border-gray-700/50 rounded-xl shadow-2xl ${langOpen ? 'opacity-100 visible translate-y-0' : 'opacity-0 invisible -translate-y-2'} transition-all duration-200 z-50`} role="menu">
              {languages.map((lang) => (
                <a key={lang.code} href={lang.url} role="menuitem" className={`block px-5 py-3 text-sm font-bold transition-colors ${lang.code === currentLang ? 'bg-primary/10 text-primary' : 'text-gray-400 hover:bg-white/10 hover:text-gray-100'} first:rounded-t-xl last:rounded-b-xl`}>
                  {lang.name}
                </a>
              ))}
            </div>
          </div>
          <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="hidden sm:flex items-center justify-center size-10 rounded-full bg-white/10 text-gray-200 hover:bg-primary hover:text-white transition-all" aria-label="Star on GitHub">
            <svg fill="currentColor" height="20" width="20" viewBox="0 0 256 256">
              <path d="M208.31,75.68A59.78,59.78,0,0,0,202.93,28,8,8,0,0,0,196,24a59.75,59.75,0,0,0-48,24H124A59.75,59.75,0,0,0,76,24a8,8,0,0,0-6.93,4,59.78,59.78,0,0,0-5.38,47.68A58.14,58.14,0,0,0,56,104v8a56.06,56.06,0,0,0,48.44,55.47A39.8,39.8,0,0,0,96,192v8H72a24,24,0,0,1-24-24A40,40,0,0,0,8,136a8,8,0,0,0,0,16,24,24,0,0,1,24,24,40,40,0,0,0,40,40H96v16a8,8,0,0,0,16,0V192a24,24,0,0,1,48,0v40a8,8,0,0,0,16,0V192a39.8,39.8,0,0,0-8.44-24.53A56.06,56.06,0,0,0,216,112v-8A58.14,58.14,0,0,0,208.31,75.68Z"></path>
            </svg>
          </a>
          <button className="md:hidden text-gray-300" onClick={() => setMobileMenuOpen(!mobileMenuOpen)} onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setMobileMenuOpen(!mobileMenuOpen) }} aria-label={mobileMenuOpen ? 'Close menu' : 'Open menu'} aria-expanded={mobileMenuOpen}>
            <MaterialIcon name={mobileMenuOpen ? 'close' : 'menu'} className="w-6 h-6" />
          </button>
        </div>
      </div>
      {mobileMenuOpen && (
        <div className="md:hidden mt-4 px-2 pb-4 space-y-1 border-t border-gray-700/30 pt-4">
          <a href="#features" onClick={() => setMobileMenuOpen(false)} className="block py-3 px-4 text-gray-300 hover:text-primary hover:bg-white/5 rounded-lg transition-colors font-medium">{t.nav.features}</a>
          <a href="#comparison" onClick={() => setMobileMenuOpen(false)} className="block py-3 px-4 text-gray-300 hover:text-primary hover:bg-white/5 rounded-lg transition-colors font-medium">{t.nav.comparison}</a>
          <a href="#install" onClick={() => setMobileMenuOpen(false)} className="block py-3 px-4 text-gray-300 hover:text-primary hover:bg-white/5 rounded-lg transition-colors font-medium">{t.install?.singleBinary || 'Install'}</a>
          <a href="#faq" onClick={() => setMobileMenuOpen(false)} className="block py-3 px-4 text-gray-300 hover:text-primary hover:bg-white/5 rounded-lg transition-colors font-medium">{t.faq?.title || 'FAQ'}</a>
          <a href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 py-3 px-4 text-gray-300 hover:text-primary hover:bg-white/5 rounded-lg transition-colors font-medium">
            {t.nav.docs}
            <MaterialIcon name="open_in_new" className="w-3.5 h-3.5" />
          </a>
          <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 py-3 px-4 text-gray-300 hover:text-primary hover:bg-white/5 rounded-lg transition-colors font-medium">
            {t.nav.github}
            <MaterialIcon name="open_in_new" className="w-3.5 h-3.5" />
          </a>
        </div>
      )}
    </nav>
  )
}

function Hero({ t }) {
  const { data: release } = useQuery({
    queryKey: ['latestRelease'],
    queryFn: async () => {
      const res = await fetch('https://api.github.com/repos/librefang/librefang/releases/latest', {
        headers: { 'Accept': 'application/vnd.github.v3+json' },
      })
      if (!res.ok) throw new Error('Failed to fetch')
      return res.json()
    },
    staleTime: 1000 * 60 * 60,
    retry: 0,
  })

  const version = release?.tag_name || ''

  return (
    <header className="relative px-6 pt-32 pb-24 md:pt-48 md:pb-40 overflow-hidden">
      <div className="absolute inset-0 opacity-30 pointer-events-none">
        <div className="absolute top-[-10%] left-1/2 -translate-x-1/2 w-[800px] h-[800px] bg-primary rounded-full blur-[180px] opacity-20"></div>
        <div className="absolute bottom-[-10%] right-[-10%] w-[500px] h-[500px] bg-cyan-600 rounded-full blur-[140px] opacity-10"></div>
      </div>
      <div className="relative z-10 max-w-7xl mx-auto grid grid-cols-1 lg:grid-cols-2 gap-16 items-center">
        <div className="text-center lg:text-left space-y-10">
          <div className="inline-flex items-center gap-2 bg-primary/10 border border-primary/20 px-5 py-2 rounded-full text-sm font-bold text-primary">
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-primary opacity-75"></span>
              <span className="relative inline-flex rounded-full h-2 w-2 bg-primary"></span>
            </span>
            <span className="min-w-[50px] inline-block">{version || ''}</span> · Rust-Powered · Open Source
          </div>
          <h1 className="text-6xl md:text-8xl font-extrabold tracking-tight leading-[1.05] bg-clip-text text-transparent bg-gradient-to-br from-white via-gray-100 to-primary flex items-center justify-center lg:justify-start gap-4">
            <img src="/fox-mascot.png" alt="LibreFang Mascot" className="w-16 h-16 md:w-20 md:h-20 rounded-xl" />
            LibreFang
          </h1>
          <p className="text-gray-400 text-xl md:text-2xl font-light leading-relaxed max-w-xl mx-auto lg:mx-0">
            {t.hero.subtitle}
          </p>
          <div className="flex flex-col sm:flex-row gap-5 justify-center lg:justify-start pt-4">
            <a href="#install" className="bg-primary hover:bg-primary-dark text-white font-extrabold py-5 px-10 rounded-full shadow-2xl shadow-primary/30 transition-all hover:scale-105 active:scale-95">
              {t.hero.getStarted}
            </a>
            <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="bg-white/10 hover:bg-white/15 text-gray-100 font-bold py-5 px-10 rounded-full border border-gray-600/50 backdrop-blur-md transition-all hover:scale-105 active:scale-95 flex items-center gap-2 justify-center">
              <svg fill="currentColor" height="20" width="20" viewBox="0 0 256 256">
                <path d="M208.31,75.68A59.78,59.78,0,0,0,202.93,28,8,8,0,0,0,196,24a59.75,59.75,0,0,0-48,24H124A59.75,59.75,0,0,0,76,24a8,8,0,0,0-6.93,4,59.78,59.78,0,0,0-5.38,47.68A58.14,58.14,0,0,0,56,104v8a56.06,56.06,0,0,0,48.44,55.47A39.8,39.8,0,0,0,96,192v8H72a24,24,0,0,1-24-24A40,40,0,0,0,8,136a8,8,0,0,0,0,16,24,24,0,0,1,24,24,40,40,0,0,0,40,40H96v16a8,8,0,0,0,16,0V192a24,24,0,0,1,48,0v40a8,8,0,0,0,16,0V192a39.8,39.8,0,0,0-8.44-24.53A56.06,56.06,0,0,0,216,112v-8A58.14,58.14,0,0,0,208.31,75.68Z"></path>
              </svg>
              {t.hero.githubRepo}
            </a>
          </div>
        </div>
        <div className="relative">
          <div className="rounded-[2.5rem] border border-gray-700/50 bg-[#0d1117] p-4 shadow-3xl backdrop-blur-sm overflow-hidden">
            <div className="w-full aspect-video rounded-[2rem] overflow-hidden relative">
              <img className="w-full h-full object-cover opacity-80" src="/librefang-vs-claws.png" alt="LibreFang Agent OS Interface" width="1920" height="1080" loading="eager" />
              <div className="absolute inset-0 bg-gradient-to-t from-[#080c10]/90 via-transparent to-transparent pointer-events-none"></div>
              <div className="absolute bottom-8 left-8 flex items-center gap-3 pointer-events-none">
                <div className="h-2 w-2 rounded-full bg-primary animate-pulse"></div>
                <span className="text-xs font-black uppercase tracking-[0.3em] text-primary">{t.hero.systemPreview}</span>
              </div>
            </div>
          </div>
          <div className="absolute -top-10 -right-10 w-40 h-40 bg-primary/10 rounded-full blur-3xl -z-10"></div>
        </div>
      </div>
    </header>
  )
}

function Stats({ t }) {
  return (
    <section className="px-6 py-20">
      <div className="max-w-5xl mx-auto">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-8">
          {[
            { value: '180ms', label: t.stats.coldStart },
            { value: '40MB', label: t.stats.memory },
            { value: '40', label: t.stats.channels },
            { value: '16', label: t.stats.security },
          ].map((stat, i) => (
            <div key={i} className="text-center space-y-3">
              <div className="text-5xl md:text-6xl font-extrabold text-primary">{stat.value}</div>
              <div className="text-gray-400 font-bold text-sm uppercase tracking-wider">{stat.label}</div>
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}

function Features({ t }) {
  const featureIcons = ['movie_edit', 'person_search', 'radar', 'trending_up', 'manage_search', 'rss_feed']
  return (
    <section className="px-6 py-32 bg-[#0d1117] rounded-section mx-4 md:mx-12 scroll-mt-20" id="features">
      <div className="max-w-7xl mx-auto space-y-20">
        <header className="text-center space-y-6">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-white">{t.features.feature1Title}</h2>
          <p className="text-gray-400 text-lg md:text-xl max-w-3xl mx-auto">{t.features.feature1Desc}</p>
          <div className="h-1.5 w-24 bg-gradient-to-r from-primary to-cyan-400 mx-auto rounded-full"></div>
        </header>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8">
          {t.featureCards.map((feature, i) => (
            <article key={i} className="bg-[#161b22] p-10 rounded-[2.5rem] border border-gray-700/50 hover:border-primary/50 transition-all duration-500 group hover:-translate-y-2 shadow-sm hover:shadow-primary/5 hover:shadow-2xl">
              <div className="size-16 bg-primary/10 rounded-2xl flex items-center justify-center text-primary mb-8 group-hover:bg-primary group-hover:text-white transition-all duration-500">
                <MaterialIcon name={featureIcons[i]} className="w-8 h-8" />
              </div>
              <h3 className="text-2xl font-extrabold mb-5 text-white">{feature.title}</h3>
              <p className="text-gray-400 text-lg leading-relaxed">{feature.description}</p>
            </article>
          ))}
        </div>
      </div>
    </section>
  )
}

function Comparison({ t }) {
  return (
    <section className="px-6 py-32 scroll-mt-20" id="comparison">
      <div className="max-w-6xl mx-auto">
        <header className="text-center mb-20">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight mb-6 text-white">{t.comparison.librefangVs}</h2>
          <p className="text-gray-400 text-xl">{t.comparison.subtitle2}</p>
        </header>
        <div className="hidden md:block overflow-hidden rounded-[2rem] border border-gray-700/50 bg-white/5 backdrop-blur-xl">
          <table className="w-full text-left">
            <thead className="bg-gray-800/80 text-gray-300">
              <tr>
                <th scope="col" className="p-8 font-bold text-lg">{t.comparison.metric}</th>
                <th scope="col" className="p-8 font-bold text-lg text-center border-l border-gray-700/50">{t.comparison.openclaw}</th>
                <th scope="col" className="p-8 font-bold text-lg text-center border-l border-gray-700/50">{t.comparison.zeroclaw}</th>
                <th scope="col" className="p-8 font-bold text-lg text-center text-primary border-l border-gray-700/50 bg-primary/5">{t.comparison.librefang}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-700/50">
              {comparisonData.map((row, i) => (
                <tr key={i} className="hover:bg-white/5 transition-colors">
                  <th scope="row" className="p-8 text-gray-100 font-semibold text-base">{row.metric}</th>
                  <td className="p-8 text-center text-gray-400 border-l border-gray-700/50">{row.openclaw}</td>
                  <td className="p-8 text-center text-gray-400 border-l border-gray-700/50">{row.zeroclaw}</td>
                  <td className="p-8 text-center text-primary font-bold border-l border-gray-700/50 bg-primary/5">{row.librefang}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {/* Mobile: show all comparison data */}
        <div className="md:hidden space-y-4">
          {comparisonData.map((row, i) => (
            <article key={i} className="bg-white/5 rounded-2xl border border-gray-700/50 p-6">
              <h3 className="text-sm font-black uppercase tracking-widest text-primary mb-4">{row.metric}</h3>
              <dl className="space-y-3">
                <div className="flex justify-between items-center py-2 border-b border-gray-700/30">
                  <dt className="text-gray-500 text-sm">{t.comparison.openclaw}</dt>
                  <dd className="text-gray-400 font-medium">{row.openclaw}</dd>
                </div>
                <div className="flex justify-between items-center py-2 border-b border-gray-700/30">
                  <dt className="text-gray-500 text-sm">{t.comparison.zeroclaw}</dt>
                  <dd className="text-gray-400 font-medium">{row.zeroclaw}</dd>
                </div>
                <div className="flex justify-between items-center py-2">
                  <dt className="text-white font-bold text-sm">LibreFang</dt>
                  <dd className="text-primary font-black text-lg">{row.librefang}</dd>
                </div>
              </dl>
            </article>
          ))}
        </div>
      </div>
    </section>
  )
}

function Workflows({ t }) {
  const workflowIcons = ['video_library', 'filter_alt', 'monitoring', 'hub', 'sync_alt', 'security']
  return (
    <section className="px-6 py-32 scroll-mt-20" id="workflows">
      <div className="max-w-7xl mx-auto space-y-20">
        <header className="text-center space-y-6">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-white">{t.workflows.workflow1Title}</h2>
          <p className="text-gray-400 text-xl max-w-3xl mx-auto">{t.workflows.workflow1Desc}</p>
        </header>
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-8">
          {t.workflowCards.map((workflow, i) => (
            <article key={i} className="bg-white/5 rounded-[2.5rem] border border-gray-700/50 p-8 space-y-6 hover:border-primary/50 transition-all group hover:-translate-y-1">
              <div className="size-16 bg-primary/10 rounded-2xl flex items-center justify-center group-hover:bg-primary transition-colors">
                <MaterialIcon name={workflowIcons[i]} className="w-8 h-8 text-primary group-hover:text-white" />
              </div>
              <h3 className="text-2xl font-extrabold text-white">{workflow.title}</h3>
              <p className="text-gray-400 text-lg leading-relaxed">{workflow.description}</p>
            </article>
          ))}
        </div>
      </div>
    </section>
  )
}

function Install({ t }) {
  const [copied, setCopied] = useState(false)
  const copyCommand = () => {
    navigator.clipboard.writeText('curl -fsSL https://librefang.sh/install | sh')
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <section className="px-6 py-32 bg-[#0d1117] rounded-section mx-4 md:mx-12 scroll-mt-20" id="install">
      <div className="max-w-5xl mx-auto space-y-16">
        <header className="text-center space-y-6">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-white">{t.install.singleBinary}</h2>
          <p className="text-gray-400 text-xl">{t.install.singleBinaryDesc}</p>
        </header>
        <div className="rounded-[2rem] overflow-hidden shadow-3xl bg-[#080c10] border border-gray-700/50">
          <div className="h-12 flex items-center px-6 justify-between bg-gray-800/30">
            <div className="flex gap-2">
              <div className="size-3 rounded-full bg-red-500/60"></div>
              <div className="size-3 rounded-full bg-yellow-500/60"></div>
              <div className="size-3 rounded-full bg-green-500/60"></div>
            </div>
            <span className="text-xs uppercase tracking-[0.2em] font-bold text-white/50">{t.install.bashInstall || 'bash — Quick Install'}</span>
            <div className="w-12"></div>
          </div>
          <div className="p-6 md:p-10 font-mono text-sm md:text-base lg:text-xl leading-relaxed relative">
            <div className="flex items-start gap-3 md:gap-4 overflow-x-auto">
              <span className="text-primary select-none font-bold shrink-0">$</span>
              <code className="flex-1 min-w-0 whitespace-nowrap md:whitespace-normal">
                <span className="text-primary">curl</span>
                <span className="text-white"> -fsSL https://librefang.sh/install</span>
                <span className="text-gray-500"> |</span>
                <span className="text-primary"> sh</span>
              </code>
            </div>
            <button onClick={copyCommand} className="absolute top-6 right-6 md:top-10 md:right-10 text-primary hover:text-white transition-colors p-2.5 md:p-3 bg-primary/10 rounded-xl hover:bg-primary" aria-label="Copy installation command">
              <MaterialIcon name={copied ? 'check' : 'content_copy'} className="w-5 h-5" />
            </button>
          </div>
        </div>
        <div className="grid md:grid-cols-2 gap-8">
          <article className="bg-white/5 rounded-[2rem] border border-gray-700/50 p-8 space-y-6 hover:border-gray-600/50 transition-colors">
            <div className="flex items-center gap-4">
              <div className="size-14 bg-primary/10 rounded-xl flex items-center justify-center">
                <MaterialIcon name="checklist" className="text-primary w-7 h-7" />
              </div>
              <h3 className="text-2xl font-extrabold text-white">{t.install.requirements}</h3>
            </div>
            <ul className="space-y-3 text-gray-400">
              <li className="flex items-center gap-3"><span className="text-primary">•</span> {t.install.platforms}</li>
              <li className="flex items-center gap-3"><span className="text-primary">•</span> 64MB RAM minimum (256MB recommended)</li>
              <li className="flex items-center gap-3"><span className="text-primary">•</span> x86_64 or ARM64 architecture</li>
              <li className="flex items-center gap-3"><span className="text-primary">•</span> LLM API Key (12 providers supported)</li>
            </ul>
          </article>
          <article className="bg-white/5 rounded-[2rem] border border-gray-700/50 p-8 space-y-6 hover:border-gray-600/50 transition-colors">
            <div className="flex items-center gap-4">
              <div className="size-14 bg-primary/10 rounded-xl flex items-center justify-center">
                <MaterialIcon name="settings_suggest" className="text-primary w-7 h-7" />
              </div>
              <h3 className="text-2xl font-extrabold text-white">{t.install.whatYouGet}</h3>
            </div>
            <ul className="space-y-3 text-gray-400">
              <li className="flex items-center gap-3"><span className="text-primary">•</span> 7 built-in autonomous Hands</li>
              <li className="flex items-center gap-3"><span className="text-primary">•</span> 10 workflow orchestration templates</li>
              <li className="flex items-center gap-3"><span className="text-primary">•</span> 40 channel adapters (incl. Feishu & DingTalk)</li>
              <li className="flex items-center gap-3"><span className="text-primary">•</span> {t.install.desktopApp}</li>
            </ul>
          </article>
        </div>
        <div className="bg-white/5 rounded-[2rem] border border-gray-700/50 p-10 space-y-8">
          <h3 className="text-2xl font-extrabold text-center text-white">{t.install.threeSteps}</h3>
          <ol className="space-y-6">
            <li className="flex gap-6 items-start">
              <div className="size-10 rounded-full bg-primary/20 text-primary flex items-center justify-center font-black text-lg flex-shrink-0">1</div>
              <div className="flex-1 pt-1">
                <h4 className="font-bold text-lg mb-2 text-white">{t.install.install}</h4>
                <pre className="bg-[#080c10] rounded-xl p-4 text-sm font-mono overflow-x-auto max-w-full border border-gray-700/30"><code><span className="text-primary">curl</span> <span className="text-white">-fsSL https://librefang.sh/install | sh</span><br/><span className="text-gray-500"># Windows PowerShell:</span><br/><span className="text-primary">irm</span> <span className="text-white">https://librefang.sh/install.ps1 | iex</span></code></pre>
              </div>
            </li>
            <li className="flex gap-6 items-start">
              <div className="size-10 rounded-full bg-primary/20 text-primary flex items-center justify-center font-black text-lg flex-shrink-0">2</div>
              <div className="flex-1 pt-1">
                <h4 className="font-bold text-lg mb-2 text-white">{t.install.initialize}</h4>
                <pre className="bg-[#080c10] rounded-xl p-4 text-sm font-mono overflow-x-auto max-w-full border border-gray-700/30"><code><span className="text-primary">librefang init</span><br/><span className="text-gray-500"># Configure your LLM provider and channel tokens</span></code></pre>
                <p className="text-gray-400 mt-3 text-sm">{t.install.initializeDesc}</p>
              </div>
            </li>
            <li className="flex gap-6 items-start">
              <div className="size-10 rounded-full bg-primary/20 text-primary flex items-center justify-center font-black text-lg flex-shrink-0">3</div>
              <div className="flex-1 pt-1">
                <h4 className="font-bold text-lg mb-2 text-white">{t.install.startAgents}</h4>
                <pre className="bg-[#080c10] rounded-xl p-4 text-sm font-mono overflow-x-auto max-w-full border border-gray-700/30"><code><span className="text-primary">librefang start</span><br/><span className="text-gray-500"># Migrate from OpenClaw:</span><br/><span className="text-primary">librefang migrate</span> <span className="text-white">--from openclaw</span></code></pre>
                <p className="text-gray-400 mt-3 text-sm">{t.install.startAgentsDesc}</p>
              </div>
            </li>
          </ol>
        </div>
        <div className="flex flex-wrap justify-center gap-10 text-sm font-bold text-gray-400">
          {['macOS', 'Linux', 'Windows', 'Raspberry Pi', 'VPS / Cloud'].map((platform, i) => (
            <div key={i} className="flex items-center gap-3">
              <MaterialIcon name="verified" className="text-primary w-5 h-5" />
              {platform}
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}

function FAQ({ t }) {
  return (
    <section className="px-6 py-32 scroll-mt-20" id="faq">
      <div className="max-w-3xl mx-auto space-y-16">
        <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-center text-white">{t.faq.title}</h2>
        <div className="space-y-4">
          {t.faq.items.map((faq, i) => (
            <details key={i} className="group bg-white/5 rounded-2xl border border-gray-700/50 overflow-hidden transition-all duration-300 hover:border-gray-600/50" open={i === 0}>
              <summary className="flex items-center justify-between p-7 cursor-pointer select-none list-none">
                <h3 className="font-extrabold text-white text-lg pr-4">{faq.question}</h3>
                <div className="bg-primary/10 p-2 rounded-full text-primary group-open:rotate-180 transition-transform shrink-0">
                  <MaterialIcon name="expand_more" className="w-5 h-5" />
                </div>
              </summary>
              <div className="px-7 pb-7 text-gray-400 text-lg leading-relaxed pt-2 border-t border-gray-700/30">
                {faq.answer}
              </div>
            </details>
          ))}
        </div>
      </div>
    </section>
  )
}

function StatCard({ icon, value, label, isLoading }) {
  return (
    <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/30 transition-colors">
      <div className="flex justify-center mb-2 text-primary/60">
        {icon}
      </div>
      <div className="text-2xl md:text-3xl font-black text-primary mb-1">
        {isLoading ? (
          <span className="inline-block w-12 h-7 bg-gray-700/50 rounded animate-pulse"></span>
        ) : value}
      </div>
      <div className="text-gray-400 font-semibold text-xs md:text-sm">{label}</div>
    </div>
  )
}

function formatNumber(num) {
  if (num === null || num === undefined) return '-'
  if (num >= 1000) return `${(num / 1000).toFixed(1)}k`
  return num
}

function GitHubStats({ t }) {
  const { data: githubData, isLoading: githubLoading, isError: githubError } = useQuery({
    queryKey: ['githubStats'],
    queryFn: async () => {
      try {
        const res = await fetch('https://stats.librefang.ai/api/github')
        if (!res.ok) throw new Error('Failed to fetch')
        return res.json()
      } catch {
        return { stars: 0, forks: 0, issues: 0, prs: 0, lastUpdate: '', downloads: 0, starHistory: [] }
      }
    },
    staleTime: 1000 * 60 * 30,
    retry: 1,
  })

  const { data: docsData, isLoading: docsLoading } = useQuery({
    queryKey: ['docsVisits'],
    queryFn: async () => {
      try {
        const res = await fetch('https://counter.librefang.ai/api')
        if (!res.ok) return { total: 0 }
        return res.json()
      } catch {
        return { total: 0 }
      }
    },
    staleTime: 1000 * 60 * 5,
    retry: 0,
  })

  const isLoading = githubLoading || docsLoading
  const docsVisits = docsData?.total ?? 0
  const stars = githubError ? null : (githubData?.stars ?? 0)
  const forks = githubData?.forks ?? 0
  const issues = githubData?.issues ?? 0
  const prs = githubData?.prs ?? 0
  const downloads = githubData?.downloads ?? 0
  const lastUpdate = githubData?.lastUpdate ? new Date(githubData.lastUpdate).toLocaleDateString() : ''

  const starHistoryData = githubData?.starHistory || []
  const starHistory = starHistoryData.length > 0
    ? starHistoryData.map(d => d.stars)
    : (stars > 0 ? [stars] : [0])
  const forksHistory = starHistoryData.length > 0
    ? starHistoryData.map(d => d.forks)
    : (forks > 0 ? [forks] : [0])
  const issuesHistory = starHistoryData.length > 0
    ? starHistoryData.map(d => d.issues)
    : (issues > 0 ? [issues] : [0])
  const prsHistory = starHistoryData.length > 0
    ? starHistoryData.map(d => d.prs ?? 0)
    : (prs > 0 ? [prs] : [0])

  const [historyTab, setHistoryTab] = useState('stars')
  const currentHistory = historyTab === 'stars' ? starHistory : historyTab === 'forks' ? forksHistory : historyTab === 'issues' ? issuesHistory : prsHistory
  const currentMax = Math.max(...currentHistory, 1)
  const currentValue = historyTab === 'stars' ? stars : historyTab === 'forks' ? forks : historyTab === 'issues' ? issues : prs

  return (
    <section className="px-6 py-24 border-t border-gray-700/50 scroll-mt-20" id="community">
      <div className="max-w-7xl mx-auto">
        <h2 className="text-4xl md:text-5xl font-extrabold text-center mb-4">
          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white to-primary">{t.githubStats?.title || 'Join Our Community'}</span>
        </h2>
        <p className="text-gray-400 text-center text-xl mb-16 max-w-2xl mx-auto">
          {t.githubStats?.subtitle || 'Help us build the future of autonomous AI agents'}
        </p>
        <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-7 gap-4">
          <StatCard icon={<Star className="w-4 h-4" />} value={formatNumber(stars)} label={t.githubStats?.stars || 'Stars'} isLoading={isLoading} />
          <StatCard icon={<GitFork className="w-4 h-4" />} value={formatNumber(forks)} label={t.githubStats?.forks || 'Forks'} isLoading={isLoading} />
          <StatCard icon={<svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24"><path d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/></svg>} value={formatNumber(downloads)} label={t.githubStats?.downloads || 'Downloads'} isLoading={isLoading} />
          <StatCard icon={<svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24"><path d="M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zM12 17c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z"/></svg>} value={formatNumber(docsVisits)} label={t.githubStats?.docsVisits || 'Docs Visits'} isLoading={isLoading} />
          <StatCard icon={<CircleDot className="w-4 h-4" />} value={formatNumber(issues)} label={t.githubStats?.issues || 'Issues'} isLoading={isLoading} />
          <StatCard icon={<GitPullRequest className="w-4 h-4" />} value={formatNumber(prs)} label={t.githubStats?.prs || 'PRs'} isLoading={isLoading} />
          <StatCard icon={<svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24"><path d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z"/></svg>} value={lastUpdate || '-'} label={t.githubStats?.lastUpdate || 'Last Update'} isLoading={isLoading} />
        </div>

        {/* History Chart */}
        <div className="mt-12 p-6 rounded-2xl bg-white/5 border border-gray-700/30">
          <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between mb-6 gap-4">
            <h3 className="text-lg font-bold text-white">{t.githubStats?.starHistory || 'History'}</h3>
            <div className="flex items-center gap-3">
              <div className="flex gap-1.5 bg-white/5 rounded-full p-1">
                {['stars', 'forks', 'issues', 'prs'].map(tab => (
                  <button
                    key={tab}
                    onClick={() => setHistoryTab(tab)}
                    className={`px-3 py-1.5 rounded-full text-xs font-semibold transition-all ${
                      historyTab === tab
                        ? 'bg-primary text-white shadow-sm'
                        : 'text-gray-400 hover:text-gray-200'
                    }`}
                  >
                    {tab === 'stars' ? (t.githubStats?.stars || 'Stars') : tab === 'forks' ? (t.githubStats?.forks || 'Forks') : tab === 'issues' ? (t.githubStats?.issues || 'Issues') : (t.githubStats?.prs || 'PRs')}
                  </button>
                ))}
              </div>
              <a href="https://star-history.com/#librefang/librefang" target="_blank" rel="noopener noreferrer" className="text-xs text-gray-500 hover:text-primary transition-colors hidden sm:inline">View Full Chart</a>
            </div>
          </div>
          <div className="h-48 flex items-end gap-1">
            {currentHistory.length > 0 ? (
              Array.from({ length: Math.min(24, currentHistory.length || 12) }, (_, i) => {
                const idx = Math.floor((i / Math.min(24, currentHistory.length || 12)) * currentHistory.length)
                const value = currentHistory[idx] || 0
                return (
                  <div key={i} className="flex-1 bg-primary/40 hover:bg-primary transition-colors rounded-t min-w-1" style={{ height: `${Math.max(4, (value / currentMax) * 100)}%` }} title={`${value} ${historyTab}`}></div>
                )
              })
            ) : (
              <div className="w-full h-32 flex items-center justify-center text-gray-500 text-sm">No data yet</div>
            )}
          </div>
          <div className="flex justify-between mt-3 text-xs text-gray-500">
            <span>12 months ago</span>
            <span className="text-primary font-semibold">Now ({currentValue ?? '-'})</span>
          </div>
        </div>

        {/* Star History & Contributors Images */}
        <div className="grid md:grid-cols-2 gap-6 mt-8">
          <a href="https://star-history.com/#librefang/librefang&Date" target="_blank" rel="noopener noreferrer" className="block rounded-2xl overflow-hidden border border-gray-700/30 hover:border-primary/30 transition-colors bg-white/5 p-4">
            <h4 className="text-sm font-bold text-gray-400 mb-3">{t.githubStats?.starHistory || 'Star History'}</h4>
            <img
              src="/assets/star-history.svg"
              alt="LibreFang Star History Chart"
              className="w-full h-auto rounded-lg"
              loading="lazy"
              onError={(e) => { e.target.style.display = 'none' }}
            />
          </a>
          <a href="https://github.com/librefang/librefang/graphs/contributors" target="_blank" rel="noopener noreferrer" className="block rounded-2xl overflow-hidden border border-gray-700/30 hover:border-primary/30 transition-colors bg-white/5 p-4">
            <h4 className="text-sm font-bold text-gray-400 mb-3">{t.contributing?.title || 'Contributors'}</h4>
            <img
              src="/assets/contributors.svg"
              alt="LibreFang Contributors"
              className="w-full h-auto rounded-lg"
              loading="lazy"
              onError={(e) => { e.target.style.display = 'none' }}
            />
          </a>
        </div>

        <div className="flex flex-col sm:flex-row justify-center gap-4 mt-12">
          <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="bg-white/10 hover:bg-primary text-white font-bold py-4 px-8 rounded-full border border-gray-600/50 transition-all hover:scale-105 hover:border-primary flex items-center gap-3 justify-center">
            <Star className="w-5 h-5" />
            {t.githubStats?.starUs || 'Star Us'}
          </a>
          <a href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer" className="bg-white/5 hover:bg-white/10 text-gray-100 font-bold py-4 px-8 rounded-full border border-gray-600/30 transition-all hover:scale-105 flex items-center gap-3 justify-center">
            <MessageCircle className="w-5 h-5" />
            {t.githubStats?.discuss || 'Discuss'}
          </a>
        </div>
      </div>
    </section>
  )
}

function Contributing({ t }) {
  return (
    <section className="px-6 py-24 border-t border-gray-700/50 bg-gradient-to-b from-transparent to-primary/5 scroll-mt-20" id="contributing">
      <div className="max-w-7xl mx-auto">
        <h2 className="text-4xl md:text-5xl font-extrabold text-center mb-4">
          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white to-primary">{t.contributing?.title || 'Contributing'}</span>
        </h2>
        <p className="text-gray-400 text-center text-xl mb-16 max-w-2xl mx-auto">
          {t.contributing?.subtitle || 'Help build the future of autonomous AI'}
        </p>
        <div className="grid md:grid-cols-3 gap-8">
          <a href="https://github.com/librefang/librefang/pulls" target="_blank" rel="noopener noreferrer" className="block p-8 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-all group hover:-translate-y-1">
            <div className="w-14 h-14 rounded-2xl bg-primary/20 flex items-center justify-center mb-6 group-hover:bg-primary/30 transition-colors">
              <MaterialIcon name="code" className="w-7 h-7 text-primary" />
            </div>
            <h3 className="text-xl font-bold text-white mb-3">{t.contributing?.code || 'Code'}</h3>
            <p className="text-gray-400 mb-6">{t.contributing?.codeDesc || 'Contribute features, fix bugs, or improve documentation'}</p>
            <span className="text-primary font-semibold">{t.contributing?.submitPR || 'Submit PR'} →</span>
          </a>
          <a href="https://github.com/librefang/librefang/issues" target="_blank" rel="noopener noreferrer" className="block p-8 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-all group hover:-translate-y-1">
            <div className="w-14 h-14 rounded-2xl bg-primary/20 flex items-center justify-center mb-6 group-hover:bg-primary/30 transition-colors">
              <MaterialIcon name="bug_report" className="w-7 h-7 text-primary" />
            </div>
            <h3 className="text-xl font-bold text-white mb-3">{t.contributing?.report || 'Report Bugs'}</h3>
            <p className="text-gray-400 mb-6">{t.contributing?.reportDesc || 'Found an issue? Help us improve by reporting it'}</p>
            <span className="text-primary font-semibold">{t.contributing?.openIssue || 'Open Issue'} →</span>
          </a>
          <a href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer" className="block p-8 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-all group hover:-translate-y-1">
            <div className="w-14 h-14 rounded-2xl bg-primary/20 flex items-center justify-center mb-6 group-hover:bg-primary/30 transition-colors">
              <MaterialIcon name="forum" className="w-7 h-7 text-primary" />
            </div>
            <h3 className="text-xl font-bold text-white mb-3">{t.contributing?.community || 'Community'}</h3>
            <p className="text-gray-400 mb-6">{t.contributing?.communityDesc || 'Join discussions and help other users'}</p>
            <span className="text-primary font-semibold">{t.contributing?.joinDiscuss || 'Join Discussion'} →</span>
          </a>
        </div>
      </div>
    </section>
  )
}

function Footer({ t }) {
  return (
    <footer className="px-6 py-20 border-t border-gray-700/50 bg-[#0a0e14]">
      <div className="max-w-7xl mx-auto flex flex-col md:flex-row justify-between items-center gap-16">
        <div className="flex flex-col items-center md:items-start gap-6">
          <a href="/" className="flex items-center gap-3">
            <div className="flex items-center justify-center">
              <img src="/logo.png" alt="LibreFang Logo" width="32" height="32" className="rounded-md" loading="lazy" decoding="async" />
            </div>
            <span className="font-black text-2xl tracking-tight text-white">LibreFang</span>
          </a>
          <p className="text-gray-400 text-lg max-w-xs text-center md:text-left leading-relaxed">{t.footer.agentOSDesc}</p>
        </div>
        <nav className="grid grid-cols-2 sm:grid-cols-3 gap-16 text-sm" aria-label="Footer navigation">
          <div className="space-y-6">
            <h4 className="font-black text-primary uppercase tracking-[0.2em] text-xs">{t.footer.project}</h4>
            <ul className="flex flex-col gap-4 text-gray-400 font-bold">
              <li><a className="hover:text-primary transition-colors" href="#features">{t.nav.features}</a></li>
              <li><a className="hover:text-primary transition-colors" href="#comparison">{t.nav.comparison}</a></li>
              <li><a className="hover:text-primary transition-colors" href="#install">{t.install?.singleBinary || 'Install'}</a></li>
            </ul>
          </div>
          <div className="space-y-6">
            <h4 className="font-black text-primary uppercase tracking-[0.2em] text-xs">{t.footer.community}</h4>
            <ul className="flex flex-col gap-4 text-gray-400 font-bold">
              <li><a className="hover:text-primary transition-colors" href="https://github.com/librefang/librefang/issues" target="_blank" rel="noopener noreferrer">{t.footer.issues}</a></li>
              <li><a className="hover:text-primary transition-colors" href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer">{t.footer.discussions}</a></li>
              <li><a className="hover:text-primary transition-colors" href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer">GitHub</a></li>
            </ul>
          </div>
          <div className="space-y-6 hidden sm:block">
            <h4 className="font-black text-primary uppercase tracking-[0.2em] text-xs">{t.footer.docs}</h4>
            <ul className="flex flex-col gap-4 text-gray-400 font-bold">
              <li><a className="hover:text-primary transition-colors" href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer">{t.footer.quickStart}</a></li>
              <li><a className="hover:text-primary transition-colors" href="https://github.com/librefang/librefang/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">{t.footer.license}</a></li>
              <li><a className="hover:text-primary transition-colors" href="/privacy/">{t.footer.privacy}</a></li>
            </ul>
          </div>
        </nav>
      </div>
      <div className="max-w-7xl mx-auto mt-20 pt-10 border-t border-gray-700/30 flex flex-col md:flex-row justify-between items-center gap-6 text-xs font-bold text-gray-500">
        <p>&copy; {new Date().getFullYear()} LibreFang.ai</p>
        <div className="flex gap-8">
          <span className="flex items-center gap-2">
            <span className="size-2 bg-primary rounded-full animate-pulse"></span>
            {t.hero.badge}
          </span>
          <span>Rust-Powered</span>
        </div>
      </div>
    </footer>
  )
}

function App() {
  const lang = (() => {
    if (typeof window !== 'undefined' && window.__INITIAL_LANG__) return window.__INITIAL_LANG__
    return getCurrentLang()
  })()

  useEffect(() => {
    document.documentElement.lang = lang
  }, [lang])

  const currentT = translations[lang]

  useEffect(() => {
    if (currentT?.meta) {
      document.title = currentT.meta.title
      const descMeta = document.querySelector('meta[name="description"]')
      if (descMeta) descMeta.setAttribute('content', currentT.meta.description)
      const ogDesc = document.querySelector('meta[property="og:description"]')
      if (ogDesc) ogDesc.setAttribute('content', currentT.meta.description)
    }
  }, [lang, currentT])

  return (
    <div className="font-display antialiased overflow-x-hidden" style={{ background: '#080c10', minHeight: '100vh', color: '#e2e8f0' }}>
      <a href="#main-content" className="skip-link">Skip to main content</a>
      <Header t={currentT} />
      <main id="main-content">
        <Hero t={currentT} />
        <Stats t={currentT} />
        <Features t={currentT} />
        <Comparison t={currentT} />
        <Workflows t={currentT} />
        <Install t={currentT} />
        <FAQ t={currentT} />
        <GitHubStats t={currentT} />
        <Contributing t={currentT} />
      </main>
      <Footer t={currentT} />
    </div>
  )
}

export default function WrappedApp() {
  return (
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  )
}
