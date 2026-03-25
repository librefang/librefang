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
interface I18nEntry { description: string }
interface Detail { id: string; name: string; description: string; category: string; icon: string; i18n?: Record<string, I18nEntry> }

async function fetchDir(path: string): Promise<GHItem[]> {
  const res = await fetch(`${API}/${path}`, { headers: HEADERS })
  if (!res.ok) { console.error(`Failed to fetch ${path}: ${res.status}`); return [] }
  const items: GHItem[] = await res.json()
  return items.filter(f => (f.type === 'dir' || f.name.endsWith('.toml')) && f.name !== 'README.md')
}

function parseToml(text: string): Detail {
  const get = (key: string) => {
    const m = text.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, 'm'))
    return m ? m[1]! : ''
  }
  // Parse i18n sections
  const i18n: Record<string, I18nEntry> = {}
  const i18nRegex = /\[i18n\.([a-zA-Z-]+)\]\s*\n(?:([^[]*?)(?=\n\[|\n*$))/g
  let match
  while ((match = i18nRegex.exec(text)) !== null) {
    const lang = match[1]!
    const block = match[2] || ''
    const descMatch = block.match(/description\s*=\s*"([^"]*)"/)
    if (descMatch) {
      i18n[lang] = { description: descMatch[1]! }
    }
  }
  const result: Detail = { id: get('id'), name: get('name'), description: get('description'), category: get('category'), icon: get('icon') }
  if (Object.keys(i18n).length > 0) result.i18n = i18n
  return result
}

async function fetchToml(path: string): Promise<Detail | null> {
  const res = await fetch(`${RAW}/${path}`)
  if (!res.ok) return null
  return parseToml(await res.text())
}

async function main() {
  console.log('Fetching registry data...')

  const [handDirs, channelFiles, providerFiles, integrationFiles, workflowFiles, agentDirs, pluginFiles] = await Promise.all([
    fetchDir('hands'),
    fetchDir('channels'),
    fetchDir('providers'),
    fetchDir('integrations'),
    fetchDir('workflows'),
    fetchDir('agents'),
    fetchDir('plugins'),
  ])

  const filter = (items: GHItem[]) => items.filter(f => f.name !== 'README.md')
  const hands = filter(handDirs)
  const channels = filter(channelFiles)
  const providers = filter(providerFiles)
  const integrations = filter(integrationFiles)
  const workflows = filter(workflowFiles)
  const agents = filter(agentDirs)
  const plugins = filter(pluginFiles)

  console.log(`Found: ${hands.length} hands, ${channels.length} channels, ${providers.length} providers, ${integrations.length} integrations, ${workflows.length} workflows, ${agents.length} agents, ${plugins.length} plugins`)

  // Fetch details
  const [handDetails, channelDetails] = await Promise.all([
    Promise.all(hands.map(h => fetchToml(`hands/${h.name}/HAND.toml`))),
    Promise.all(channels.map(c => fetchToml(`channels/${c.name}`))),
  ])

  const data = {
    hands: handDetails.filter(Boolean),
    channels: channelDetails.filter(Boolean),
    handsCount: hands.length,
    channelsCount: channels.length,
    providersCount: providers.length,
    integrationsCount: integrations.length,
    workflowsCount: workflows.length,
    agentsCount: agents.length,
    pluginsCount: plugins.length,
    fetchedAt: new Date().toISOString(),
  }

  const fs = await import('fs')
  const path = await import('path')
  const outPath = path.join(import.meta.dirname, '..', 'public', 'registry.json')
  fs.writeFileSync(outPath, JSON.stringify(data, null, 2))
  console.log(`Written to ${outPath}`)
}

main().catch(console.error)
