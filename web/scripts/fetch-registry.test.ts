import { describe, expect, it, vi } from 'vitest'
import {
  createRegistryData,
  fetchDir,
  fetchToml,
  fetchWithRetry,
  parseSkillMd,
  parseToml,
  type Detail,
} from './fetch-registry'

function detail(id: string): Detail {
  return {
    id,
    name: id,
    description: '',
    category: 'test',
    icon: '',
  }
}

describe('registry requests', () => {
  it('retries transient HTTP failures', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(new Response('temporary', { status: 503 }))
      .mockResolvedValueOnce(new Response('limited', { status: 429 }))
      .mockResolvedValueOnce(new Response('ok', { status: 200 }))

    const response = await fetchWithRetry('https://example.test/registry', {}, {
      fetchImpl: fetchImpl as typeof fetch,
      retries: 2,
      retryDelayMs: 0,
    })

    expect(await response.text()).toBe('ok')
    expect(fetchImpl).toHaveBeenCalledTimes(3)
  })

  it('does not retry permanent HTTP failures', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response('forbidden', { status: 403 }))

    const response = await fetchWithRetry('https://example.test/registry', {}, {
      fetchImpl: fetchImpl as typeof fetch,
      retries: 2,
      retryDelayMs: 0,
    })

    expect(response.status).toBe(403)
    expect(fetchImpl).toHaveBeenCalledOnce()
  })

  it('aborts a request after its timeout', async () => {
    const fetchImpl = vi.fn((_input: RequestInfo | URL, init?: RequestInit) =>
      new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'))
        })
      }),
    )

    await expect(fetchWithRetry('https://example.test/hangs', {}, {
      fetchImpl: fetchImpl as typeof fetch,
      retries: 0,
      timeoutMs: 5,
    })).rejects.toMatchObject({ name: 'AbortError' })
  })

  it('keeps only explicitly optional directory 404s empty', async () => {
    const missing = await fetchDir(
      'optional',
      async () => new Response('missing', { status: 404 }),
      true,
    )

    expect(missing).toEqual([])

    await expect(fetchDir(
      'hands',
      async () => new Response('missing', { status: 404 }),
    )).rejects.toThrow('Failed to fetch hands: HTTP 404: missing')
  })

  it('fails non-404 directory requests with GitHub diagnostics', async () => {
    await expect(fetchDir(
      'mcp',
      async () => new Response('{"message":"API rate limit exceeded"}', { status: 403 }),
    )).rejects.toThrow('Failed to fetch mcp: HTTP 403: {"message":"API rate limit exceeded"}')
  })

  it('keeps missing manifests optional but fails on other raw-content errors', async () => {
    await expect(fetchToml(
      'agents/missing/agent.toml',
      'missing',
      async () => new Response('missing', { status: 404 }),
    )).resolves.toBeNull()

    await expect(fetchToml(
      'agents/private/agent.toml',
      'private',
      async () => new Response('upstream unavailable', { status: 503 }),
    )).rejects.toThrow(
      'Failed to fetch agents/private/agent.toml: HTTP 503: upstream unavailable',
    )
  })
})

describe('registry manifest parsing', () => {
  it('parses escaped quotes in top-level and localized TOML strings', () => {
    const parsed = parseToml(
      [
        'id = "quoted"',
        'name = "Agent \\"Smith\\""',
        'description = "Handles \\"quoted\\" input"',
        '[i18n.de]',
        'name = "Agent \\"Schmidt\\""',
      ].join('\n'),
      'fallback',
    )

    expect(parsed.name).toBe('Agent "Smith"')
    expect(parsed.description).toBe('Handles "quoted" input')
    expect(parsed.i18n?.de?.name).toBe('Agent "Schmidt"')
  })

  it('parses quoted and unquoted YAML scalars without truncating inline quotes', () => {
    const parsed = parseSkillMd(
      [
        '---',
        'id: sample',
        'name: "Quoted \\"Skill\\""',
        'description: Supports "inline quotes" without YAML wrapping',
        '---',
      ].join('\n'),
      'fallback',
    )

    expect(parsed?.name).toBe('Quoted "Skill"')
    expect(parsed?.description).toBe('Supports "inline quotes" without YAML wrapping')
  })

  it('derives published counts from successfully parsed details', () => {
    const data = createRegistryData({
      hands: [detail('one')],
      channels: [],
      providers: [detail('one'), detail('two')],
      workflows: [],
      agents: [],
      plugins: [],
      skills: [],
      mcp: [],
    }, '2026-08-17T00:00:00.000Z')

    expect(data.handsCount).toBe(data.hands.length)
    expect(data.providersCount).toBe(data.providers.length)
    expect(data.mcpCount).toBe(data.mcp.length)
    expect(data.fetchedAt).toBe('2026-08-17T00:00:00.000Z')
  })
})
