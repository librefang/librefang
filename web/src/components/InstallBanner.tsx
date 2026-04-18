import { useEffect, useState } from 'react'
import { Download, X } from 'lucide-react'
import { translations } from '../i18n'
import { useAppStore } from '../store'

// BeforeInstallPromptEvent isn't in lib.dom.d.ts yet, define the shape we
// actually touch.
interface BIPEvent extends Event {
  readonly platforms?: string[]
  prompt: () => Promise<void>
  readonly userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>
}

const DISMISS_KEY = 'librefang.install.dismissed'

export default function InstallBanner() {
  const lang = useAppStore(s => s.lang)
  const t = translations[lang] || translations['en']!
  const [event, setEvent] = useState<BIPEvent | null>(null)
  const [dismissed, setDismissed] = useState(false)

  useEffect(() => {
    if (typeof window === 'undefined') return
    if (localStorage.getItem(DISMISS_KEY) === '1') {
      setDismissed(true)
      return
    }
    const onPrompt = (e: Event) => {
      // Capture the event — Chrome suppresses its own mini-infobar only if
      // we call preventDefault(), then lets us trigger prompt() on demand.
      e.preventDefault()
      setEvent(e as BIPEvent)
    }
    window.addEventListener('beforeinstallprompt', onPrompt)
    return () => window.removeEventListener('beforeinstallprompt', onPrompt)
  }, [])

  if (dismissed || !event) return null

  const close = () => {
    localStorage.setItem(DISMISS_KEY, '1')
    setDismissed(true)
  }

  const install = async () => {
    try {
      await event.prompt()
      const outcome = await event.userChoice
      if (outcome.outcome === 'accepted' || outcome.outcome === 'dismissed') {
        close()
      }
    } catch { /* user cancelled */ }
  }

  return (
    <div className="fixed bottom-4 left-1/2 -translate-x-1/2 z-40 flex items-center gap-3 px-4 py-3 bg-surface-100 border border-cyan-500/30 rounded shadow-xl max-w-md">
      <Download className="w-4 h-4 text-cyan-500 shrink-0" />
      <div className="min-w-0">
        <div className="text-sm font-bold text-slate-900 dark:text-white">
          {t.pwa?.title || 'Install LibreFang'}
        </div>
        <div className="text-xs text-gray-500 truncate">
          {t.pwa?.desc || 'Add the site to your home screen or dock.'}
        </div>
      </div>
      <button
        onClick={install}
        className="shrink-0 px-3 py-1 text-xs font-bold bg-cyan-500 hover:bg-cyan-400 text-surface rounded"
      >
        {t.pwa?.install || 'Install'}
      </button>
      <button
        onClick={close}
        aria-label={t.pwa?.dismiss || 'Dismiss'}
        className="shrink-0 p-1 text-gray-400 hover:text-slate-900 dark:hover:text-white transition-colors"
      >
        <X className="w-4 h-4" />
      </button>
    </div>
  )
}
