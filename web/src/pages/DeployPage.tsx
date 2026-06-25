import { useState, useEffect, useCallback } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import {
  ArrowLeft, Copy, Check, ExternalLink, ChevronDown,
  Zap, Server, Cloud, Container, Terminal, Monitor, Layers,
  Loader2, CheckCircle2, XCircle
} from 'lucide-react'
import { Github } from '../components/BrandIcons'
import { cn } from '../lib/utils'
import { getTranslation } from '../i18n'
import type { RichTextPart, Translation } from '../i18n'
import { useAppStore } from '../store'

type DeployCopy = NonNullable<Translation['deploy']>

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

function CopyButton({ copyKey, text, copiedKey, onCopy, labels, className }: {
  copyKey: string
  text: string
  copiedKey: string | null
  onCopy: (key: string, text: string) => void
  labels: { copied: string; copyToClipboard: string }
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
      aria-label={isCopied ? labels.copied : labels.copyToClipboard}
    >
      {isCopied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
    </button>
  )
}

function RichTroubleshootingText({ lines }: { lines: RichTextPart[][] }) {
  return (
    <>
      {lines.map((line, lineIndex) => (
        <span key={lineIndex} className="block">
          {line.map((part, partIndex) => {
            if (part.code) {
              return <code key={partIndex} className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">{part.code}</code>
            }
            if (part.em) {
              return <em key={partIndex}>{part.em}</em>
            }
            return <span key={partIndex}>{part.text}</span>
          })}
        </span>
      ))}
    </>
  )
}

// ---- Platform card data ----

const PLATFORMS = [
  {
    id: 'flyio',
    name: 'Fly.io',
    icon: Zap,
    badgeClass: 'bg-purple-500/20 text-purple-400',
    accentBorder: 'hover:border-purple-500/50',
    accentShadow: 'hover:shadow-purple-500/10',
    demoUrl: 'https://flyio.librefang.ai',
    action: 'flyio' as const,
  },
  {
    id: 'render',
    name: 'Render',
    icon: Server,
    badgeClass: 'bg-green-500/20 text-green-400',
    accentBorder: 'hover:border-green-500/50',
    accentShadow: 'hover:shadow-green-500/10',
    demoUrl: 'https://render.librefang.ai',
    url: 'https://dashboard.render.com/blueprint/new?repo=https://github.com/librefang/librefang',
  },
  {
    id: 'railway',
    name: 'Railway',
    icon: Layers,
    accentBorder: 'hover:border-blue-500/50',
    accentShadow: 'hover:shadow-blue-500/10',
    url: 'https://railway.com/deploy/Bb7HnN',
  },
  {
    id: 'gcp',
    name: 'GCP',
    icon: Cloud,
    badgeClass: 'bg-blue-500/20 text-blue-400',
    accentBorder: 'hover:border-blue-500/50',
    accentShadow: 'hover:shadow-blue-500/10',
    url: 'https://github.com/librefang/librefang/tree/main/deploy/gcp',
  },
  {
    id: 'docker',
    name: 'Docker',
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
    icon: Monitor,
    cmd: 'brew install librefang/tap/librefang',
  },
  {
    id: 'linux',
    icon: Terminal,
    cmd: 'curl -fsSL https://librefang.ai/install.sh | sh',
  },
  {
    id: 'windows',
    icon: Monitor,
    cmd: 'irm https://librefang.ai/install.ps1 | iex',
  },
] as const

const DEPLOY_STEP_IDS = ['auth', 'app', 'net', 'vol', 'machine'] as const

// ---- Main component ----

export default function DeployPage() {
  const lang = useAppStore((s) => s.lang)
  const t = getTranslation(lang)
  const deployCopy = t.deploy!
  const commonCopy = t.common!
  const [view, setView] = useState<DeployView>('platforms')
  const [version, setVersion] = useState<string>('')
  const { copiedKey, copy: copyToClipboard } = useCopy()

  // Read ?platform= from URL on mount
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    if (params.get('platform') === 'flyio') {
      setView('flyio')
    }
  }, [])

  // Fetch latest version from releases proxy
  useEffect(() => {
    fetch('https://stats.librefang.ai/api/releases')
      .then(r => r.ok ? r.json() as Promise<{ tag_name: string }[]> : null)
      .then(data => data?.[0] ?? null)
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
              {deployCopy.title}
            </span>
          </h1>
          <p className="text-gray-500 text-sm">{deployCopy.subtitle}</p>
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
            onCopy={copyToClipboard}
            onFlyClick={showFlyDeploy}
            text={deployCopy}
            labels={commonCopy}
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
              <FlyDeployForm onBack={showPlatforms} text={deployCopy} />
            </motion.div>
          )}
        </AnimatePresence>

        {/* Terminal deploy */}
        <div className="mt-4 bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-5 text-center">
          <p className="text-gray-500 text-sm mb-3">{deployCopy.terminalIntro}</p>
          <div className="bg-surface rounded-lg border border-black/10 dark:border-white/5 px-4 py-3 flex items-center justify-between gap-3 overflow-x-auto">
            <code className="text-sm text-green-400 whitespace-nowrap font-mono">
              <span className="text-gray-600 select-none">$ </span>
              curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash
            </code>
            <CopyButton
              copyKey="terminal-cmd"
              text="curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash"
              copiedKey={copiedKey}
              onCopy={copyToClipboard}
              labels={commonCopy}
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
            <a href="/" className="hover:text-cyan-500 transition-colors">{deployCopy.website}</a>
            <span className="text-gray-700">&bull;</span>
            <a href="https://discord.gg/DzTYqAZZmc" target="_blank" rel="noopener noreferrer" className="hover:text-cyan-500 transition-colors">Discord</a>
          </div>
          <p className="text-gray-600">&copy; {new Date().getFullYear()} LibreFang &mdash; {deployCopy.copyrightSuffix}</p>
        </footer>
      </div>
    </div>
  )
}

// ---- Platform Grid ----

function PlatformGrid({ copiedKey, onCopy, onFlyClick, text, labels }: {
  copiedKey: string | null
  onCopy: (key: string, text: string) => void
  onFlyClick: () => void
  text: DeployCopy
  labels: { copied: string; copyToClipboard: string }
}) {
  return (
    <>
      {/* Cloud platforms */}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3.5 mb-4">
        {PLATFORMS.map((platform) => {
          const Icon = platform.icon
          const platformText = text.platforms[platform.id]!

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
                  desc={platformText.desc}
                  badge={platformText.badge}
                  badgeClass={platform.badgeClass}
                  demo={platform.demoUrl ? { label: platformText.demo ?? '', url: platform.demoUrl } : undefined}
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
                desc={platformText.desc}
                badge={platformText.badge}
                badgeClass={'badgeClass' in platform ? platform.badgeClass : undefined}
                demo={'demoUrl' in platform && platform.demoUrl ? { label: platformText.demo ?? '', url: platform.demoUrl } : undefined}
                warning={platformText.warning}
              />
              {'cmd' in platform && platform.cmd && (
                <div className="mt-2 flex items-center gap-2 bg-surface rounded px-2 py-1.5 font-mono text-xs text-green-400 overflow-hidden">
                  <code className="overflow-x-auto whitespace-nowrap scrollbar-hide flex-1">{platform.cmd}</code>
                  <CopyButton
                    copyKey={`platform-${platform.id}`}
                    text={platform.cmd}
                    copiedKey={copiedKey}
                    onCopy={onCopy}
                    labels={labels}
                  />
                </div>
              )}
            </a>
          )
        })}
      </div>

      {/* Install locally */}
      <div className="mt-8 mb-4">
        <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wider mb-3">{text.installLocally}</h2>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3.5">
          {LOCAL_INSTALLS.map((item) => {
            const Icon = item.icon
            const itemText = text.localInstalls[item.id]!
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
                <div className="font-semibold text-slate-900 dark:text-white text-sm mb-1">{itemText.name}</div>
                <div className="text-xs text-gray-500 mb-2">{itemText.desc}</div>
                <div className="flex items-center gap-2 bg-surface rounded px-2 py-1.5 font-mono text-xs text-green-400 overflow-hidden">
                  <code className="overflow-x-auto whitespace-nowrap scrollbar-hide flex-1">{item.cmd}</code>
                  <CopyButton
                    copyKey={`local-${item.id}`}
                    text={item.cmd}
                    copiedKey={copiedKey}
                    onCopy={onCopy}
                    labels={labels}
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

function FlyDeployForm({ onBack, text }: { onBack: () => void; text: DeployCopy }) {
  const [token, setToken] = useState('')
  const [deploying, setDeploying] = useState(false)
  const [steps, setSteps] = useState<ProgressStep[]>(
    DEPLOY_STEP_IDS.map(id => ({ id, status: 'pending' as ProgressStepStatus }))
  )
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<DeployResult | null>(null)
  const [troubleshootOpen, setTroubleshootOpen] = useState<string | null>(null)

  const deploy = useCallback(async () => {
    const trimmed = token.trim()
    if (!trimmed) {
      setError(text.tokenRequired)
      return
    }

    setDeploying(true)
    setError(null)
    setResult(null)

    // Reset steps
    const initial = DEPLOY_STEP_IDS.map(id => ({ id, status: 'pending' as ProgressStepStatus }))
    initial[0]!.status = 'active'
    setSteps([...initial])

    // Animate steps progressively
    let currentStep = 0
    const stepInterval = setInterval(() => {
      if (currentStep < DEPLOY_STEP_IDS.length - 1) {
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
        throw new Error(data.error || text.deployFailed)
      }

      // Mark all steps done
      setSteps(prev => prev.map(s => ({ ...s, status: 'done' as ProgressStepStatus })))
      setResult(data)
    } catch (err) {
      clearInterval(stepInterval)
      setError(err instanceof Error ? err.message : text.deployFailed)
      setDeploying(false)
      setSteps(DEPLOY_STEP_IDS.map(id => ({ id, status: 'pending' as ProgressStepStatus })))
    }
  }, [text.deployFailed, text.tokenRequired, token])

  return (
    <div>
      {/* Back button */}
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 text-sm text-gray-500 hover:text-cyan-500 transition-colors mb-5 px-3 py-2 border border-black/10 dark:border-white/5 rounded-lg hover:border-cyan-500/30"
      >
        <ArrowLeft className="w-4 h-4" />
        {text.backToPlatforms}
      </button>

      {/* Badges */}
      <div className="flex flex-wrap justify-center gap-2 mb-6">
        {[
          { dotClass: 'bg-green-400' },
          { dotClass: 'bg-purple-400' },
          { dotClass: 'bg-amber-400' },
        ].map((b, index) => (
          <span key={text.badges[index]} className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full border border-black/10 dark:border-white/5 bg-surface-100 text-xs font-medium text-gray-400">
            <span className={cn('w-2 h-2 rounded-full', b.dotClass)} />
            {text.badges[index]}
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
          <h2 className="text-xl font-bold text-green-400 mb-3">{text.deployed}</h2>
          <p className="text-gray-500 text-sm mb-6">
            {text.deployedDesc}
          </p>
          <div className="flex flex-col sm:flex-row gap-3 justify-center mb-6">
            <a
              href={result.url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center justify-center gap-2 px-6 py-3 bg-green-500 text-black font-semibold rounded-lg hover:bg-green-400 transition-colors"
            >
              {text.openDashboard}
              <ExternalLink className="w-4 h-4" />
            </a>
            <a
              href={result.dashboardUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center justify-center gap-2 px-6 py-3 bg-surface-100 border border-black/10 dark:border-white/5 text-gray-300 font-semibold rounded-lg hover:bg-surface-200 transition-colors"
            >
              {text.flyConsole}
              <ExternalLink className="w-4 h-4" />
            </a>
          </div>
          <div className="text-sm text-gray-500 space-y-1">
            <p>{text.app}: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">{result.appName}</code> &bull; {text.region}: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">{result.region}</code></p>
            <p>{text.model}: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">{text.includedModel}</code></p>
            <p>{text.upgradeModel}: <code className="text-green-400 bg-surface px-1.5 py-0.5 rounded text-xs">flyctl secrets set OPENAI_API_KEY=sk-... --app {result.appName}</code></p>
          </div>
        </motion.div>
      ) : (
        <>
          {/* Free note */}
          <div className="bg-green-500/5 border border-green-500/20 rounded-xl px-5 py-4 text-sm text-green-400 mb-4 leading-relaxed">
            {text.freeNote}
          </div>

          {/* Steps card */}
          <div className="bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-6 mb-4">
            <div className="flex items-start gap-3 mb-5">
              <div className="w-7 h-7 rounded-full bg-purple-500/15 border border-purple-500/30 flex items-center justify-center text-xs font-bold text-purple-400 shrink-0 mt-0.5">1</div>
              <div>
                <div className="font-semibold text-slate-900 dark:text-white text-sm mb-1">{text.stepOneTitle}</div>
                <div className="text-xs text-gray-500 leading-relaxed">
                  {text.stepOnePrefix}
                  <a href="https://fly.io/app/sign-up" target="_blank" rel="noopener noreferrer" className="text-purple-400 hover:underline">{text.signUp}</a> {text.or}{' '}
                  <a href="https://fly.io/app/sign-in" target="_blank" rel="noopener noreferrer" className="text-purple-400 hover:underline">{text.logIn}</a> {text.stepOneMiddle}{' '}
                  <a href="https://fly.io/user/personal_access_tokens" target="_blank" rel="noopener noreferrer" className="text-purple-400 hover:underline">{text.personalAccessTokens}</a> {text.stepOneSuffix}
                </div>
              </div>
            </div>
            <div className="flex items-start gap-3">
              <div className="w-7 h-7 rounded-full bg-purple-500/15 border border-purple-500/30 flex items-center justify-center text-xs font-bold text-purple-400 shrink-0 mt-0.5">2</div>
              <div>
                <div className="font-semibold text-slate-900 dark:text-white text-sm mb-1">{text.stepTwoTitle}</div>
                <div className="text-xs text-gray-500">{text.stepTwoDesc}</div>
              </div>
            </div>
          </div>

          {/* Token input and deploy */}
          <div className="bg-surface-100 border border-black/10 dark:border-white/5 rounded-xl p-6">
            <label htmlFor="fly-token" className="block text-sm font-medium text-gray-500 mb-2">
              {text.tokenLabel} <span className="text-red-400">*</span>
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
              {deploying ? text.deploying : text.deployToFly}
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
                    {text.steps[step.id]}
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
        <div className="font-semibold text-slate-900 dark:text-white text-sm mb-3">{text.troubleshooting}</div>
        {(['sso', 'image', 'llm'] as const).map(id => {
          const item = text.troubleshootingItems[id]!
          return (
          <div key={id} className="mb-2 last:mb-0">
            <button
              onClick={() => setTroubleshootOpen(troubleshootOpen === id ? null : id)}
              className="flex items-center gap-2 w-full text-left py-2 text-sm text-gray-500 hover:text-gray-300 transition-colors"
            >
              <ChevronDown className={cn('w-3.5 h-3.5 transition-transform shrink-0', troubleshootOpen === id && 'rotate-180')} />
              {item.q}
            </button>
            {troubleshootOpen === id && (
              <motion.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: 'auto', opacity: 1 }}
                transition={{ duration: 0.2 }}
                className="overflow-hidden"
              >
                <div className="pl-5.5 pb-2 text-xs text-gray-500 leading-relaxed">
                  <RichTroubleshootingText lines={item.lines} />
                </div>
              </motion.div>
            )}
          </div>
        )})}
      </div>
    </div>
  )
}
