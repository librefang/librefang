import { useCallback, useEffect, useState } from 'react'

const KEY = 'librefang-favorites'

type FavSet = Set<string> // "category/id" tokens

function read(): FavSet {
  if (typeof window === 'undefined') return new Set()
  try {
    const raw = window.localStorage.getItem(KEY)
    if (!raw) return new Set()
    const arr = JSON.parse(raw) as string[]
    return new Set(Array.isArray(arr) ? arr : [])
  } catch {
    return new Set()
  }
}

function write(set: FavSet) {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(KEY, JSON.stringify([...set]))
  } catch { /* quota / privacy mode — silent */ }
}

// Shared subscriber list so multiple hook instances stay in sync within
// the same tab (e.g., star toggled on a card re-renders the count in the
// header). Cross-tab sync is handled by the 'storage' event below.
const listeners = new Set<() => void>()
let current: FavSet | null = null

function getCurrent(): FavSet {
  if (current === null) current = read()
  return current
}

function notify() {
  for (const l of listeners) l()
}

export function useFavorites() {
  const [tick, setTick] = useState(0)

  useEffect(() => {
    const bump = () => setTick(t => t + 1)
    listeners.add(bump)
    const onStorage = (e: StorageEvent) => {
      if (e.key === KEY) { current = read(); bump() }
    }
    window.addEventListener('storage', onStorage)
    return () => {
      listeners.delete(bump)
      window.removeEventListener('storage', onStorage)
    }
  }, [])

  const favs = getCurrent()
  const key = useCallback((category: string, id: string) => `${category}/${id}`, [])
  const isFavorite = useCallback((category: string, id: string) => favs.has(key(category, id)), [favs, key])

  const toggle = useCallback((category: string, id: string) => {
    const k = key(category, id)
    const next = new Set(getCurrent())
    if (next.has(k)) next.delete(k)
    else next.add(k)
    current = next
    write(next)
    notify()
  }, [key])

  // For future use: a page listing all starred items. Returns an ordered
  // array of tokens; callers resolve them against registry data.
  const list = [...favs]
  void tick
  return { isFavorite, toggle, list, count: favs.size }
}
