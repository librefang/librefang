import { useState, useEffect } from 'react'
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'
import { translations, languages } from './i18n'
import { ExternalLink, Globe, ChevronDown, Menu, X, ClipboardCheck, Settings, BadgeCheck, Code, Bug, MessageCircle, Copy, Check, Video, UserSearch, Radar, TrendingUp, Activity, Filter, Network, RefreshCw, Shield, Library, Monitor, SearchCheck, Rss } from 'lucide-react'

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
  const Icon = icons[name]
  if (!Icon) return <span className={className}>{name}</span>
  return <Icon className={className} />
}

function Skeleton({ className = '' }) {
  return (
    <div className={`animate-pulse bg-gray-700 rounded ${className}`}></div>
  )
}

function Header({ t }) {
  const currentLangName = languages.find(l => {
    const path = window.location.pathname
    if (path.startsWith('/zh-TW')) return l.code === 'zh-TW'
    if (path.startsWith('/zh')) return l.code === 'zh'
    if (path.startsWith('/de')) return l.code === 'de'
    if (path.startsWith('/ja')) return l.code === 'ja'
    if (path.startsWith('/ko')) return l.code === 'ko'
    if (path.startsWith('/es')) return l.code === 'es'
    return l.code === 'en'
  })?.name || 'English'
  const [langOpen, setLangOpen] = useState(false)
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)

  useEffect(() => {
    const handleEscape = (e) => {
      if (e.key === 'Escape') {
        setMobileMenuOpen(false)
        setLangOpen(false)
      }
    }
    document.addEventListener('keydown', handleEscape)
    return () => document.removeEventListener('keydown', handleEscape)
  }, [])

  return (
    <nav className="fixed top-0 left-0 right-0 z-40 px-4 md:px-12 py-4 glass-effect" role="navigation" aria-label="Main navigation">
      <div className="max-w-7xl mx-auto flex items-center justify-between">
        <a href="/" className="flex items-center gap-3">
          <div className="flex items-center justify-center p-1">
            <img src="/logo.png" alt="Librefang Logo" width="32" height="32" className="rounded-md" loading="lazy" decoding="async" />
          </div>
          <span className="font-extrabold text-2xl tracking-tight">Librefang</span>
        </a>
        <div className="hidden md:flex items-center gap-8 text-sm font-semibold text-gray-400">
          <a className="hover:text-primary transition-colors" href="#features">{t.nav.features}</a>
          <a className="hover:text-primary transition-colors" href="#comparison">{t.nav.comparison}</a>
          <a className="flex items-center gap-1 hover:text-primary transition-colors" href="https://docs.librefang.ai" target="_blank" rel="noopener noreferrer">
            <span>{t.nav.docs}</span>
            <MaterialIcon name="open_in_new" className="text-sm" />
          </a>
          <a className="flex items-center gap-1 hover:text-primary transition-colors" href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer">
            <span>{t.nav.github}</span>
            <MaterialIcon name="open_in_new" className="text-sm" />
          </a>
        </div>
        <div className="flex items-center gap-4">
          <div className="relative group">
            <button className="flex items-center gap-2 p-2 text-gray-400 hover:text-primary transition-colors rounded-lg hover:bg-white/10" aria-label="Switch language" onClick={() => setLangOpen(!langOpen)}>
              <MaterialIcon name="language" />
              <span className="hidden sm:inline text-sm font-bold">{currentLangName}</span>
              <MaterialIcon name="expand_more" className="text-sm" />
            </button>
            <div className={`absolute right-0 mt-2 w-40 bg-[#161b22] border border-gray-700/50 rounded-md shadow-2xl ${langOpen ? 'opacity-100 visible' : 'opacity-0 invisible'} transition-all duration-300 z-50`} role="menu">
              {languages.map((lang, i) => (
                <a key={lang.code} href={lang.url} role="menuitem" className={`block px-5 py-3 text-sm font-bold transition-colors ${i === 0 ? 'bg-primary/10 text-primary rounded-t-md' : 'text-gray-400 hover:bg-white/10 hover:text-gray-100'} ${i === languages.length - 1 ? 'rounded-b-md' : ''}`}>
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
          <button className="md:hidden" onClick={() => setMobileMenuOpen(!mobileMenuOpen)} onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setMobileMenuOpen(!mobileMenuOpen) }} aria-label={mobileMenuOpen ? 'Close menu' : 'Open menu'} aria-expanded={mobileMenuOpen}>
            <MaterialIcon name={mobileMenuOpen ? 'close' : 'menu'} />
          </button>
        </div>
      </div>
      {mobileMenuOpen && (
        <div className="md:hidden px-4 pb-4 space-y-2 bg-[#080c10]/95">
          <a href="#features" className="block py-2 text-gray-400">Features</a>
          <a href="#comparison" className="block py-2 text-gray-400">Comparison</a>
          <a href="#install" className="block py-2 text-gray-400">Docs</a>
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
              <span className="relative inline-flex rounded-full h-2 w-2 bg-primary"></span>
            </span>
            <span className="min-w-[50px] inline-block">{version || ''}</span> · Rust-Powered · Open Source
          </div>
          <h1 className="text-6xl md:text-8xl font-extrabold tracking-tight leading-[1.05] bg-clip-text text-transparent bg-gradient-to-br from-white via-gray-100 to-primary flex items-center justify-center lg:justify-start gap-4">
            <img src="/fox-mascot.png" alt="Librefang Mascot" className="w-16 h-16 md:w-20 md:h-20 rounded-xl" />
            Librefang
          </h1>
          <p className="text-gray-400 text-xl md:text-2xl font-light leading-relaxed max-w-xl mx-auto lg:mx-0">
            {t.hero.subtitle}
          </p>
          <div className="flex flex-col sm:flex-row gap-5 justify-center lg:justify-start pt-4">
            <a href="#install" className="bg-primary hover:bg-primary-dark text-white font-extrabold py-5 px-10 rounded-full shadow-2xl shadow-primary/30 transition-all hover:scale-105 active:scale-95">
              {t.hero.getStarted}
            </a>
            <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="bg-white/10 hover:bg-white/5 text-gray-100 font-bold py-5 px-10 rounded-full border border-gray-600/50 backdrop-blur-md transition-all hover:scale-105 active:scale-95 flex items-center gap-2 justify-center">
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
              <img className="w-full h-full object-cover opacity-80" src="/librefang-vs-claws.png" alt="Librefang Agent OS Interface" width="1920" height="1080" loading="eager" />
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
    <section className="px-6 py-32 bg-[#0d1117] rounded-section mx-4 md:mx-12" id="features">
      <div className="max-w-7xl mx-auto space-y-20">
        <header className="text-center space-y-6">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-white">{t.features.feature1Title}</h2>
          <p className="text-gray-400 text-lg md:text-xl max-w-3xl mx-auto">{t.features.feature1Desc}</p>
          <div className="h-2 w-32 bg-primary mx-auto rounded-full"></div>
        </header>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8">
          {t.featureCards.map((feature, i) => (
            <article key={i} className="bg-[#161b22] p-10 rounded-[2.5rem] border border-gray-700/50 hover:border-primary/50 transition-all duration-500 group hover:-translate-y-3 shadow-sm">
              <div className="size-16 bg-primary/10 rounded-2xl flex items-center justify-center text-primary mb-8 group-hover:bg-primary group-hover:text-white transition-all duration-500">
                <MaterialIcon name={featureIcons[i]} className="text-4xl font-bold" />
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
    <section className="px-6 py-32" id="comparison">
      <div className="max-w-6xl mx-auto">
        <header className="text-center mb-20">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight mb-6 text-white">{t.comparison.librefangVs}</h2>
          <p className="text-gray-400 text-xl">{t.comparison.subtitle2}</p>
        </header>
        <div className="hidden md:block overflow-hidden rounded-[3rem] border border-gray-700/50 bg-white/5 backdrop-blur-xl">
          <table className="w-full text-left">
            <thead className="bg-gray-800 text-gray-300">
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
        <div className="md:hidden space-y-6">
          {[
            { title: 'Cold Start', val: '180ms' },
            { title: 'Security Layers', val: '16' },
            { title: 'Channel Adapters', val: '40' },
          ].map((item, i) => (
            <article key={i} className="bg-white/8 rounded-[2rem] border border-gray-700/50 p-8">
              <h3 className="text-sm font-black uppercase tracking-widest text-primary mb-6">{item.title}</h3>
              <dl className="space-y-4">
                <div className="flex justify-between items-center py-4 border-b border-gray-700/50">
                  <dt className="text-gray-400 font-medium">{t.comparison.openclaw} / {t.comparison.zeroclaw}</dt>
                  <dd className="text-gray-400">{i === 0 ? '2.5s – 4s+' : i === 1 ? '2–3 layers' : '8–15'}</dd>
                </div>
                <div className="flex justify-between items-center py-4">
                  <dt className="text-gray-100 font-bold">Librefang</dt>
                  <dd className="text-primary font-black">{item.val}</dd>
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
    <section className="px-6 py-32">
      <div className="max-w-7xl mx-auto space-y-20">
        <header className="text-center space-y-6">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-white">{t.workflows.workflow1Title}</h2>
          <p className="text-gray-400 text-xl max-w-3xl mx-auto">{t.workflows.workflow1Desc}</p>
        </header>
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-8">
          {t.workflowCards.map((workflow, i) => (
            <article key={i} className="bg-white/5 rounded-[2.5rem] border border-gray-700/50 p-8 space-y-6 hover:border-primary/50 transition-all group">
              <div className="size-16 bg-primary/10 rounded-2xl flex items-center justify-center group-hover:bg-primary transition-colors">
                <MaterialIcon name={workflowIcons[i]} className="text-4xl text-primary group-hover:text-white" />
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
    <section className="px-6 py-32 bg-[#0d1117] rounded-section mx-4 md:mx-12" id="install">
      <div className="max-w-5xl mx-auto space-y-16">
        <header className="text-center space-y-6">
          <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-white">{t.install.singleBinary}</h2>
          <p className="text-gray-400 text-xl">{t.install.singleBinaryDesc}</p>
        </header>
        <div className="rounded-[2.5rem] overflow-hidden shadow-3xl bg-[#080c10] border border-gray-700/50">
          <div className="h-12 flex items-center px-6 justify-between">
            <div className="flex gap-2.5">
              <div className="size-3.5 rounded-full bg-gray-50/20"></div>
              <div className="size-3.5 rounded-full bg-gray-50/20"></div>
              <div className="size-3.5 rounded-full bg-gray-50/20"></div>
            </div>
            <span className="text-xs uppercase tracking-[0.2em] font-black text-white/80">{t.install.bashInstall || 'bash — Quick Install'}</span>
            <div className="w-12"></div>
          </div>
          <div className="p-6 md:p-10 font-mono text-sm md:text-base lg:text-xl leading-relaxed relative bg-black/40">
            <div className="flex items-start gap-3 md:gap-4 overflow-x-auto">
              <span className="text-primary select-none font-bold shrink-0">$</span>
              <code className="flex-1 min-w-0 whitespace-nowrap md:whitespace-normal">
                <span className="text-primary">curl</span>
                <span className="text-white"> -fsSL https://librefang.sh/install</span>
                <span className="text-gray-500"> |</span>
                <span className="text-primary"> sh</span>
              </code>
            </div>
            <button onClick={copyCommand} className="absolute top-6 right-6 md:top-10 md:right-10 text-primary hover:text-white transition-colors p-2.5 md:p-3 bg-primary/10 rounded-xl md:rounded-2xl hover:bg-primary" aria-label="Copy installation command">
              <MaterialIcon name={copied ? 'check' : 'content_copy'} className="text-[24px]" />
            </button>
          </div>
        </div>
        <div className="grid md:grid-cols-2 gap-8">
          <article className="bg-white/5 rounded-[2rem] border border-gray-700/50 p-8 space-y-6">
            <div className="flex items-center gap-4">
              <div className="size-14 bg-primary/10 rounded-xl flex items-center justify-center">
                <MaterialIcon name="checklist" className="text-primary text-3xl" />
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
          <article className="bg-white/5 rounded-[2rem] border border-gray-700/50 p-8 space-y-6">
            <div className="flex items-center gap-4">
              <div className="size-14 bg-primary/10 rounded-xl flex items-center justify-center">
                <MaterialIcon name="settings_suggest" className="text-primary text-3xl" />
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
              <MaterialIcon name="verified" className="text-primary" />
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
    <section className="px-6 py-32" id="faq">
      <div className="max-w-3xl mx-auto space-y-16">
        <h2 className="text-4xl md:text-6xl font-extrabold tracking-tight text-center text-white">{t.faq.title}</h2>
        <div className="space-y-6">
          {t.faq.items.map((faq, i) => (
            <details key={i} className="group bg-white/5 rounded-[2rem] border border-gray-700/50 overflow-hidden transition-all duration-500" open={i === 0}>
              <summary className="flex items-center justify-between p-8 cursor-pointer select-none list-none">
                <h3 className="font-extrabold text-white text-xl">{faq.question}</h3>
                <div className="bg-primary/10 p-2 rounded-full text-primary group-open:rotate-180 transition-transform">
                  <MaterialIcon name="expand_more" className="font-bold" />
                </div>
              </summary>
              <div className="px-8 pb-8 text-gray-400 text-lg leading-relaxed pt-2 border-t border-gray-700/30">
                {faq.answer}
              </div>
            </details>
          ))}
        </div>
      </div>
    </section>
  )
}

function GitHubStats({ t }) {
  // Use worker proxy for GitHub stats (avoids rate limits)
  const { data: githubData, isError: githubError } = useQuery({
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
    initialData: { stars: 0, forks: 0, issues: 0, prs: 0, downloads: 0, lastUpdate: '', starHistory: [] },
    staleTime: 1000 * 60 * 30,
    retry: 1,
  })

  const { data: docsData } = useQuery({
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

  const docsVisits = docsData?.total ?? 0
  const stars = githubError ? null : (githubData?.stars ?? 0)
  const forks = githubData?.forks ?? 0
  const issues = githubData?.issues ?? 0
  const prs = githubData?.prs ?? 0
  const downloads = githubData?.downloads ?? 0
  const lastUpdate = githubData?.lastUpdate ? new Date(githubData.lastUpdate).toLocaleDateString() : ''

  // Use real star history from API, or generate if not available
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
    <section className="px-6 py-20 border-t border-gray-700/50">
      <div className="max-w-7xl mx-auto">
        <h2 className="text-4xl md:text-5xl font-extrabold text-center mb-4">
          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white to-primary">{t.githubStats?.title || 'Join Our Community'}</span>
        </h2>
        <p className="text-gray-400 text-center text-xl mb-16 max-w-2xl mx-auto">
          {t.githubStats?.subtitle || 'Help us build the future of autonomous AI agents'}
        </p>
        <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-7 gap-4">
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-3xl font-black text-primary mb-1">{stars === null ? '-' : stars >= 1000 ? `${(stars/1000).toFixed(1)}k` : stars}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.stars || 'Stars'}</div>
          </div>
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-3xl font-black text-primary mb-1">{forks === null ? '-' : forks >= 1000 ? `${(forks/1000).toFixed(1)}k` : forks}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.forks || 'Forks'}</div>
          </div>
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-3xl font-black text-primary mb-1">{downloads >= 1000 ? `${(downloads/1000).toFixed(1)}k` : downloads}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.downloads || 'Downloads'}</div>
          </div>
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-3xl font-black text-primary mb-1">{docsVisits >= 1000 ? `${(docsVisits/1000).toFixed(1)}k` : docsVisits}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.docsVisits || 'Docs Visits'}</div>
          </div>
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-3xl font-black text-primary mb-1">{issues >= 1000 ? `${(issues/1000).toFixed(1)}k` : issues}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.issues || 'Issues'}</div>
          </div>
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-3xl font-black text-primary mb-1">{prs >= 1000 ? `${(prs/1000).toFixed(1)}k` : prs}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.prs || 'PRs'}</div>
          </div>
          <div className="text-center p-5 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-colors">
            <div className="text-2xl font-black text-primary mb-1">{lastUpdate || '-'}</div>
            <div className="text-gray-400 font-semibold text-sm">{t.githubStats?.lastUpdate || 'Last Update'}</div>
          </div>
        </div>

        {/* Star History */}
        <div className="mt-12 p-6 rounded-2xl bg-white/5 border border-gray-700/30">
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-lg font-bold text-white">{t.githubStats?.starHistory || 'History'}</h3>
            <a href="https://star-history.com/#librefang/librefang" target="_blank" rel="noopener noreferrer" className="text-sm text-primary hover:underline">View Full Chart →</a>
          </div>
          {/* Tabs */}
          <div className="flex gap-2 mb-4">
            {['stars', 'forks', 'issues', 'prs'].map(tab => (
              <button
                key={tab}
                onClick={() => setHistoryTab(tab)}
                className={`px-4 py-2 rounded-full text-sm font-semibold transition-all ${
                  historyTab === tab
                    ? 'bg-primary text-white'
                    : 'bg-white/10 text-gray-400 hover:bg-white/20'
                }`}
              >
                {tab === 'stars' ? (t.githubStats?.stars || 'Stars') : tab === 'forks' ? (t.githubStats?.forks || 'Forks') : tab === 'issues' ? (t.githubStats?.issues || 'Issues') : (t.githubStats?.prs || 'PRs')}
              </button>
            ))}
          </div>
          <div className="h-48 flex items-end gap-1 overflow-x-auto">
            {currentHistory.length > 0 ? (
              Array.from({ length: 12 }, (_, i) => {
                const idx = Math.floor((i / 12) * currentHistory.length)
                const value = currentHistory[idx] || 0
                return (
                  <div key={i} className="flex-1 bg-primary/60 hover:bg-primary transition-colors rounded-t min-w-4" style={{ height: `${Math.max(5, (value / currentMax) * 100)}%` }} title={`${value} ${historyTab}`}></div>
                )
              })
            ) : (
              <div className="w-full h-32 flex items-center justify-center text-gray-500">No data yet</div>
            )}
          </div>
          <div className="flex justify-between mt-2 text-xs text-gray-500">
            <span>12 months ago</span>
            <span>Now ({currentValue ?? '-'})</span>
          </div>
        </div>
        <div className="flex justify-center gap-6 mt-12">
          <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="bg-white/10 hover:bg-primary text-white font-bold py-4 px-8 rounded-full border border-gray-600/50 transition-all hover:scale-105 flex items-center gap-3">
            <svg className="w-6 h-6" fill="currentColor" viewBox="0 0 24 24"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/></svg>
            {t.githubStats?.starUs || 'Star Us'}
          </a>
          <a href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer" className="bg-white/5 hover:bg-white/10 text-gray-100 font-bold py-4 px-8 rounded-full border border-gray-600/30 transition-all hover:scale-105 flex items-center gap-3">
            <svg className="w-6 h-6" fill="currentColor" viewBox="0 0 24 24"><path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z"/></svg>
            {t.githubStats?.discuss || 'Discuss'}
          </a>
        </div>
      </div>
    </section>
  )
}

function Contributing({ t }) {
  return (
    <section className="px-6 py-20 border-t border-gray-700/50 bg-gradient-to-b from-transparent to-primary/5">
      <div className="max-w-7xl mx-auto">
        <h2 className="text-4xl md:text-5xl font-extrabold text-center mb-4">
          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white to-primary">{t.contributing?.title || 'Contributing'}</span>
        </h2>
        <p className="text-gray-400 text-center text-xl mb-16 max-w-2xl mx-auto">
          {t.contributing?.subtitle || 'Help build the future of autonomous AI'}
        </p>
        <div className="grid md:grid-cols-3 gap-8">
          <div className="p-8 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-all group">
            <div className="w-14 h-14 rounded-2xl bg-primary/20 flex items-center justify-center mb-6 group-hover:bg-primary/30 transition-colors">
              <MaterialIcon name="code" className="text-3xl text-primary" />
            </div>
            <h3 className="text-xl font-bold text-white mb-3">{t.contributing?.code || 'Code'}</h3>
            <p className="text-gray-400 mb-6">{t.contributing?.codeDesc || 'Contribute features, fix bugs, or improve documentation'}</p>
            <a href="https://github.com/librefang/librefang/pulls" target="_blank" rel="noopener noreferrer" className="text-primary font-semibold hover:underline">{t.contributing?.submitPR || 'Submit PR'} →</a>
          </div>
          <div className="p-8 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-all group">
            <div className="w-14 h-14 rounded-2xl bg-primary/20 flex items-center justify-center mb-6 group-hover:bg-primary/30 transition-colors">
              <MaterialIcon name="bug_report" className="text-3xl text-primary" />
            </div>
            <h3 className="text-xl font-bold text-white mb-3">{t.contributing?.report || 'Report Bugs'}</h3>
            <p className="text-gray-400 mb-6">{t.contributing?.reportDesc || 'Found an issue? Help us improve by reporting it'}</p>
            <a href="https://github.com/librefang/librefang/issues" target="_blank" rel="noopener noreferrer" className="text-primary font-semibold hover:underline">{t.contributing?.openIssue || 'Open Issue'} →</a>
          </div>
          <div className="p-8 rounded-2xl bg-white/5 border border-gray-700/30 hover:border-primary/50 transition-all group">
            <div className="w-14 h-14 rounded-2xl bg-primary/20 flex items-center justify-center mb-6 group-hover:bg-primary/30 transition-colors">
              <MaterialIcon name="forum" className="text-3xl text-primary" />
            </div>
            <h3 className="text-xl font-bold text-white mb-3">{t.contributing?.community || 'Community'}</h3>
            <p className="text-gray-400 mb-6">{t.contributing?.communityDesc || 'Join discussions and help other users'}</p>
            <a href="https://github.com/librefang/librefang/discussions" target="_blank" rel="noopener noreferrer" className="text-primary font-semibold hover:underline">{t.contributing?.joinDiscuss || 'Join Discussion'} →</a>
          </div>
        </div>
      </div>
    </section>
  )
}

function Footer({ t }) {
  return (
    <footer className="px-6 py-20 border-t border-gray-700/50 bg-white/5">
      <div className="max-w-7xl mx-auto flex flex-col md:flex-row justify-between items-center gap-16">
        <div className="flex flex-col items-center md:items-start gap-6">
          <a href="/" className="flex items-center gap-3">
            <div className="flex items-center justify-center">
              <img src="/logo.png" alt="Librefang Logo" width="32" height="32" className="rounded-md" loading="lazy" decoding="async" />
            </div>
            <span className="font-black text-2xl tracking-tight text-white">Librefang</span>
          </a>
          <p className="text-gray-400 text-lg max-w-xs text-center md:text-left leading-relaxed">{t.footer.agentOSDesc}</p>
        </div>
        <nav className="grid grid-cols-2 sm:grid-cols-3 gap-16 text-sm">
          <div className="space-y-6">
            <h4 className="font-black text-primary uppercase tracking-[0.2em] text-xs">{t.footer.project}</h4>
            <ul className="flex flex-col gap-4 text-gray-400 font-bold">
              <li><a className="hover:text-primary transition-colors" href="#features">{t.nav.features}</a></li>
              <li><a className="hover:text-primary transition-colors" href="#comparison">{t.nav.comparison}</a></li>
              <li><a className="hover:text-primary transition-colors" href="#install">{t.nav.docs}</a></li>
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
              <li><a className="hover:text-primary transition-colors" href="https://github.com/librefang/librefang#readme" target="_blank" rel="noopener noreferrer">{t.footer.quickStart}</a></li>
              <li><a className="hover:text-primary transition-colors" href="https://github.com/librefang/librefang/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">{t.footer.license}</a></li>
              <li><a className="hover:text-primary transition-colors" href="/privacy/">{t.footer.privacy}</a></li>
            </ul>
          </div>
        </nav>
      </div>
      <div className="max-w-7xl mx-auto mt-20 pt-10 border-t border-gray-700/30 flex flex-col md:flex-row justify-between items-center gap-6 text-xs font-bold text-gray-400">
        <p>© 2026 Librefang.ai</p>
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
    if (typeof window === 'undefined') return 'en'
    const path = window.location.pathname
    if (path.startsWith('/zh-TW')) return 'zh-TW'
    if (path.startsWith('/zh')) return 'zh'
    if (path.startsWith('/de')) return 'de'
    if (path.startsWith('/ja')) return 'ja'
    if (path.startsWith('/ko')) return 'ko'
    if (path.startsWith('/es')) return 'es'
    return 'en'
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
    <div className="bg-background-light font-display text-slate-900 antialiased overflow-x-hidden" style={{ background: '#080c10', minHeight: '100vh' }}>
      <Header t={currentT} />
      <main>
        <Hero t={currentT} />
        <Stats t={currentT} />
        <Features t={currentT} />
        <Comparison t={currentT} />
        <Workflows t={currentT} />
        <Install t={currentT} />
        <FAQ t={currentT} />
      </main>
      <GitHubStats t={currentT} />
      <Contributing t={currentT} />
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
