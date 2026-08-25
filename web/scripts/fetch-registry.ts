#!/usr/bin/env npx tsx
// Build-time script: fetch registry data from GitHub and save as static JSON
// Run: npx tsx scripts/fetch-registry.ts

import { writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const API = 'https://api.github.com/repos/librefang/librefang-registry/contents'
const RAW = 'https://raw.githubusercontent.com/librefang/librefang-registry/main'
const HEADERS: Record<string, string> = { Accept: 'application/vnd.github.v3+json' }
const REQUEST_TIMEOUT_MS = 15_000
const REQUEST_RETRIES = 2
const RETRY_DELAY_MS = 250

// Use token if available to avoid rate limits
const token = process.env.GITHUB_TOKEN
if (token) HEADERS['Authorization'] = `Bearer ${token}`

interface GHItem { name: string; type: string }
interface I18nEntry { name?: string; description?: string }
export interface Detail { id: string; name: string; description: string; category: string; icon: string; tags?: string[]; i18n?: Record<string, I18nEntry> }

interface RequestOptions {
  fetchImpl?: typeof fetch
  retries?: number
  retryDelayMs?: number
  timeoutMs?: number
}

type RequestFn = (url: string, init?: RequestInit) => Promise<Response>

function delay(ms: number): Promise<void> {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, ms))
}

function retryableStatus(status: number): boolean {
  return status === 408 || status === 429 || status >= 500
}

export async function fetchWithRetry(
  url: string,
  init: RequestInit = {},
  options: RequestOptions = {},
): Promise<Response> {
  const fetchImpl = options.fetchImpl ?? fetch
  const retries = options.retries ?? REQUEST_RETRIES
  const retryDelayMs = options.retryDelayMs ?? RETRY_DELAY_MS
  const timeoutMs = options.timeoutMs ?? REQUEST_TIMEOUT_MS
  let lastError: unknown

  for (let attempt = 0; attempt <= retries; attempt++) {
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), timeoutMs)
    try {
      const response = await fetchImpl(url, { ...init, signal: controller.signal })
      if (!retryableStatus(response.status) || attempt === retries) return response
      await response.body?.cancel()
      lastError = new Error(`HTTP ${response.status}`)
    } catch (error) {
      lastError = error
      if (attempt === retries) throw error
    } finally {
      clearTimeout(timeout)
    }
    if (retryDelayMs > 0) await delay(retryDelayMs * (attempt + 1))
  }

  throw lastError instanceof Error ? lastError : new Error('Registry request failed')
}

async function responseError(label: string, response: Response): Promise<Error> {
  const body = (await response.text()).trim().slice(0, 500)
  const detail = body ? `: ${body}` : ''
  return new Error(`Failed to fetch ${label}: HTTP ${response.status}${detail}`)
}

export async function fetchDir(
  path: string,
  request: RequestFn = fetchWithRetry,
  allowMissing = false,
): Promise<GHItem[]> {
  const res = await request(`${API}/${path}`, { headers: HEADERS })
  if (!res.ok) {
    if (res.status === 404 && allowMissing) return []
    throw await responseError(path, res)
  }
  const items: GHItem[] = await res.json()
  return items.filter(f => (f.type === 'dir' || f.name.endsWith('.toml')) && f.name !== 'README.md')
}

function escapeRegExp(value: string): string {
  const special = '\\^$.*+?()[]{}|'
  return [...value].map((character) => special.includes(character) ? '\\' + character : character).join('')
}

function decodeDoubleQuoted(value: string): string {
  try {
    return JSON.parse('"' + value + '"') as string
  } catch {
    return value
  }
}

export function parseToml(text: string, fallbackId: string): Detail {
  const get = (key: string) => {
    const pattern = '^' + escapeRegExp(key) + '\\s*=\\s*"((?:[^"\\\\]|\\\\.)*)"'
    const m = text.match(new RegExp(pattern, 'm'))
    return m ? decodeDoubleQuoted(m[1]!) : ''
  }
  // Parse i18n sections — capture both name and description so the
  // card title localizes (not just the blurb). Line-oriented on
  // purpose: a regex that captures "everything between two headers"
  // breaks when a value inside the block contains a `[` character
  // (e.g. `tags = ["popular"]`).
  const i18n: Record<string, I18nEntry> = {}
  const lines = text.split(/\r?\n/)
  // Match only top-level [i18n.<lang>] headers (no dots in the lang
  // token), so we ignore nested [i18n.zh.agents.main] subsections.
  const headerRe = /^\[i18n\.([a-zA-Z-]+)\]\s*$/
  const anyHeaderRe = /^\[/
  const kvRe = (k: string) =>
    new RegExp('^\\s*' + escapeRegExp(k) + '\\s*=\\s*"((?:[^"\\\\]|\\\\.)*)"')
  const nameRe = kvRe('name')
  const descRe = kvRe('description')
  for (let i = 0; i < lines.length; i++) {
    const h = lines[i]!.match(headerRe)
    if (!h) continue
    const lang = h[1]!
    const entry: I18nEntry = {}
    for (let j = i + 1; j < lines.length; j++) {
      if (anyHeaderRe.test(lines[j]!)) break
      const n = lines[j]!.match(nameRe)
      if (n && entry.name === undefined) entry.name = decodeDoubleQuoted(n[1]!)
      const d = lines[j]!.match(descRe)
      if (d && entry.description === undefined) entry.description = decodeDoubleQuoted(d[1]!)
    }
    if (entry.name || entry.description) i18n[lang] = entry
  }
  // Parse tags = ["popular", ...]
  const tagsMatch = text.match(/^tags\s*=\s*\[([^\]]*)\]/m)
  const tags = tagsMatch ? tagsMatch[1]!.match(/"([^"]*)"/g)?.map(s => s.replace(/"/g, '')) : undefined

  const result: Detail = {
    id: get('id') || fallbackId,
    name: get('name') || fallbackId,
    description: get('description'),
    category: get('category'),
    icon: get('icon'),
  }
  if (tags && tags.length > 0) result.tags = tags
  if (Object.keys(i18n).length > 0) result.i18n = i18n
  return result
}

export async function fetchToml(
  path: string,
  fallbackId: string,
  request: RequestFn = fetchWithRetry,
): Promise<Detail | null> {
  const res = await request(`${RAW}/${path}`)
  if (res.status === 404) return null
  if (!res.ok) throw await responseError(path, res)
  return parseToml(await res.text(), fallbackId)
}

// Skills ship as SKILL.md with YAML frontmatter instead of TOML.
// Only `name` and `description` are guaranteed; id falls back to the
// directory name and category is always "skills".
export function parseSkillMd(text: string, fallbackId: string): Detail | null {
  const fm = text.match(/^---\s*\n([\s\S]*?)\n---/)
  if (!fm) return null
  const block = fm[1]!
  const get = (key: string) => {
    for (const line of block.split(/\r?\n/)) {
      const match = line.match(/^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$/)
      if (!match || match[1] !== key) continue
      const value = match[2]!.trim()
      if (value.startsWith('"') && value.endsWith('"')) {
        return decodeDoubleQuoted(value.slice(1, -1))
      }
      if (value.startsWith("'") && value.endsWith("'")) {
        return value.slice(1, -1).replace(/''/g, "'")
      }
      return value
    }
    return ''
  }
  return {
    id: get('id') || fallbackId,
    name: get('name') || fallbackId,
    description: get('description'),
    category: 'skills',
    icon: '',
  }
}

export async function fetchSkillMd(
  path: string,
  fallbackId: string,
  request: RequestFn = fetchWithRetry,
): Promise<Detail | null> {
  const res = await request(`${RAW}/${path}`)
  if (res.status === 404) return null
  if (!res.ok) throw await responseError(path, res)
  return parseSkillMd(await res.text(), fallbackId)
}

type Fetcher = (path: string, fallbackId: string) => Promise<Detail | null>

async function fetchBatch(
  items: GHItem[],
  resolvePath: (item: GHItem) => string,
  fetcher: Fetcher = fetchToml,
): Promise<Detail[]> {
  const out: Detail[] = []
  for (let i = 0; i < items.length; i += 10) {
    const slice = items.slice(i, i + 10)
    const details = await Promise.all(slice.map(item => {
      const id = item.name.endsWith('.toml') ? item.name.replace(/\.toml$/, '') : item.name
      return fetcher(resolvePath(item), id)
    }))
    for (const d of details) if (d) out.push(d)
  }
  return out
}

interface RegistryDetails {
  hands: Detail[]
  channels: Detail[]
  providers: Detail[]
  workflows: Detail[]
  agents: Detail[]
  plugins: Detail[]
  skills: Detail[]
  mcp: Detail[]
}

export function createRegistryData(
  details: RegistryDetails,
  fetchedAt = new Date().toISOString(),
) {
  return {
    ...details,
    handsCount: details.hands.length,
    channelsCount: details.channels.length,
    providersCount: details.providers.length,
    workflowsCount: details.workflows.length,
    agentsCount: details.agents.length,
    pluginsCount: details.plugins.length,
    skillsCount: details.skills.length,
    mcpCount: details.mcp.length,
    fetchedAt,
  }
}

async function main() {
  console.log('Fetching registry data...')

  const [handDirs, channelFiles, providerFiles, workflowFiles, agentDirs, pluginFiles, skillDirs, mcpFiles] = await Promise.all([
    fetchDir('hands'),
    fetchDir('channels'),
    fetchDir('providers'),
    fetchDir('workflows'),
    fetchDir('agents'),
    fetchDir('plugins'),
    fetchDir('skills', fetchWithRetry, true),
    fetchDir('mcp', fetchWithRetry, true),
  ])

  const filter = (items: GHItem[]) => items.filter(f => f.name !== 'README.md')
  const hands = filter(handDirs)
  const channels = filter(channelFiles)
  const providers = filter(providerFiles)
  const workflows = filter(workflowFiles)
  const agents = filter(agentDirs)
  const plugins = filter(pluginFiles)
  const skills = filter(skillDirs)
  const mcp = filter(mcpFiles)

  console.log(
    `Found: ${hands.length} hands, ${channels.length} channels, ${providers.length} providers, ` +
    `${workflows.length} workflows, ${agents.length} agents, ` +
    `${plugins.length} plugins, ${skills.length} skills, ${mcp.length} mcp`
  )

  // Fetch manifest details for all categories in parallel.
  const [handDetails, agentDetails, skillDetails, channelDetails, providerDetails, workflowDetails, pluginDetails, mcpDetails] = await Promise.all([
    fetchBatch(hands, h => `hands/${h.name}/HAND.toml`),
    fetchBatch(agents, a => `agents/${a.name}/agent.toml`),
    fetchBatch(skills, s => `skills/${s.name}/SKILL.md`, fetchSkillMd),
    fetchBatch(channels, c => `channels/${c.name}`),
    fetchBatch(providers, p => `providers/${p.name}`),
    fetchBatch(workflows, w => `workflows/${w.name}`),
    fetchBatch(plugins, p => `plugins/${p.name}/plugin.toml`),
    fetchBatch(mcp, m => m.name.endsWith('.toml') ? `mcp/${m.name}` : `mcp/${m.name}/MCP.toml`),
  ])

  const data = createRegistryData({
    hands: handDetails,
    channels: channelDetails,
    providers: providerDetails,
    workflows: workflowDetails,
    agents: agentDetails,
    plugins: pluginDetails,
    skills: skillDetails,
    mcp: mcpDetails,
  })

  const outPath = resolve(import.meta.dirname, '..', 'public', 'registry.json')
  writeFileSync(outPath, JSON.stringify(data, null, 2))
  console.log(`Written to ${outPath}`)
}

const entrypoint = process.argv[1]
if (entrypoint && import.meta.url === pathToFileURL(resolve(entrypoint)).href) {
  main().catch((error: unknown) => {
    console.error(error)
    process.exitCode = 1
  })
}
