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

export const useAppStore = create<AppState>((set) => ({
  lang: detectLang(),
  switchLang: (code: string) => {
    set({ lang: code })
    const url = code === 'en' ? '/' : `/${code}`
    window.history.pushState(null, '', url)
    document.documentElement.lang = code
    loadCJKFont(code)
  },
  theme: (typeof window !== 'undefined' && localStorage.getItem('theme') as 'dark' | 'light') || 'dark',
  toggleTheme: () => {
    set((state) => {
      const next = state.theme === 'dark' ? 'light' : 'dark'
      localStorage.setItem('theme', next)
      document.documentElement.classList.toggle('dark', next === 'dark')
      document.documentElement.classList.toggle('light', next === 'light')
      return { theme: next }
    })
  },
}))
