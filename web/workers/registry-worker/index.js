// Registry Worker
// Proxies librefang-registry (GitHub) with Cache API for HTTP responses.
// Storage: D1 (registry_clicks, kv_store, ui_errors). No KV dependency.
//
// Plugin signing (#3805): when REGISTRY_PRIVATE_KEY (PKCS#8 base64) is set as
// a Worker secret and REGISTRY_PUBLIC_KEY (raw 32-byte base64) is set as a
// var, the cron refresh signs the canonical index.json with Ed25519 and
// stores the signature in kv_store('registry_data_sig'). The daemon
// (librefang-runtime/plugin_manager.rs) fetches:
//   GET /api/registry/index.json     — canonical bytes that were signed
//   GET /api/registry/index.json.sig — base64 Ed25519 signature
//   GET /.well-known/registry-pubkey — base64 raw 32-byte Ed25519 pubkey
// See web/workers/SIGNING.md for keygen + deploy.

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
}

const REGISTRY_API = 'https://api.github.com/repos/librefang/librefang-registry/contents'
const REGISTRY_RAW = 'https://raw.githubusercontent.com/librefang/librefang-registry/main'

const REGISTRY_CACHE_TTL = 3600       // 1h Cache API TTL
const REGISTRY_STALE_TTL = 86400      // 24h — beyond this, refresh inline

const CATEGORIES = ['hands', 'channels', 'providers', 'workflows', 'agents', 'plugins', 'skills', 'mcp']
const CATEGORY_RE = /^(hands|channels|providers|workflows|agents|plugins|skills|mcp)\//
const ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/i

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

    // Signing endpoints (Ed25519). The daemon contract is:
    //   index.json     — canonical bytes that were signed
    //   index.json.sig — base64 Ed25519 signature over those bytes
    //   .well-known/registry-pubkey — raw 32-byte base64 public key
    if (path === '/api/registry/index.json' && request.method === 'GET')
      return handleSignedIndex(env)

    if (path === '/api/registry/index.json.sig' && request.method === 'GET')
      return handleSignedIndexSig(env)

    if (path === '/.well-known/registry-pubkey' && request.method === 'GET')
      return handlePubkey(env)

    return new Response('Not Found', { status: 404 })
  },

  async scheduled(_event, env) {
    await refreshRegistryCache(env)
  },
}

// ---------------------------------------------------------------------------
// Registry (Cache API read, D1 written by cron)
// ---------------------------------------------------------------------------

async function handleRegistry(request, env, ctx, forceRefresh) {
  const cacheKey = new Request('https://internal/registry_data', request)

  if (!forceRefresh) {
    const cached = await caches.default.match(cacheKey)
    if (cached) return addCors(cached)
  }

  const row = await env.DB.prepare(
    `SELECT value, updated_at FROM kv_store WHERE key = 'registry_data'`,
  ).first()

  if (row && (Date.now() / 1000 - row.updated_at) < REGISTRY_STALE_TTL) {
    const response = jsonResponse(row.value, REGISTRY_CACHE_TTL)
    ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
    return response
  }

  await refreshRegistryCache(env)
  const fresh = await env.DB.prepare(
    `SELECT value FROM kv_store WHERE key = 'registry_data'`,
  ).first()

  if (fresh) {
    const response = jsonResponse(fresh.value, REGISTRY_CACHE_TTL)
    ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
    return response
  }

  return new Response(
    JSON.stringify({ error: 'registry unavailable', fetchedAt: new Date().toISOString() }),
    { status: 503, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...CORS } },
  )
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

  const upstream = await fetch(
    `https://api.github.com/repos/librefang/librefang-registry/commits?path=${encodeURIComponent(rawPath)}&per_page=1`,
    { headers: ghHeaders(env) },
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

  const response = jsonResponse(JSON.stringify(result), 21600)
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

// ---------------------------------------------------------------------------
// Click tracking (D1 atomic upsert)
// ---------------------------------------------------------------------------

async function handleClick(request, env, ctx) {
  let body
  try { body = await request.json() } catch { return new Response('invalid json', { status: 400, headers: CORS }) }

  const { category, id } = body || {}
  if (!CATEGORIES.includes(category) || !ID_RE.test(id))
    return new Response('invalid payload', { status: 400, headers: CORS })

  ctx.waitUntil(
    env.DB.prepare(
      `INSERT INTO registry_clicks (category, item_id, count) VALUES (?, ?, 1)
       ON CONFLICT(category, item_id) DO UPDATE SET count = count + 1`,
    ).bind(category, id).run(),
  )

  return new Response('{"ok":true}', { headers: { 'Content-Type': 'application/json', ...CORS } })
}

async function handleTrending(request, env, ctx, category) {
  if (!CATEGORIES.includes(category)) return errorResponse('invalid category', 400)

  const cacheKey = new Request(`https://internal/trending/${category}`, request)
  const cached = await caches.default.match(cacheKey)
  if (cached) return addCors(cached)

  const rows = await env.DB.prepare(
    `SELECT item_id as id, count as clicks FROM registry_clicks
     WHERE category = ? ORDER BY count DESC LIMIT 10`,
  ).bind(category).all()

  const response = jsonResponse(JSON.stringify({ category, top: rows.results }), 600)
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

async function handleMetrics(env) {
  const [perCatRows, topRows, totalRow] = await Promise.all([
    env.DB.prepare(
      `SELECT category, SUM(count) as total, COUNT(*) as items
       FROM registry_clicks GROUP BY category`,
    ).all(),
    env.DB.prepare(
      `SELECT category, item_id as id, count as clicks
       FROM registry_clicks ORDER BY count DESC LIMIT 10`,
    ).all(),
    env.DB.prepare(`SELECT SUM(count) as total FROM registry_clicks`).first(),
  ])

  const perCategory = {}
  for (const row of perCatRows.results) {
    perCategory[row.category] = { total: row.total, items: row.items }
  }

  return jsonResponse(JSON.stringify({
    generatedAt: new Date().toISOString(),
    perCategory,
    topOverall: topRows.results,
    totalClicks: totalRow?.total ?? 0,
  }), 300)
}

// ---------------------------------------------------------------------------
// UI error reports (D1)
// ---------------------------------------------------------------------------

async function handleErrorReport(request, env, ctx) {
  let body
  try { body = await request.json() } catch { return new Response('invalid json', { status: 400, headers: CORS }) }

  const { message, stack, pathname, lang, ua } = body || {}
  if (typeof message !== 'string' || message.length === 0 || message.length > 2000)
    return new Response('invalid payload', { status: 400, headers: CORS })

  ctx.waitUntil((async () => {
    await env.DB.prepare(
      `INSERT INTO ui_errors (at, message, stack, pathname, lang, ua)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).bind(
      new Date().toISOString(),
      message.slice(0, 2000),
      typeof stack === 'string' ? stack.slice(0, 4000) : null,
      typeof pathname === 'string' ? pathname.slice(0, 256) : null,
      typeof lang === 'string' ? lang.slice(0, 16) : null,
      typeof ua === 'string' ? ua.slice(0, 256) : null,
    ).run()
    await env.DB.prepare(
      `DELETE FROM ui_errors WHERE at < datetime('now', '-30 days')`,
    ).run()
  })())

  return new Response('{"ok":true}', { headers: { 'Content-Type': 'application/json', ...CORS } })
}

async function handleErrorList(env) {
  const rows = await env.DB.prepare(
    `SELECT * FROM ui_errors ORDER BY at DESC LIMIT 100`,
  ).all()

  return new Response(JSON.stringify({ errors: rows.results }), {
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

    const existing = await env.DB.prepare(
      `SELECT value FROM kv_store WHERE key = 'registry_data'`,
    ).first()
    if (existing) {
      try {
        if (JSON.parse(existing.value).signature === signature) {
          await env.DB.prepare(
            `UPDATE kv_store SET updated_at = ? WHERE key = 'registry_data'`,
          ).bind(Math.floor(Date.now() / 1000)).run()
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

    const indexJson = JSON.stringify(result)
    const now = Math.floor(Date.now() / 1000)

    await env.DB.prepare(
      `INSERT INTO kv_store (key, value, updated_at) VALUES ('registry_data', ?, ?)
       ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
    ).bind(indexJson, now).run()

    // Sign the canonical index bytes if a private key is configured. Signing
    // is best-effort — if the secret is missing we skip (the daemon can still
    // fetch the index but signature verification will fail closed at its end).
    try {
      const sig = await signWithRegistryKey(env, new TextEncoder().encode(indexJson))
      if (sig) {
        await env.DB.prepare(
          `INSERT INTO kv_store (key, value, updated_at) VALUES ('registry_data_sig', ?, ?)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
        ).bind(sig, now).run()
      }
    } catch (e) {
      console.error('Registry signing failed:', e.message)
    }

    console.log('Registry refreshed:', hands.length, 'hands,', channels.length, 'channels,',
      agents.length, 'agents,', skills.length, 'skills,', mcp.length, 'mcp')
  } catch (e) {
    console.error('Registry refresh failed:', e.message)
  }
}

// ---------------------------------------------------------------------------
// Signed-index endpoints (Ed25519)
// ---------------------------------------------------------------------------

async function handleSignedIndex(env) {
  const row = await env.DB.prepare(
    `SELECT value FROM kv_store WHERE key = 'registry_data'`,
  ).first()
  if (!row) return errorResponse('index not yet built — try again after the next cron tick', 503)
  return new Response(row.value, {
    headers: {
      'Content-Type': 'application/json',
      'Cache-Control': `public, max-age=${REGISTRY_CACHE_TTL}`,
      ...CORS,
    },
  })
}

async function handleSignedIndexSig(env) {
  const row = await env.DB.prepare(
    `SELECT value FROM kv_store WHERE key = 'registry_data_sig'`,
  ).first()
  if (!row) return errorResponse('signature not available — REGISTRY_PRIVATE_KEY may be unset', 503)
  return new Response(row.value, {
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': `public, max-age=${REGISTRY_CACHE_TTL}`,
      ...CORS,
    },
  })
}

function handlePubkey(env) {
  const pub = (env.REGISTRY_PUBLIC_KEY || '').trim()
  if (!pub) return errorResponse('public key not configured', 503)
  return new Response(pub, {
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': 'public, max-age=86400',
      ...CORS,
    },
  })
}

// Sign `bytes` with the configured PKCS#8 private key. Returns base64
// signature, or null when no key is configured. Throws on malformed key.
async function signWithRegistryKey(env, bytes) {
  const pkcs8B64 = (env.REGISTRY_PRIVATE_KEY || '').trim()
  if (!pkcs8B64) return null
  const pkcs8 = bytesFromB64(pkcs8B64)
  const key = await crypto.subtle.importKey(
    'pkcs8', pkcs8, { name: 'Ed25519' }, false, ['sign'],
  )
  const sig = await crypto.subtle.sign({ name: 'Ed25519' }, key, bytes)
  return b64FromBytes(new Uint8Array(sig))
}

function b64FromBytes(bytes) {
  let s = ''
  for (const b of bytes) s += String.fromCharCode(b)
  return btoa(s)
}

function bytesFromB64(b64) {
  const bin = atob(b64.replace(/\s+/g, ''))
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
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
