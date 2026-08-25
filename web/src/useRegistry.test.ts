import { describe, expect, it, vi } from 'vitest'
import {
  fetchRegistryData,
  getCategoryItems,
  getLocalizedDesc,
  getLocalizedName,
  type Detail,
  type RegistryData,
} from './useRegistry'

function detail(id: string, category = 'hands'): Detail {
  return {
    id,
    name: `Name ${id}`,
    description: `Description ${id}`,
    category,
    icon: 'box',
  }
}

function response(body: unknown): Response {
  return {
    ok: true,
    json: async () => body,
  } as Response
}

function registryData(overrides: Partial<RegistryData>): RegistryData {
  return {
    hands: [],
    channels: [],
    providers: [],
    workflows: [],
    agents: [],
    plugins: [],
    skills: [],
    mcp: [],
    ...overrides,
  }
}

describe('registry data contracts', () => {
  it('starts local and API requests together and falls back when local loading rejects', async () => {
    const apiData = { hands: [detail('api')] }
    const fetcher = vi.fn((url: string | URL | Request) => {
      if (String(url) === '/registry.json') return Promise.reject(new Error('cache unavailable'))
      return Promise.resolve(response(apiData))
    }) as unknown as typeof fetch

    const resultPromise = fetchRegistryData(fetcher)

    expect(fetcher).toHaveBeenCalledTimes(2)
    await expect(resultPromise).resolves.toMatchObject(apiData)
  })

  it('keeps valid API items when count values are explicitly null', async () => {
    const local = { hands: [detail('local')], handsCount: 4 }
    const api = { hands: [detail('api')], handsCount: null }
    const fetcher = vi.fn((url: string | URL | Request) => Promise.resolve(
      response(String(url) === '/registry.json' ? local : api),
    )) as unknown as typeof fetch

    const result = await fetchRegistryData(fetcher)

    expect(result.hands.map(item => item.id)).toEqual(['local', 'api'])
    expect(getCategoryItems(result, 'hands')).toMatchObject({ count: 4 })
  })

  it('falls back to API data when local JSON parsing fails', async () => {
    const api = { skills: [detail('api-skill', 'skills')], skillsCount: 1 }
    const fetcher = vi.fn((url: string | URL | Request) => {
      if (String(url) === '/registry.json') {
        return Promise.resolve({
          ok: true,
          json: async () => { throw new SyntaxError('invalid JSON') },
        } as unknown as Response)
      }
      return Promise.resolve(response(api))
    }) as unknown as typeof fetch

    await expect(fetchRegistryData(fetcher)).resolves.toMatchObject(api)
  })

  it('uses exact and language-prefix translations without unchecked indexing', () => {
    const translated = {
      ...detail('localized'),
      i18n: {
        zh: { name: '中文名称', description: '中文描述' },
        'zh-TW': { name: '繁體名稱' },
      },
    }

    expect(getLocalizedName(translated, 'zh-TW')).toBe('繁體名稱')
    expect(getLocalizedDesc(translated, 'zh-TW')).toBe('中文描述')
    expect(getLocalizedName(translated, '')).toBe('Name localized')
  })

  it('reads counts only through the category count-field map', () => {
    const data = registryData({
      hands: [detail('one')],
      handsCount: 7,
      skills: [detail('skill', 'skills')],
    })

    expect(getCategoryItems(data, 'hands').count).toBe(7)
    expect(getCategoryItems(data, 'skills').count).toBe(1)
  })
})
