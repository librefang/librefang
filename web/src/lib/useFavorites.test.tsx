/** @vitest-environment jsdom */

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useFavorites } from './useFavorites'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined

afterEach(async () => {
  if (root) {
    await act(async () => root?.unmount())
    root = undefined
  }
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

describe('useFavorites', () => {
  it('shares stable snapshots and rejects in-memory changes when persistence fails', async () => {
    let raw: string | null = null
    let failWrites = false
    const setItem = vi.fn((_key: string, value: string) => {
      if (failWrites) throw new Error('quota exceeded')
      raw = value
    })
    vi.stubGlobal('localStorage', {
      getItem: () => raw,
      setItem,
      removeItem: () => {},
      clear: () => { raw = null },
      key: () => null,
      length: 0,
    } satisfies Storage)

    const snapshots = new Map<string, string[]>()
    const listReferences = new Map<string, string[]>()
    function Consumer({ id }: { id: string }) {
      const favorites = useFavorites()
      const [, setRenderCount] = useState(0)
      snapshots.set(id, favorites.list)
      listReferences.set(id, favorites.list)
      return (
        <div data-consumer={id}>
          <span data-count>{favorites.count}</span>
          <span data-favorite>{String(favorites.isFavorite('skills', 'alpha'))}</span>
          <button data-toggle onClick={() => favorites.toggle('skills', 'alpha')}>toggle</button>
          <button data-rerender onClick={() => setRenderCount(count => count + 1)}>rerender</button>
        </div>
      )
    }

    document.body.innerHTML = '<div id="root"></div>'
    root = createRoot(document.querySelector('#root')!)
    await act(async () => root?.render(<><Consumer id="first" /><Consumer id="second" /></>))

    await act(async () => document.querySelector<HTMLElement>('[data-consumer="first"] [data-toggle]')?.click())
    expect(Array.from(document.querySelectorAll('[data-count]'), node => node.textContent)).toEqual(['1', '1'])
    expect(setItem).toHaveBeenCalledWith('librefang-favorites', '["skills/alpha"]')

    const stableList = listReferences.get('first')
    await act(async () => document.querySelector<HTMLElement>('[data-consumer="first"] [data-rerender]')?.click())
    expect(listReferences.get('first')).toBe(stableList)

    failWrites = true
    await act(async () => document.querySelector<HTMLElement>('[data-consumer="second"] [data-toggle]')?.click())
    expect(Array.from(document.querySelectorAll('[data-count]'), node => node.textContent)).toEqual(['1', '1'])
    expect(Array.from(document.querySelectorAll('[data-favorite]'), node => node.textContent)).toEqual(['true', 'true'])

    failWrites = false
    raw = '["plugins/beta"]'
    await act(async () => window.dispatchEvent(new StorageEvent('storage', { key: 'librefang-favorites' })))
    expect(snapshots.get('first')).toEqual(['plugins/beta'])
    expect(snapshots.get('second')).toEqual(['plugins/beta'])
  })
})
