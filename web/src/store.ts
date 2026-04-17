import { create } from 'zustand'

function detectLang(): string {
  if (typeof window === 'undefined') return 'en'
  if (window.__INITIAL_LANG__) return window.__INITIAL_LANG__
  const path = window.location.pathname
  if (path.startsWith('/zh-TW')) return 'zh-TW'
  if (path.startsWith('/zh')) return 'zh'
  if (path.startsWith('/de')) return 'de'
  if (path.startsWith('/ja')) return 'ja'
  if (path.startsWith('/ko')) return 'ko'
  if (path.startsWith('/es')) return 'es'
  return 'en'
}

const CJK_FONTS: Record<string, string> = {
  zh: 'Noto+Sans+SC',
  'zh-TW': 'Noto+Sans+TC',
  ja: 'Noto+Sans+JP',
  ko: 'Noto+Sans+KR',
}

const loadedFonts = new Set<string>()

function loadCJKFont(lang: string) {
  const font = CJK_FONTS[lang]
  if (!font || loadedFonts.has(font)) return
  loadedFonts.add(font)
  const link = document.createElement('link')
  link.rel = 'stylesheet'
  link.href = `https://fonts.googleapis.com/css2?family=${font}:wght@400;500;700;900&display=swap`
  document.head.appendChild(link)
}

interface AppState {
  lang: string
  switchLang: (code: string) => void
  theme: 'dark' | 'light'
  toggleTheme: () => void
}

const LOCALE_PREFIXES = ['zh-TW', 'zh', 'de', 'ja', 'ko', 'es']

// Strip any locale prefix from a pathname so we can re-attach the new one.
// `/zh/skills/foo` → `/skills/foo`, `/skills` → `/skills`, `/` → `/`.
function stripLocalePrefix(pathname: string): string {
  for (const prefix of LOCALE_PREFIXES) {
    if (pathname === `/${prefix}`) return '/'
    if (pathname.startsWith(`/${prefix}/`)) return pathname.slice(prefix.length + 1)
  }
  return pathname
}

export const useAppStore = create<AppState>((set) => ({
  lang: detectLang(),
  switchLang: (code: string) => {
    set({ lang: code })
    const bare = typeof window === 'undefined' ? '/' : stripLocalePrefix(window.location.pathname)
    const url = code === 'en' ? bare : `/${code}${bare === '/' ? '' : bare}`
    window.history.pushState(null, '', url + window.location.search + window.location.hash)
    document.documentElement.lang = code
    loadCJKFont(code)
  },
  theme: (typeof window !== 'undefined' && localStorage.getItem('theme') as 'dark' | 'light') || 'dark',
  toggleTheme: () => {
    // Wrap the class swap in document.startViewTransition when the browser
    // supports it, so dark↔light cross-fades instead of popping. Falls back
    // to the direct swap on Safari / Firefox stable (as of early 2026).
    const apply = () => set((state) => {
      const next = state.theme === 'dark' ? 'light' : 'dark'
      localStorage.setItem('theme', next)
      document.documentElement.classList.toggle('dark', next === 'dark')
      document.documentElement.classList.toggle('light', next === 'light')
      return { theme: next }
    })
    const start = typeof document !== 'undefined' && (document as Document & { startViewTransition?: (cb: () => void) => void }).startViewTransition
    if (start) start.call(document, apply)
    else apply()
  },
}))
