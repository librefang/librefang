/** @vitest-environment jsdom */

import { afterEach, describe, expect, it } from 'vitest'
import {
  createAppQueryClient,
  DEFAULT_QUERY_STALE_TIME_MS,
  requireRootElement,
} from './bootstrap'

afterEach(() => {
  document.body.innerHTML = ''
})

describe('website bootstrap contracts', () => {
  it('reports a clear error when the HTML root is absent', () => {
    expect(() => requireRootElement(document)).toThrow(
      'Root element #root not found; ensure the HTML template includes <div id="root"></div>',
    )

    document.body.innerHTML = '<div id="root"></div>'
    expect(requireRootElement(document)).toBe(document.getElementById('root'))
  })

  it('uses a fresh global query policy while allowing stable queries to override it', () => {
    const queryClient = createAppQueryClient()
    const defaults = queryClient.getDefaultOptions().queries

    expect(DEFAULT_QUERY_STALE_TIME_MS).toBe(30_000)
    expect(defaults?.staleTime).toBe(30_000)
    expect(defaults?.refetchOnWindowFocus).toBe(true)
  })
})
