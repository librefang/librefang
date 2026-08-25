import { useCallback, useMemo, useSyncExternalStore } from 'react'

const KEY = 'librefang-favorites'

type FavSet = Set<string> // "category/id" tokens

const EMPTY_FAVORITES: FavSet = new Set()

function read(): FavSet {
  if (typeof window === 'undefined') return new Set()
  try {
    const raw = window.localStorage.getItem(KEY)
    if (!raw) return new Set()
    const parsed: unknown = JSON.parse(raw)
    return new Set(Array.isArray(parsed) ? parsed.filter(value => typeof value === 'string') : [])
  } catch {
    return new Set()
  }
}

function write(set: FavSet): boolean {
  if (typeof window === 'undefined') return false
  try {
    window.localStorage.setItem(KEY, JSON.stringify([...set]))
    return true
  } catch {
    return false
  }
}

// Shared subscriber list so multiple hook instances stay in sync within
// the same tab. One storage listener handles cross-tab updates regardless
// of how many components subscribe.
const listeners = new Set<() => void>()
let current: FavSet | null = null

function getCurrent(): FavSet {
  if (current === null) current = read()
  return current
}

function getServerSnapshot(): FavSet {
  return EMPTY_FAVORITES
}

function notify() {
  for (const listener of listeners) listener()
}

function handleStorage(event: StorageEvent) {
  if (event.key !== KEY && event.key !== null) return
  current = read()
  notify()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  if (listeners.size === 1 && typeof window !== 'undefined') {
    window.addEventListener('storage', handleStorage)
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && typeof window !== 'undefined') {
      window.removeEventListener('storage', handleStorage)
    }
  }
}

function favoriteKey(category: string, id: string): string {
  return `${category}/${id}`
}

export function useFavorites() {
  const favs = useSyncExternalStore(subscribe, getCurrent, getServerSnapshot)
  const isFavorite = useCallback(
    (category: string, id: string) => favs.has(favoriteKey(category, id)),
    [favs],
  )

  const toggle = useCallback((category: string, id: string): boolean => {
    const token = favoriteKey(category, id)
    const next = new Set(getCurrent())
    if (next.has(token)) next.delete(token)
    else next.add(token)
    if (!write(next)) return false
    current = next
    notify()
    return true
  }, [])

  // For future use: a page listing all starred items. Returns an ordered
  // array of tokens; callers resolve them against registry data.
  const list = useMemo(() => [...favs], [favs])
  return { isFavorite, toggle, list, count: favs.size }
}
