import { create } from 'zustand'

type Theme = 'dark' | 'light'

const LOCALE_PREFIXES = ['zh-TW', 'zh', 'de', 'ja', 'ko', 'es', 'pl', 'uk'] as const
const SUPPORTED_LANGS = new Set<string>(['en', ...LOCALE_PREFIXES])

export function detectLangFromPath(path: string, initialLang?: string): string {
  if (initialLang && SUPPORTED_LANGS.has(initialLang)) return initialLang
  for (const prefix of LOCALE_PREFIXES) {
    if (path === `/${prefix}` || path.startsWith(`/${prefix}/`)) return prefix
  }
  return 'en'
}

function detectLang(): string {
  if (typeof window === 'undefined') return 'en'
  return detectLangFromPath(window.location.pathname, window.__INITIAL_LANG__)
}

const CJK_FONTS: Record<string, string> = {
  zh: 'Noto+Sans+SC',
  'zh-TW': 'Noto+Sans+TC',
  ja: 'Noto+Sans+JP',
  ko: 'Noto+Sans+KR',
}

const loadedFonts = new Set<string>()

if (import.meta.hot) {
  import.meta.hot.dispose(() => loadedFonts.clear())
}

export function loadCJKFont(lang: string) {
  if (typeof document === 'undefined') return
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
  theme: Theme
  toggleTheme: () => void
}

// Strip any locale prefix from a pathname so we can re-attach the new one.
// `/zh/skills/foo` → `/skills/foo`, `/skills` → `/skills`, `/` → `/`.
export function stripLocalePrefix(pathname: string): string {
  for (const prefix of LOCALE_PREFIXES) {
    if (pathname === `/${prefix}`) return '/'
    if (pathname.startsWith(`/${prefix}/`)) return pathname.slice(prefix.length + 1)
  }
  return pathname
}

export function buildLocalizedPath(code: string, pathname: string): string {
  const bare = stripLocalePrefix(pathname)
  const prefix = code === 'en' ? '' : `/${code}`
  const suffix = bare === '/' ? '' : bare
  return prefix ? `${prefix}${suffix}` : bare
}

export function readStoredTheme(storage?: Pick<Storage, 'getItem'>): Theme {
  try {
    const stored = storage?.getItem('theme')
    return stored === 'dark' || stored === 'light' ? stored : 'dark'
  } catch {
    return 'dark'
  }
}

function readBrowserTheme(): Theme {
  if (typeof window === 'undefined') return 'dark'
  try {
    return readStoredTheme(window.localStorage)
  } catch {
    return 'dark'
  }
}

export const useAppStore = create<AppState>((set) => ({
  lang: detectLang(),
  switchLang: (code: string) => {
    if (typeof window === 'undefined' || typeof document === 'undefined') return
    const url = buildLocalizedPath(code, window.location.pathname)
    set({ lang: code })
    window.history.pushState(null, '', url + window.location.search + window.location.hash)
    document.documentElement.lang = code
    loadCJKFont(code)
  },
  theme: readBrowserTheme(),
  toggleTheme: () => {
    if (typeof window === 'undefined' || typeof document === 'undefined') return
    // Wrap the class swap in document.startViewTransition when the browser
    // supports it, so dark↔light cross-fades instead of popping. Falls back
    // to the direct swap on Safari / Firefox stable (as of early 2026).
    const apply = () => {
      const next = useAppStore.getState().theme === 'dark' ? 'light' : 'dark'
      try {
        window.localStorage.setItem('theme', next)
      } catch {
        // Keep the in-memory theme usable when browser storage is unavailable.
      }
      document.documentElement.classList.toggle('dark', next === 'dark')
      document.documentElement.classList.toggle('light', next === 'light')
      set({ theme: next })
    }
    const start = (document as Document & {
      startViewTransition?: (callback: () => void) => void
    }).startViewTransition
    if (start) start.call(document, apply)
    else apply()
  },
}))
