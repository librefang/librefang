#!/usr/bin/env npx tsx
// Build-time script: fetch registry data from GitHub and save as static JSON
// Run: npx tsx scripts/fetch-registry.ts

const API = 'https://api.github.com/repos/librefang/librefang-registry/contents'
const RAW = 'https://raw.githubusercontent.com/librefang/librefang-registry/main'
const HEADERS: Record<string, string> = { Accept: 'application/vnd.github.v3+json' }

// Use token if available to avoid rate limits
const token = process.env.GITHUB_TOKEN
if (token) HEADERS['Authorization'] = `Bearer ${token}`

interface GHItem { name: string; type: string }
interface I18nEntry { name?: string; description?: string }
interface Detail { id: string; name: string; description: string; category: string; icon: string; tags?: string[]; i18n?: Record<string, I18nEntry> }

async function fetchDir(path: string): Promise<GHItem[]> {
  const res = await fetch(`${API}/${path}`, { headers: HEADERS })
  if (!res.ok) {
    // 404 is expected for optional categories (skills, mcp may not exist yet).
    if (res.status !== 404) console.error(`Failed to fetch ${path}: ${res.status}`)
    return []
  }
  const items: GHItem[] = await res.json()
  return items.filter(f => (f.type === 'dir' || f.name.endsWith('.toml')) && f.name !== 'README.md')
}

function parseToml(text: string, fallbackId: string): Detail {
  const get = (key: string) => {
    const m = text.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, 'm'))
    return m ? m[1]! : ''
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
  const kvRe = (k: string) => new RegExp(`^\\s*${k}\\s*=\\s*"((?:[^"\\\\]|\\\\.)*)"`)
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
      if (n && entry.name === undefined) entry.name = n[1]!
      const d = lines[j]!.match(descRe)
      if (d && entry.description === undefined) entry.description = d[1]!
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

async function fetchToml(path: string, fallbackId: string): Promise<Detail | null> {
  const res = await fetch(`${RAW}/${path}`)
  if (!res.ok) return null
  return parseToml(await res.text(), fallbackId)
}

// Skills ship as SKILL.md with YAML frontmatter instead of TOML.
// Only `name` and `description` are guaranteed; id falls back to the
// directory name and category is always "skills".
async function fetchSkillMd(path: string, fallbackId: string): Promise<Detail | null> {
  const res = await fetch(`${RAW}/${path}`)
  if (!res.ok) return null
  const text = await res.text()
  const fm = text.match(/^---\s*\n([\s\S]*?)\n---/)
  if (!fm) return null
  const block = fm[1]!
  const get = (key: string) => {
    const m = block.match(new RegExp(`^${key}\\s*:\\s*"?([^"\\n]*?)"?\\s*$`, 'm'))
    return m ? m[1]!.trim() : ''
  }
  return {
    id: get('id') || fallbackId,
    name: get('name') || fallbackId,
    description: get('description'),
    category: 'skills',
    icon: '',
  }
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

async function main() {
  console.log('Fetching registry data...')

  // The upstream registry is migrating `integrations/` → `mcp/`. Prefer
  // the new name and fall back to the legacy path while the rename rolls
  // out. Track which path we read from so file fetches below use it.
  let mcpFiles = await fetchDir('mcp')
  let mcpDir = 'mcp'
  if (mcpFiles.length === 0) {
    const legacy = await fetchDir('integrations')
    if (legacy.length > 0) {
      console.log(`Using legacy 'integrations/' path (${legacy.length} files) — upstream rename to 'mcp/' not yet merged`)
      mcpFiles = legacy
      mcpDir = 'integrations'
    }
  }
  const [handDirs, channelFiles, providerFiles, workflowFiles, agentDirs, pluginFiles, skillDirs] = await Promise.all([
    fetchDir('hands'),
    fetchDir('channels'),
    fetchDir('providers'),
    fetchDir('workflows'),
    fetchDir('agents'),
    fetchDir('plugins'),
    fetchDir('skills'),
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
    `${plugins.length} plugins, ${skills.length} skills, ${mcp.length} mcp (from ${mcpDir}/)`
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
    fetchBatch(mcp, m => m.name.endsWith('.toml') ? `${mcpDir}/${m.name}` : `${mcpDir}/${m.name}/MCP.toml`),
  ])

  const data = {
    hands: handDetails,
    channels: channelDetails,
    providers: providerDetails,
    workflows: workflowDetails,
    agents: agentDetails,
    plugins: pluginDetails,
    skills: skillDetails,
    mcp: mcpDetails,
    handsCount: hands.length,
    channelsCount: channels.length,
    providersCount: providers.length,
    workflowsCount: workflows.length,
    agentsCount: agents.length,
    pluginsCount: plugins.length,
    skillsCount: skills.length,
    mcpCount: mcp.length,
    fetchedAt: new Date().toISOString(),
  }

  const fs = await import('fs')
  const path = await import('path')
  const outPath = path.join(import.meta.dirname, '..', 'public', 'registry.json')
  fs.writeFileSync(outPath, JSON.stringify(data, null, 2))
  console.log(`Written to ${outPath}`)
}

main().catch(console.error)
