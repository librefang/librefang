import { afterEach, describe, expect, it, vi } from 'vitest'
import { fetchMarketplace, marketplaceQueryOptions } from './useMarketplace'

vi.mock('@tanstack/react-query', () => ({ useQuery: vi.fn() }))

function response(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

describe('fetchMarketplace', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('loads every package page with the API maximum page size', async () => {
    const first = Array.from({ length: 100 }, (_, id) => ({ id: String(id) }))
    const second = [{ id: '100' }]
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(response({ packages: first, total: 101 }))
      .mockResolvedValueOnce(response({ packages: second, total: 101 }))
    vi.stubGlobal('fetch', fetchMock)

    const packages = await fetchMarketplace('skill')

    expect(packages).toHaveLength(101)
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/v1/packages?kind=skill&limit=100&offset=0')
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/v1/packages?kind=skill&limit=100&offset=100')
  })

  it('rejects an incomplete paginated response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(response({ packages: [], total: 1 })))

    await expect(fetchMarketplace('hand')).rejects.toThrow('incomplete package list')
  })
})

describe('marketplaceQueryOptions', () => {
  it('keys covered categories by the requested marketplace kind', () => {
    const options = marketplaceQueryOptions('plugins')

    expect(options.queryKey).toEqual(['marketplace', 'extension'])
    expect(options.enabled).toBe(true)
  })

  it('guards categories without marketplace coverage', async () => {
    const options = marketplaceQueryOptions('agents')

    expect(options.queryKey).toEqual(['marketplace', null])
    expect(options.enabled).toBe(false)
    await expect(options.queryFn()).resolves.toEqual([])
  })
})
