import { afterEach, describe, expect, it, vi } from 'vitest'
// @ts-expect-error — Cloudflare worker is plain JavaScript without declarations
import worker from '../workers/registry-worker/index.js'

describe('registry worker stale-cache refresh', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('refreshes an empty registry cache through the repository sync path', async () => {
    const registryText = JSON.stringify({ hands: [{ id: 'researcher' }] })
    const signature = 'A'.repeat(86)
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith('/plugins-index.json')) return new Response('[]')
      if (url.endsWith('/plugins-index.json.sig')) return new Response(signature)
      if (url.endsWith('/registry-index.json')) return new Response(registryText)
      throw new Error(`unexpected URL: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    let registryRow: { value: string; updated_at: number } | null = null
    const db = {
      prepare: vi.fn((sql: string) => {
        if (sql.includes("WHERE key = 'registry_data'")) {
          return { first: vi.fn(async () => registryRow) }
        }
        return {
          bind: (...args: unknown[]) => ({ sql, args }),
        }
      }),
      batch: vi.fn(async (statements: { sql: string; args: unknown[] }[]) => {
        const registryWrite = statements.find(statement =>
          statement.sql.includes("VALUES ('registry_data', ?, ?)"),
        )
        expect(registryWrite).toBeDefined()
        registryRow = {
          value: registryWrite!.args[0] as string,
          updated_at: registryWrite!.args[1] as number,
        }
      }),
    }
    const cache = {
      match: vi.fn(async () => undefined),
      put: vi.fn(async () => undefined),
      delete: vi.fn(async () => true),
    }
    vi.stubGlobal('caches', { default: cache })
    const waits: Promise<unknown>[] = []
    const ctx = { waitUntil: (promise: Promise<unknown>) => waits.push(promise) }

    const response = await worker.fetch(
      new Request('https://registry.librefang.ai/api/registry'),
      { DB: db },
      ctx,
    )
    await Promise.all(waits)

    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ hands: [{ id: 'researcher' }] })
    expect(fetchMock).toHaveBeenCalledTimes(3)
    expect(db.batch).toHaveBeenCalledTimes(1)
  })
})
