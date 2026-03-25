import { useState, useEffect, useCallback } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import {
  ArrowLeft, Copy, Check, ExternalLink, ChevronDown,
  Zap, Server, Cloud, Container, Terminal, Monitor, Layers,
  Loader2, CheckCircle2, XCircle, Github
} from 'lucide-react'
import { cn } from '../lib/utils'

// ---- Types ----

type DeployView = 'platforms' | 'flyio'

interface DeployResult {
  url: string
  dashboardUrl: string
  appName: string
  region: string
}

type ProgressStepStatus = 'pending' | 'active' | 'done' | 'error'

interface ProgressStep {
  id: string
  label: string
  status: ProgressStepStatus
}

// ---- Copy button hook ----

function useCopy() {
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  const copy = useCallback((key: string, text: string) => {
    navigator.clipboard.writeText(text)
    setCopiedKey(key)
    setTimeout(() => setCopiedKey(null), 2000)
  }, [])

  return { copiedKey, copy }
}

// ---- CopyButton component ----

function CopyButton({ copyKey, text, copiedKey, onCopy, className }: {
  copyKey: string
  text: string
  copiedKey: string | null
  onCopy: (key: string, text: string) => void
  className?: string
}) {
  const isCopied = copiedKey === copyKey
  return (
    <button
      onClick={(e) => { e.preventDefault(); e.stopPropagation(); onCopy(copyKey, text) }}
      className={cn(
        'flex-shrink-0 p-1.5 rounded border transition-all',
        isCopied
          ? 'border-green-500/30 text-green-400'
          : 'border-white/10 text-gray-500 hover:text-cyan-400 hover:border-cyan-500/30',
        className,
      )}
      aria-label={isCopied ? 'Copied' : 'Copy to clipboard'}
    >
      {isCopied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
    </button>
  )
}

// ---- Platform card data ----

const PLATFORMS = [
  {
    id: 'flyio',
    name: 'Fly.io',
    desc: 'Free forever, persistent storage',
    icon: Zap,
    badge: 'Recommended',
    badgeClass: 'bg-purple-500/20 text-purple-400',
    accentBorder: 'hover:border-purple-500/50',
    accentShadow: 'hover:shadow-purple-500/10',
    demo: { label: 'Live Demo', url: 'https://flyio.librefang.ai' },
    action: 'flyio' as const,
  },
  {
    id: 'render',
    name: 'Render',
    desc: 'One-click OAuth deploy',
    icon: Server,
    badge: 'Easiest',
    badgeClass: 'bg-green-500/20 text-green-400',
    accentBorder: 'hover:border-green-500/50',
    accentShadow: 'hover:shadow-green-500/10',
    demo: { label: 'Live Demo', url: 'https://render.librefang.ai' },
    warning: 'Free tier: sleeps after 15 min, no persistent storage',
    url: 'https://dashboard.render.com/blueprint/new?repo=https://github.com/librefang/librefang',
  },
  {
    id: 'railway',
    name: 'Railway',
    desc: 'Simple deploy with $5 free credit',
    icon: Layers,
    accentBorder: 'hover:border-blue-500/50',
    accentShadow: 'hover:shadow-blue-500/10',
    url: 'https://railway.com/deploy/Bb7HnN',
  },
  {
    id: 'gcp',
    name: 'GCP',
    desc: 'Free forever (e2-micro), 30GB storage',
    icon: Cloud,
    badge: 'Terraform',
    badgeClass: 'bg-blue-500/20 text-blue-400',
    accentBorder: 'hover:border-blue-500/50',
    accentShadow: 'hover:shadow-blue-500/10',
    url: 'https://github.com/librefang/librefang/tree/main/deploy/gcp',
  },
  {
    id: 'docker',
    name: 'Docker',
    desc: 'One command, runs anywhere',
    icon: Container,
    accentBorder: 'hover:border-blue-500/50',
    accentShadow: 'hover:shadow-blue-500/10',
    url: 'https://github.com/librefang/librefang/blob/main/deploy/docker-compose.yml',
    cmd: 'docker run -p 4545:4545 ghcr.io/librefang/librefang',
  },
] as const

const LOCAL_INSTALLS = [
  {
    id: 'macos',
    name: 'macOS',
    desc: 'Homebrew or download binary',
    icon: Monitor,
    cmd: 'brew install librefang/tap/librefang',
  },
  {
    id: 'linux',
    name: 'Linux',
    desc: 'Install script or download binary',
    icon: Terminal,
    cmd: 'curl -fsSL https://librefang.ai/install.sh | sh',
  },
  {
    id: 'windows',
    name: 'Windows',
    desc: 'PowerShell installer or .msi',
    icon: Monitor,
    cmd: 'irm https://librefang.ai/install.ps1 | iex',
  },
] as const

const DEPLOY_STEPS: Omit<ProgressStep, 'status'>[] = [
  { id: 'auth', label: 'Verifying token...' },
  { id: 'app', label: 'Creating app...' },
  { id: 'net', label: 'Allocating IP addresses...' },
  { id: 'vol', label: 'Creating persistent volume...' },
  { id: 'machine', label: 'Launching machine with Step 3.5 Flash...' },
]

// ---- Main component ----

export default function DeployPage() {
  const [view, setView] = useState<DeployView>('platforms')
  const [version, setVersion] = useState<string>('')
  const { copiedKey, copy } = useCopy()

  // Read ?platform= from URL on mount
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    if (params.get('platform') === 'flyio') {
      setView('flyio')
    }
  }, [])

  // Fetch latest version from GitHub
  useEffect(() => {
    fetch('https://api.github.com/repos/librefang/librefang/releases/latest', {
      headers: { Accept: 'application/vnd.github.v3+json' },
    })
      .then(r => r.ok ? r.json() as Promise<{ tag_name: string }> : null)
      .then(data => { if (data?.tag_name) setVersion(data.tag_name) })
      .catch(() => {})
  }, [])

  const showFlyDeploy = useCallback(() => {
    const url = new URL(window.location.href)
    url.searchParams.set('platform', 'flyio')
    history.replaceState(null, '', url.toString())
    setView('flyio')
    window.scrollTo({ top: 0, behavior: 'smooth' })
  }, [])

  const showPlatforms = useCallback(() => {
    const url = new URL(window.location.href)
    url.searchParams.delete('platform')
    history.replaceState(null, '', url.toString())
    setView('platforms')
  }, [])

  return (
    <div className="min-h-screen bg-surface">
      <div className="max-w-[720px] mx-auto px-4 sm:px-6 py-10 sm:py-12">
        {/* Home link */}
        <a
          href="/"
          className="inline-flex items-center gap-1.5 text-sm text-gray-500 hover:text-cyan-500 transition-colors mb-8"
        >
          <ArrowLeft className="w-4 h-4" />
          librefang.ai
        </a>

        {/* Header */}
        <header className="text-center mb-10">
          <img src="/logo.png" alt="LibreFang" className="w-16 h-16 rounded-2xl mx-auto mb-5" />
          <h1 className="text-3xl sm:text-4xl font-black tracking-tight mb-2">
            <span className="bg-gradient-to-r from-slate-900 dark:from-white to-cyan-600 dark:to-cyan-400 bg-clip-text text-transparent">
              Deploy LibreFang
            </span>
          </h1>
          <p className="text-gray-500 text-sm">Choose your platform</p>
          {version && (
            <div className="inline-flex items-center gap-2 mt-4 px-3 py-1 rounded-full border border-cyan-500/20 bg-cyan-500/5 text-xs font-mono text-cyan-600 dark:text-cyan-400">
              <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-pulse" />
              {version}
            </div>
          )}
        </header>

        {/* Content */}
        <AnimatePresence mode="wait">
          {view === 'platforms' ? (
            <motion.div
              key="platforms"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.25 }}
            >
              <PlatformGrid
                copiedKey={copiedKey}
                onCopy={copy}
                onFlyClick={showFlyDeploy}
              />
            </motion.div>
          ) : (
            <motion.div
              key="flyio"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.25 }}
            >
              <FlyDeployForm onBack={showPlatforms} />
            </motion.div>
          )}
        </AnimatePresence>

        {/* Terminal deploy */}
        <div className="mt-4 bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-5 text-center">
          <p className="text-gray-500 text-sm mb-3">Or deploy from your terminal:</p>
          <div className="bg-surface rounded-lg border border-black/10 dark:border-white/5 px-4 py-3 flex items-center justify-between gap-3 overflow-x-auto">
            <code className="text-sm text-green-400 whitespace-nowrap font-mono">
              <span className="text-gray-600 select-none">$ </span>
              curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash
            </code>
            <CopyButton
              copyKey="terminal-cmd"
              text="curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash"
              copiedKey={copiedKey}
              onCopy={copy}
            />
          </div>
        </div>

        {/* Footer */}
        <footer className="text-center py-8 mt-8 text-sm text-gray-500">
          <div className="flex items-center justify-center gap-4 mb-3">
            <a href="https://github.com/librefang/librefang" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-500 transition-colors flex items-center gap-1.5">
              <Github className="w-4 h-4" />
              GitHub
            </a>
            <span className="text-gray-700">&bull;</span>
            <a href="/" className="hover:text-cyan-500 transition-colors">Website</a>
            <span className="text-gray-700">&bull;</span>
            <a href="https://discord.gg/DzTYqAZZmc" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-500 transition-colors">Discord</a>
          </div>
          <p className="text-gray-600">&copy; {new Date().getFullYear()} LibreFang &mdash; Agent Operating System</p>
        </footer>
      </div>
    </div>
  )
}

// ---- Platform Grid ----

function PlatformGrid({ copiedKey, onCopy, onFlyClick }: {
  copiedKey: string | null
  onCopy: (key: string, text: string) => void
  onFlyClick: () => void
}) {
  return (
    <>
      {/* Cloud platforms */}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3.5 mb-4">
        {PLATFORMS.map((platform) => {
          const Icon = platform.icon

          // Fly.io uses onClick
          if (platform.id === 'flyio') {
            return (
              <button
                key={platform.id}
                onClick={onFlyClick}
                className={cn(
                  'relative text-left bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-5',
                  'transition-all hover:-translate-y-0.5 hover:shadow-lg',
                  platform.accentBorder, platform.accentShadow,
                )}
              >
                <PlatformCardContent
                  icon={<Icon className="w-7 h-7" />}
                  name={platform.name}
                  desc={platform.desc}
                  badge={platform.badge}
                  badgeClass={platform.badgeClass}
                  demo={platform.demo}
                />
              </button>
            )
          }

          // External link platforms
          return (
            <a
              key={platform.id}
              href={platform.url}
              target="_blank"
              rel="noopener noreferrer"
              className={cn(
                'relative block bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-5',
                'transition-all hover:-translate-y-0.5 hover:shadow-lg',
                platform.accentBorder, platform.accentShadow,
              )}
            >
              <PlatformCardContent
                icon={<Icon className="w-7 h-7" />}
                name={platform.name}
                desc={platform.desc}
                badge={'badge' in platform ? platform.badge : undefined}
                badgeClass={'badgeClass' in platform ? platform.badgeClass : undefined}
                demo={'demo' in platform ? platform.demo : undefined}
                warning={'warning' in platform ? platform.warning : undefined}
              />
              {'cmd' in platform && platform.cmd && (
                <div className="mt-2 flex items-center gap-2 bg-surface rounded px-2 py-1.5 font-mono text-xs text-green-400 overflow-hidden">
                  <code className="overflow-x-auto whitespace-nowrap scrollbar-hide flex-1">{platform.cmd}</code>
                  <CopyButton
                    copyKey={`platform-${platform.id}`}
                    text={platform.cmd}
                    copiedKey={copiedKey}
                    onCopy={onCopy}
                  />
                </div>
              )}
            </a>
          )
        })}
      </div>

      {/* Install locally */}
      <div className="mt-8 mb-4">
        <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wider mb-3">Install locally</h2>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3.5">
          {LOCAL_INSTALLS.map((item) => {
            const Icon = item.icon
            return (
              <a
                key={item.id}
                href="https://github.com/librefang/librefang/releases/latest"
                target="_blank"
                rel="noopener noreferrer"
                className={cn(
                  'relative block bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-5',
                  'transition-all hover:-translate-y-0.5 hover:shadow-lg hover:border-blue-500/50 hover:shadow-blue-500/10',
                )}
              >
                <Icon className="w-7 h-7 mb-2.5 text-gray-600 dark:text-gray-400" />
                <div className="font-semibold text-slate-900 dark:text-white text-sm mb-1">{item.name}</div>
                <div className="text-xs text-gray-500 mb-2">{item.desc}</div>
                <div className="flex items-center gap-2 bg-surface rounded px-2 py-1.5 font-mono text-xs text-green-400 overflow-hidden">
                  <code className="overflow-x-auto whitespace-nowrap scrollbar-hide flex-1">{item.cmd}</code>
                  <CopyButton
                    copyKey={`local-${item.id}`}
                    text={item.cmd}
                    copiedKey={copiedKey}
                    onCopy={onCopy}
                  />
                </div>
              </a>
            )
          })}
        </div>
      </div>
    </>
  )
}

// ---- Platform card inner content ----

function PlatformCardContent({ icon, name, desc, badge, badgeClass, demo, warning }: {
  icon: React.ReactNode
  name: string
  desc: string
  badge?: string
  badgeClass?: string
  demo?: { label: string; url: string }
  warning?: string
}) {
  return (
    <>
      {badge && (
        <span className={cn('absolute top-3 right-3 text-[10px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-md', badgeClass)}>
          {badge}
        </span>
      )}
      <div className="text-gray-600 dark:text-gray-400 mb-2.5">{icon}</div>
      <div className="font-semibold text-slate-900 dark:text-white mb-1">{name}</div>
      <div className="text-xs text-gray-500 leading-relaxed">{desc}</div>
      {demo && (
        <div className="mt-2">
          <span
            onClick={(e) => { e.preventDefault(); e.stopPropagation(); window.open(demo.url, '_blank') }}
            className="text-xs text-purple-400 hover:text-purple-300 font-medium cursor-pointer"
            role="link"
            tabIndex={0}
            onKeyDown={(e) => { if (e.key === 'Enter') window.open(demo.url, '_blank') }}
          >
            {demo.label} <ExternalLink className="w-3 h-3 inline" />
          </span>
        </div>
      )}
      {warning && (
        <div className="mt-1.5 text-[11px] text-amber-400 leading-tight">{warning}</div>
      )}
    </>
  )
}

// ---- Fly.io Deploy Form ----

function FlyDeployForm({ onBack }: { onBack: () => void }) {
  const [token, setToken] = useState('')
  const [deploying, setDeploying] = useState(false)
  const [steps, setSteps] = useState<ProgressStep[]>(
    DEPLOY_STEPS.map(s => ({ ...s, status: 'pending' as ProgressStepStatus }))
  )
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<DeployResult | null>(null)
  const [troubleshootOpen, setTroubleshootOpen] = useState<string | null>(null)

  const deploy = useCallback(async () => {
    const trimmed = token.trim()
    if (!trimmed) {
      setError('Please enter your Fly.io API Token.')
      return
    }

    setDeploying(true)
    setError(null)
    setResult(null)

    // Reset steps
    const initial = DEPLOY_STEPS.map(s => ({ ...s, status: 'pending' as ProgressStepStatus }))
    initial[0]!.status = 'active'
    setSteps([...initial])

    // Animate steps progressively
    let currentStep = 0
    const stepInterval = setInterval(() => {
      if (currentStep < DEPLOY_STEPS.length - 1) {
        setSteps(prev => {
          const next = [...prev]
          const cur = next[currentStep]
          if (cur) cur.status = 'done'
          currentStep++
          const nextStep = next[currentStep]
          if (nextStep) nextStep.status = 'active'
          return next
        })
      }
    }, 1500)

    try {
      const res = await fetch('/api/deploy', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ token: trimmed }),
      })

      clearInterval(stepInterval)
      const data = await res.json() as (DeployResult & { error?: string })

      if (!res.ok || data.error) {
        throw new Error(data.error || 'Deployment failed')
      }

      // Mark all steps done
      setSteps(prev => prev.map(s => ({ ...s, status: 'done' as ProgressStepStatus })))
      setResult(data)
    } catch (err) {
      clearInterval(stepInterval)
      setError(err instanceof Error ? err.message : 'Deployment failed')
      setDeploying(false)
      setSteps(DEPLOY_STEPS.map(s => ({ ...s, status: 'pending' as ProgressStepStatus })))
    }
  }, [token])

  return (
    <div>
      {/* Back button */}
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 text-sm text-gray-500 hover:text-cyan-500 transition-colors mb-5 px-3 py-2 border border-black/10 dark:border-white/5 rounded-lg hover:border-cyan-500/30"
      >
        <ArrowLeft className="w-4 h-4" />
        Back to platforms
      </button>

      {/* Badges */}
      <div className="flex flex-wrap justify-center gap-2 mb-6">
        {[
          { label: 'Free LLM included', dotClass: 'bg-green-400' },
          { label: 'No API key needed', dotClass: 'bg-purple-400' },
          { label: '1 GB storage', dotClass: 'bg-amber-400' },
        ].map(b => (
          <span key={b.label} className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full border border-black/10 dark:border-white/5 bg-surface-100 text-xs font-medium text-gray-400">
            <span className={cn('w-2 h-2 rounded-full', b.dotClass)} />
            {b.label}
          </span>
        ))}
      </div>

      {/* Show result or form */}
      {result ? (
        <motion.div
          initial={{ opacity: 0, scale: 0.95 }}
          animate={{ opacity: 1, scale: 1 }}
          className="bg-green-500/5 border border-green-500/20 rounded-xl p-8 text-center"
        >
          <CheckCircle2 className="w-12 h-12 text-green-400 mx-auto mb-4" />
          <h2 className="text-xl font-bold text-green-400 mb-3">Deployed!</h2>
          <p className="text-gray-500 text-sm mb-6">
            Your LibreFang instance is starting up (1-2 min).<br />
            Free LLM (Step 3.5 Flash) is pre-configured and ready to use.
          </p>
          <div className="flex flex-col sm:flex-row gap-3 justify-center mb-6">
            <a
              href={result.url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center justify-center gap-2 px-6 py-3 bg-green-500 text-black font-semibold rounded-lg hover:bg-green-400 transition-colors"
            >
              Open Dashboard
              <ExternalLink className="w-4 h-4" />
            </a>
            <a
              href={result.dashboardUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center justify-center gap-2 px-6 py-3 bg-surface-100 border border-black/10 dark:border-white/5 text-gray-300 font-semibold rounded-lg hover:bg-surface-200 transition-colors"
            >
              Fly.io Console
              <ExternalLink className="w-4 h-4" />
            </a>
          </div>
          <div className="text-sm text-gray-500 space-y-1">
            <p>App: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">{result.appName}</code> &bull; Region: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">{result.region}</code></p>
            <p>Model: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">Step 3.5 Flash (free)</code></p>
            <p>Upgrade model: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">flyctl secrets set OPENAI_API_KEY=sk-... --app {result.appName}</code></p>
          </div>
        </motion.div>
      ) : (
        <>
          {/* Free note */}
          <div className="bg-green-500/5 border border-green-500/20 rounded-xl px-5 py-4 text-sm text-green-400 mb-4 leading-relaxed">
            A free LLM (Step 3.5 Flash via OpenRouter) is pre-configured. Your instance works out of the box &mdash; no API keys required.
          </div>

          {/* Steps card */}
          <div className="bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-6 mb-4">
            <div className="flex items-start gap-3 mb-5">
              <div className="w-7 h-7 rounded-full bg-purple-500/15 border border-purple-500/30 flex items-center justify-center text-xs font-bold text-purple-400 shrink-0 mt-0.5">1</div>
              <div>
                <div className="font-semibold text-slate-900 dark:text-white text-sm mb-1">Get a Fly.io API Token</div>
                <div className="text-xs text-gray-500 leading-relaxed">
                  <a href="https://fly.io/app/sign-up" target="_blank" rel="noopener noreferrer" className="text-purple-400 hover:underline">Sign up</a> or{' '}
                  <a href="https://fly.io/app/sign-in" target="_blank" rel="noopener noreferrer" className="text-purple-400 hover:underline">log in</a> to Fly.io, then go to{' '}
                  <a href="https://fly.io/user/personal_access_tokens" target="_blank" rel="noopener noreferrer" className="text-purple-400 hover:underline">Personal Access Tokens</a> and create a new token.
                </div>
              </div>
            </div>
            <div className="flex items-start gap-3">
              <div className="w-7 h-7 rounded-full bg-purple-500/15 border border-purple-500/30 flex items-center justify-center text-xs font-bold text-purple-400 shrink-0 mt-0.5">2</div>
              <div>
                <div className="font-semibold text-slate-900 dark:text-white text-sm mb-1">Paste and deploy</div>
                <div className="text-xs text-gray-500">Your token is only sent to the Fly.io API and is never stored on our servers.</div>
              </div>
            </div>
          </div>

          {/* Token input and deploy */}
          <div className="bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-6">
            <label htmlFor="fly-token" className="block text-sm font-medium text-gray-500 mb-2">
              Fly.io API Token <span className="text-red-400">*</span>
            </label>
            <input
              id="fly-token"
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="fo1_xxxxxxxxxxxx"
              autoComplete="off"
              disabled={deploying}
              className={cn(
                'w-full px-4 py-3 rounded-lg border bg-surface text-slate-900 dark:text-white text-sm font-mono outline-none transition-colors',
                'border-black/10 dark:border-white/10 focus:border-purple-500/50',
                'placeholder:text-gray-600',
                deploying && 'opacity-50 cursor-not-allowed',
              )}
              onKeyDown={(e) => { if (e.key === 'Enter' && !deploying) deploy() }}
            />

            <button
              onClick={deploy}
              disabled={deploying}
              className={cn(
                'w-full mt-3 py-3.5 rounded-lg font-semibold text-sm transition-all',
                deploying
                  ? 'bg-surface-200 border border-black/10 dark:border-white/5 text-gray-500 cursor-not-allowed'
                  : 'bg-purple-600 hover:bg-purple-500 text-white',
              )}
            >
              {deploying ? 'Deploying...' : 'Deploy to Fly.io'}
            </button>

            {/* Progress steps */}
            {deploying && (
              <motion.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: 'auto', opacity: 1 }}
                transition={{ duration: 0.3 }}
                className="mt-4 space-y-1"
              >
                {steps.map((step) => (
                  <div
                    key={step.id}
                    className={cn(
                      'flex items-center gap-2.5 py-1.5 text-sm transition-colors',
                      step.status === 'pending' && 'text-gray-600',
                      step.status === 'active' && 'text-slate-900 dark:text-white',
                      step.status === 'done' && 'text-green-400',
                      step.status === 'error' && 'text-red-400',
                    )}
                  >
                    <span className="w-5 flex justify-center">
                      {step.status === 'active' && <Loader2 className="w-4 h-4 animate-spin text-purple-400" />}
                      {step.status === 'done' && <CheckCircle2 className="w-4 h-4" />}
                      {step.status === 'error' && <XCircle className="w-4 h-4" />}
                    </span>
                    {step.label}
                  </div>
                ))}
              </motion.div>
            )}

            {/* Error message */}
            {error && (
              <motion.div
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0 }}
                className="mt-3 bg-red-500/10 border border-red-500/20 rounded-lg px-4 py-3 text-sm text-red-400"
              >
                {error}
              </motion.div>
            )}
          </div>
        </>
      )}

      {/* Troubleshooting */}
      <div className="bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-6 mt-4">
        <div className="font-semibold text-slate-900 dark:text-white text-sm mb-3">Troubleshooting</div>
        {[
          {
            id: 'sso',
            q: 'Cannot create Personal Access Token (SSO error)',
            a: (
              <>
                If you see: <em>&quot;Access Tokens cannot be created because an organization requires SSO&quot;</em><br />
                Use a per-org token instead. Run in your terminal:<br />
                <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">flyctl tokens org &lt;your-org-name&gt;</code><br />
                Then paste the generated token above.
              </>
            ),
          },
          {
            id: 'image',
            q: 'Deploy failed: image not found',
            a: (
              <>
                The Docker image <code className="text-green-400 text-xs">ghcr.io/librefang/librefang:latest</code> is built on each release.
                If no release has been published yet, use the terminal deploy script below &mdash; it builds from source.
              </>
            ),
          },
          {
            id: 'llm',
            q: 'How to add or change LLM provider after deploy?',
            a: (
              <>
                <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name</code><br />
                Then edit <code className="text-green-400 text-xs">/data/config.toml</code> via <code className="text-green-400 text-xs">flyctl ssh console</code> to update the default model.
              </>
            ),
          },
        ].map(item => (
          <div key={item.id} className="mb-2 last:mb-0">
            <button
              onClick={() => setTroubleshootOpen(troubleshootOpen === item.id ? null : item.id)}
              className="flex items-center gap-2 w-full text-left py-2 text-sm text-gray-500 hover:text-gray-300 transition-colors"
            >
              <ChevronDown className={cn('w-3.5 h-3.5 transition-transform shrink-0', troubleshootOpen === item.id && 'rotate-180')} />
              {item.q}
            </button>
            {troubleshootOpen === item.id && (
              <motion.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: 'auto', opacity: 1 }}
                transition={{ duration: 0.2 }}
                className="overflow-hidden"
              >
                <div className="pl-5.5 pb-2 text-xs text-gray-500 leading-relaxed">
                  {item.a}
                </div>
              </motion.div>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}
