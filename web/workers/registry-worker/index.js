// Registry Worker
// Proxies librefang-registry (GitHub) with Cache API for HTTP responses.
// KV is used only for mutable counters (clicks, errors) and registry_data
// written by cron — everything else is Cache API (free, no quota).

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
}

const REGISTRY_API = 'https://api.github.com/repos/librefang/librefang-registry/contents'
const REGISTRY_RAW = 'https://raw.githubusercontent.com/librefang/librefang-registry/main'

// registry_data is written by cron and must survive cache eviction, so it
// stays in KV. Serve it through Cache API on reads so KV is only hit when
// the cache is cold or stale.
const REGISTRY_CACHE_TTL = 3600        // 1 hour — Cache API max-age
const REGISTRY_STALE_KV_TTL = 86400   // 24 hours — fall back to KV if cache miss

const CATEGORIES = ['hands', 'channels', 'providers', 'workflows', 'agents', 'plugins', 'skills', 'mcp']
const CATEGORY_RE = /^(hands|channels|providers|workflows|agents|plugins|skills|mcp)\//
const ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/i

const CLICK_SHARDS = 8
const ERROR_SHARDS = 4
const ERRORS_MAX_PER_SHARD = 25

export default {
  async fetch(request, env, ctx) {
    if (request.method === 'OPTIONS') return new Response(null, { headers: CORS })

    const url = new URL(request.url)
    const path = url.pathname

    if (path === '/api/registry' && request.method === 'GET')
      return handleRegistry(request, env, ctx, url.searchParams.has('refresh'))

    if (path === '/api/registry/raw' && request.method === 'GET')
      return handleRegistryRaw(request, env, ctx, url.searchParams.get('path') || '')

    if (path === '/api/registry/commit' && request.method === 'GET')
      return handleRegistryCommit(request, env, ctx, url.searchParams.get('path') || '')

    if (path === '/api/registry/click' && request.method === 'POST')
      return handleClick(request, env, ctx)

    if (path === '/api/registry/trending' && request.method === 'GET')
      return handleTrending(request, env, ctx, url.searchParams.get('category') || '')

    if (path === '/api/registry/metrics' && request.method === 'GET')
      return handleMetrics(env)

    if (path === '/api/errors' && request.method === 'POST')
      return handleErrorReport(request, env, ctx)

    if (path === '/api/errors' && request.method === 'GET')
      return handleErrorList(env)

    return new Response('Not Found', { status: 404 })
  },

  async scheduled(_event, env) {
    await refreshRegistryCache(env)
  },
}

// ---------------------------------------------------------------------------
// Registry (stale-while-revalidate via Cache API, fallback to KV)
// ---------------------------------------------------------------------------

async function handleRegistry(request, env, ctx, forceRefresh) {
  const cacheKey = new Request('https://internal/registry_data', request)

  if (!forceRefresh) {
    const cached = await caches.default.match(cacheKey)
    if (cached) {
      // Trigger background refresh if the KV copy is newer than cache
      ctx.waitUntil(maybeRevalidate(env, ctx, cacheKey))
      return addCors(cached)
    }
  }

  // Cache miss — pull from KV (written by cron or inline refresh)
  const kvData = await env.KV.get('registry_data')
  const kvTime = await env.KV.get('registry_data_time')
  const age = kvTime ? (Date.now() - parseInt(kvTime, 10)) / 1000 : Infinity

  if (kvData && age < REGISTRY_STALE_KV_TTL) {
    const response = jsonResponse(kvData, REGISTRY_CACHE_TTL)
    ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
    return response
  }

  // KV also stale — refresh inline
  await refreshRegistryCache(env)
  const fresh = await env.KV.get('registry_data')
  if (fresh) {
    const response = jsonResponse(fresh, REGISTRY_CACHE_TTL)
    ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
    return response
  }

  return new Response(
    JSON.stringify({ error: 'registry unavailable', fetchedAt: new Date().toISOString() }),
    { status: 503, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...CORS } },
  )
}

async function maybeRevalidate(env, ctx, cacheKey) {
  const kvTime = await env.KV.get('registry_data_time')
  if (!kvTime) return
  // If KV was updated more recently than 5 min ago, refresh the cache entry
  if (Date.now() - parseInt(kvTime, 10) < 5 * 60 * 1000) {
    const kvData = await env.KV.get('registry_data')
    if (kvData) await caches.default.put(cacheKey, jsonResponse(kvData, REGISTRY_CACHE_TTL))
  }
}

// ---------------------------------------------------------------------------
// Registry raw files (Cache API, 1h TTL)
// ---------------------------------------------------------------------------

async function handleRegistryRaw(request, env, ctx, rawPath) {
  if (!rawPath || !CATEGORY_RE.test(rawPath) || rawPath.includes('..') || rawPath.includes('\\'))
    return errorResponse('invalid path', 400)

  const cacheKey = new Request(`https://internal/registry_raw/${rawPath}`, request)
  const cached = await caches.default.match(cacheKey)
  if (cached) return addCors(cached)

  const upstream = await fetch(`${REGISTRY_RAW}/${rawPath}`)
  if (!upstream.ok) return errorResponse(`upstream ${upstream.status}`, upstream.status)

  const body = await upstream.text()
  const response = new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'public, max-age=3600', ...CORS },
  })
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

// ---------------------------------------------------------------------------
// Registry commit metadata (Cache API, 6h TTL)
// ---------------------------------------------------------------------------

async function handleRegistryCommit(request, env, ctx, rawPath) {
  if (!rawPath || !CATEGORY_RE.test(rawPath) || rawPath.includes('..'))
    return errorResponse('invalid path', 400)

  const cacheKey = new Request(`https://internal/registry_commit/${rawPath}`, request)
  const cached = await caches.default.match(cacheKey)
  if (cached) return addCors(cached)

  const headers = ghHeaders(env)
  const upstream = await fetch(
    `https://api.github.com/repos/librefang/librefang-registry/commits?path=${encodeURIComponent(rawPath)}&per_page=1`,
    { headers },
  )

  if (!upstream.ok) return errorResponse(`upstream ${upstream.status}`, upstream.status)

  const commits = await upstream.json()
  const first = Array.isArray(commits) && commits.length > 0 ? commits[0] : null
  const result = first
    ? {
        sha: first.sha,
        date: first.commit?.author?.date || first.commit?.committer?.date || null,
        message: (first.commit?.message || '').split('\n')[0].slice(0, 200),
      }
    : { sha: null, date: null, message: null }

  const response = jsonResponse(JSON.stringify(result), 21600) // 6h
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

// ---------------------------------------------------------------------------
// Click tracking (KV — mutable counters, must persist)
// ---------------------------------------------------------------------------

async function handleClick(request, env, ctx) {
  let body
  try { body = await request.json() } catch { return new Response('invalid json', { status: 400, headers: CORS }) }

  const { category, id } = body || {}
  if (!CATEGORIES.includes(category) || !ID_RE.test(id))
    return new Response('invalid payload', { status: 400, headers: CORS })

  const shard = Math.floor(Math.random() * CLICK_SHARDS)
  const key = `registry_clicks:${category}:${shard}`

  ctx.waitUntil((async () => {
    let counts = {}
    try {
      const raw = await env.KV.get(key)
      if (raw) counts = JSON.parse(raw)
    } catch (_) { counts = {} }

    counts[id] = (counts[id] || 0) + 1

    const entries = Object.entries(counts)
    if (entries.length > 500) {
      entries.sort((a, b) => b[1] - a[1])
      counts = Object.fromEntries(entries.slice(0, 500))
    }
    await env.KV.put(key, JSON.stringify(counts))
  })())

  return new Response('{"ok":true}', { headers: { 'Content-Type': 'application/json', ...CORS } })
}

// Trending uses Cache API with short TTL to avoid 8 KV reads per request.
async function handleTrending(request, env, ctx, category) {
  if (!CATEGORIES.includes(category)) return errorResponse('invalid category', 400)

  const cacheKey = new Request(`https://internal/trending/${category}`, request)
  const cached = await caches.default.match(cacheKey)
  if (cached) return addCors(cached)

  const counts = await loadClickTotals(env, category)
  const top = Object.entries(counts)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 10)
    .map(([id, clicks]) => ({ id, clicks }))

  const response = jsonResponse(JSON.stringify({ category, top }), 600) // 10 min
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

async function handleMetrics(env) {
  const perCategory = {}
  const allItems = []

  for (const cat of CATEGORIES) {
    const counts = await loadClickTotals(env, cat)
    let total = 0
    for (const [id, n] of Object.entries(counts)) {
      total += n
      allItems.push({ category: cat, id, clicks: n })
    }
    perCategory[cat] = { total, items: Object.keys(counts).length }
  }

  allItems.sort((a, b) => b.clicks - a.clicks)

  return jsonResponse(JSON.stringify({
    generatedAt: new Date().toISOString(),
    perCategory,
    topOverall: allItems.slice(0, 10),
    totalClicks: allItems.reduce((s, x) => s + x.clicks, 0),
  }), 300)
}

async function loadClickTotals(env, category) {
  const shards = await Promise.all(
    Array.from({ length: CLICK_SHARDS }, (_, i) =>
      env.KV.get(`registry_clicks:${category}:${i}`).catch(() => null),
    ),
  )
  const totals = {}
  for (const raw of shards) {
    if (!raw) continue
    let counts = {}
    try { counts = JSON.parse(raw) } catch (_) { continue }
    for (const [id, n] of Object.entries(counts)) {
      totals[id] = (totals[id] || 0) + (typeof n === 'number' ? n : 0)
    }
  }
  return totals
}

// ---------------------------------------------------------------------------
// UI error reports (KV — mutable log)
// ---------------------------------------------------------------------------

async function handleErrorReport(request, env, ctx) {
  let body
  try { body = await request.json() } catch { return new Response('invalid json', { status: 400, headers: CORS }) }

  const { message, stack, pathname, lang, ua } = body || {}
  if (typeof message !== 'string' || message.length === 0 || message.length > 2000)
    return new Response('invalid payload', { status: 400, headers: CORS })

  const entry = {
    at: new Date().toISOString(),
    message: message.slice(0, 2000),
    stack: typeof stack === 'string' ? stack.slice(0, 4000) : undefined,
    pathname: typeof pathname === 'string' ? pathname.slice(0, 256) : undefined,
    lang: typeof lang === 'string' ? lang.slice(0, 16) : undefined,
    ua: typeof ua === 'string' ? ua.slice(0, 256) : undefined,
  }

  const shard = Math.floor(Math.random() * ERROR_SHARDS)
  const key = `ui_errors:${shard}`

  ctx.waitUntil((async () => {
    let errors = []
    try {
      const raw = await env.KV.get(key)
      if (raw) errors = JSON.parse(raw)
    } catch (_) { errors = [] }
    errors.unshift(entry)
    if (errors.length > ERRORS_MAX_PER_SHARD) errors.length = ERRORS_MAX_PER_SHARD
    await env.KV.put(key, JSON.stringify(errors))
  })())

  return new Response('{"ok":true}', { headers: { 'Content-Type': 'application/json', ...CORS } })
}

async function handleErrorList(env) {
  const shards = await Promise.all([
    ...Array.from({ length: ERROR_SHARDS }, (_, i) =>
      env.KV.get(`ui_errors:${i}`).catch(() => null),
    ),
    env.KV.get('ui_errors').catch(() => null), // legacy key
  ])

  const merged = []
  for (const raw of shards) {
    if (!raw) continue
    try {
      const arr = JSON.parse(raw)
      if (Array.isArray(arr)) merged.push(...arr)
    } catch (_) { continue }
  }

  merged.sort((a, b) => String(b?.at || '').localeCompare(String(a?.at || '')))
  if (merged.length > 100) merged.length = 100

  return new Response(JSON.stringify({ errors: merged }), {
    headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...CORS },
  })
}

// ---------------------------------------------------------------------------
// Registry cache refresh (cron + inline)
// ---------------------------------------------------------------------------

async function refreshRegistryCache(env) {
  const headers = ghHeaders(env)

  async function fetchDir(path) {
    const res = await fetch(`${REGISTRY_API}/${path}`, { headers })
    if (!res.ok) return []
    const items = await res.json()
    return items.filter(f => f.type === 'dir' || (f.name.endsWith('.toml') && f.name !== 'README.md'))
  }

  async function fetchToml(path) {
    const res = await fetch(`${REGISTRY_RAW}/${path}`)
    if (!res.ok) return null
    const text = await res.text()
    const get = key => { const m = text.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, 'm')); return m ? m[1] : '' }

    const i18n = {}
    const i18nRe = /\[i18n\.([a-zA-Z-]+)\]\s*\n(?:([^[]*?)(?=\n\[|\n*$))/g
    let m
    while ((m = i18nRe.exec(text)) !== null) {
      const descM = (m[2] || '').match(/description\s*=\s*"([^"]*)"/)
      if (descM) i18n[m[1]] = { description: descM[1] }
    }

    const tagsM = text.match(/^tags\s*=\s*\[([^\]]*)\]/m)
    const tags = tagsM ? tagsM[1].match(/"([^"]*)"/g)?.map(s => s.replace(/"/g, '')) : undefined

    const result = { id: get('id'), name: get('name'), description: get('description'), category: get('category'), icon: get('icon') }
    if (tags?.length) result.tags = tags
    if (Object.keys(i18n).length) result.i18n = i18n
    return result
  }

  async function fetchSkillMd(path, fallbackId) {
    const res = await fetch(`${REGISTRY_RAW}/${path}`)
    if (!res.ok) return null
    const text = await res.text()
    const fm = text.match(/^---\s*\n([\s\S]*?)\n---/)
    if (!fm) return null
    const get = key => { const m = fm[1].match(new RegExp(`^${key}\\s*:\\s*"?([^"\\n]*?)"?\\s*$`, 'm')); return m ? m[1].trim() : '' }
    return { id: get('id') || fallbackId, name: get('name') || fallbackId, description: get('description'), category: 'skills', icon: '' }
  }

  async function fetchBatch(items, pathFn, fetcher = p => fetchToml(p)) {
    const results = []
    for (let i = 0; i < items.length; i += 10) {
      const batch = await Promise.all(items.slice(i, i + 10).map(item => fetcher(pathFn(item), item.name)))
      results.push(...batch)
    }
    return results.filter(Boolean)
  }

  try {
    const dirs = await Promise.all(
      ['hands', 'channels', 'providers', 'workflows', 'agents', 'plugins', 'skills', 'mcp'].map(fetchDir),
    )
    const [hands, channels, providers, workflows, agents, plugins, skills, mcp] =
      dirs.map(items => items.filter(f => f.name !== 'README.md'))

    const sigOf = items => items.map(i => `${i.name}@${i.sha || ''}`).sort().join(',')
    const signature = ['hands', 'channels', 'providers', 'workflows', 'agents', 'plugins', 'skills', 'mcp']
      .map((cat, i) => `${cat}=${sigOf(dirs[i].filter(f => f.name !== 'README.md'))}`)
      .join('|')

    const existing = await env.KV.get('registry_data')
    if (existing) {
      try {
        if (JSON.parse(existing).signature === signature) {
          await env.KV.put('registry_data_time', String(Date.now()))
          console.log('Registry unchanged, skipping manifest fetch')
          return
        }
      } catch (_) { /* refetch */ }
    }

    const [handDetails, agentDetails, skillDetails, channelDetails, providerDetails, workflowDetails, pluginDetails, mcpDetails] = await Promise.all([
      fetchBatch(hands,     h => `hands/${h.name}/HAND.toml`),
      fetchBatch(agents,    a => `agents/${a.name}/agent.toml`),
      fetchBatch(skills,    s => `skills/${s.name}/SKILL.md`, fetchSkillMd),
      fetchBatch(channels,  c => `channels/${c.name}`),
      fetchBatch(providers, p => `providers/${p.name}`),
      fetchBatch(workflows, w => `workflows/${w.name}`),
      fetchBatch(plugins,   p => `plugins/${p.name}/plugin.toml`),
      fetchBatch(mcp,       m => m.name.endsWith('.toml') ? `mcp/${m.name}` : `mcp/${m.name}/MCP.toml`),
    ])

    const result = {
      hands: handDetails, channels: channelDetails, providers: providerDetails,
      workflows: workflowDetails, agents: agentDetails, plugins: pluginDetails,
      skills: skillDetails, mcp: mcpDetails,
      handsCount: hands.length, channelsCount: channels.length, providersCount: providers.length,
      workflowsCount: workflows.length, agentsCount: agents.length, pluginsCount: plugins.length,
      skillsCount: skills.length, mcpCount: mcp.length,
      fetchedAt: new Date().toISOString(),
      signature,
    }

    await Promise.all([
      env.KV.put('registry_data', JSON.stringify(result)),
      env.KV.put('registry_data_time', String(Date.now())),
    ])
    console.log('Registry refreshed:', hands.length, 'hands,', channels.length, 'channels,',
      agents.length, 'agents,', skills.length, 'skills,', mcp.length, 'mcp')
  } catch (e) {
    console.error('Registry refresh failed:', e.message)
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function ghHeaders(env) {
  const h = { Accept: 'application/vnd.github.v3+json', 'User-Agent': 'LibrefangStats/1.0' }
  if (env.GITHUB_TOKEN) h['Authorization'] = `token ${env.GITHUB_TOKEN}`
  return h
}

function jsonResponse(body, maxAge = 300) {
  return new Response(body, {
    headers: { 'Content-Type': 'application/json', 'Cache-Control': `public, max-age=${maxAge}`, ...CORS },
  })
}

function addCors(response) {
  const r = new Response(response.body, response)
  Object.entries(CORS).forEach(([k, v]) => r.headers.set(k, v))
  return r
}

function errorResponse(message, status = 500) {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS },
  })
}
