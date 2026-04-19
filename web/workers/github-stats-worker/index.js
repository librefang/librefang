// GitHub Stats Worker
// Optimized: stores history as single JSON blob to minimize KV operations
// Includes one-time migration from old individual KV keys (stars_YYYY-MM-DD)

export default {
  async fetch(request, env, ctx) {
    return handleFetch(request, env, ctx)
  },

  async scheduled(event, env, ctx) {
    ctx.waitUntil(recordDailyStats(env))
    ctx.waitUntil(refreshRegistryCache(env))
  },
}

// Migrate old individual KV keys (stars_YYYY-MM-DD, forks_YYYY-MM-DD, etc.)
// into the stats_history blob. Runs once when blob has < 7 entries.
async function migrateOldKeys(env, history) {
  if (history.length >= 7) return history

  const migrated = await env.KV.get('stats_migration_done')
  if (migrated) return history

  const existingDates = new Set(history.map(h => h.date))
  const newEntries = []

  // Read old individual keys for last 90 days
  for (let i = 0; i < 90; i++) {
    const d = new Date()
    d.setDate(d.getDate() - i)
    const dateStr = d.toISOString().split('T')[0]

    if (existingDates.has(dateStr)) continue

    const stars = await env.KV.get('stars_' + dateStr)
    if (stars) {
      const forks = await env.KV.get('forks_' + dateStr)
      const issues = await env.KV.get('issues_' + dateStr)
      const prs = await env.KV.get('prs_' + dateStr)
      newEntries.push({
        date: dateStr,
        stars: parseInt(stars, 10),
        forks: forks ? parseInt(forks, 10) : 0,
        issues: issues ? parseInt(issues, 10) : 0,
        prs: prs ? parseInt(prs, 10) : 0,
      })
    }
  }

  if (newEntries.length > 0) {
    history = [...history, ...newEntries]
    history.sort((a, b) => a.date.localeCompare(b.date))
    // Deduplicate by date (keep latest)
    const seen = new Map()
    for (const entry of history) {
      seen.set(entry.date, entry)
    }
    history = Array.from(seen.values())
    if (history.length > 90) {
      history = history.slice(-90)
    }
    await env.KV.put('stats_history', JSON.stringify(history))
    console.log('Migration: merged', newEntries.length, 'old entries into blob')
  }

  // Mark migration done so we don't re-scan
  await env.KV.put('stats_migration_done', '1')
  return history
}

async function recordDailyStats(env) {
  const headers = {
    'Accept': 'application/vnd.github.v3+json',
    'User-Agent': 'LibrefangStats/1.0',
  }

  if (env.GITHUB_TOKEN) {
    headers['Authorization'] = `token ${env.GITHUB_TOKEN}`
  }

  try {
    const [repoRes, pullsRes] = await Promise.all([
      fetch('https://api.github.com/repos/librefang/librefang', { headers }),
      fetch('https://api.github.com/repos/librefang/librefang/pulls?state=open&per_page=1', { headers }),
    ])

    if (repoRes.ok) {
      const data = await repoRes.json()
      const today = new Date().toISOString().split('T')[0]

      const prLink = pullsRes.headers.get('link')
      let prCount = 0
      if (prLink) {
        const match = prLink.match(/page=(\d+)>.*rel="last"/)
        if (match) prCount = parseInt(match[1], 10)
      }

      const todayEntry = {
        date: today,
        stars: data.stargazers_count || 0,
        forks: data.forks_count || 0,
        issues: data.open_issues_count || 0,
        prs: prCount,
      }

      // Read existing history blob, append today, trim to 90 days
      let history = []
      try {
        const raw = await env.KV.get('stats_history')
        if (raw) history = JSON.parse(raw)
      } catch (e) { console.log('KV read error:', e.message) }

      // Run migration if needed
      history = await migrateOldKeys(env, history)

      // Replace or append today's entry
      const idx = history.findIndex(h => h.date === today)
      if (idx >= 0) {
        history[idx] = todayEntry
      } else {
        history.push(todayEntry)
      }

      // Keep last 90 days max
      if (history.length > 90) {
        history = history.slice(-90)
      }

      await env.KV.put('stats_history', JSON.stringify(history))
      console.log('Recorded:', today, 'stars:', todayEntry.stars, 'forks:', todayEntry.forks)
    }
  } catch (e) {
    console.error('Failed to record stats:', e.message)
  }
}

function handleFetch(request, env, ctx) {
  const url = new URL(request.url)
  const path = url.pathname

  const cors = {
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type',
  }

  if (request.method === 'OPTIONS') {
    return new Response(null, { headers: cors })
  }

  if (path === '/api/github' && request.method === 'GET') {
    const forceRefresh = url.searchParams.has('refresh')
    return handleGitHubStats(env, cors, forceRefresh)
  }

  if (path === '/api/registry' && request.method === 'GET') {
    const forceRefresh = url.searchParams.has('refresh')
    return handleRegistry(env, cors, ctx, forceRefresh)
  }

  if (path === '/api/registry/raw' && request.method === 'GET') {
    return handleRegistryRaw(env, cors, url.searchParams.get('path') || '')
  }

  if (path === '/api/registry/commit' && request.method === 'GET') {
    return handleRegistryCommit(env, cors, url.searchParams.get('path') || '')
  }

  if (path === '/api/registry/click' && request.method === 'POST') {
    return handleRegistryClick(env, cors, request, ctx)
  }

  if (path === '/api/registry/trending' && request.method === 'GET') {
    return handleRegistryTrending(env, cors, url.searchParams.get('category') || '')
  }

  if (path === '/api/registry/metrics' && request.method === 'GET') {
    return handleRegistryMetrics(env, cors)
  }

  if (path === '/api/errors' && request.method === 'POST') {
    return handleErrorReport(env, cors, request, ctx)
  }

  if (path === '/api/errors' && request.method === 'GET') {
    return handleErrorList(env, cors)
  }

  if (path === '/api/releases' && request.method === 'GET') {
    return handleReleases(env, cors)
  }

  return new Response('Not Found', { status: 404 })
}

// ─── Registry raw-file proxy (GitHub raw) ───
// Serves individual TOML / MD files out of librefang-registry so browsers
// don't have to hit raw.githubusercontent.com directly (same 403 risk,
// plus caching and CORS headers we control).
async function handleRegistryRaw(env, cors, rawPath) {
  // Allowlist: only the categories we actually expose, plus README.
  const allowedTop = /^(hands|channels|providers|workflows|agents|plugins|skills|mcp)\//
  // Reject path traversal or anything not matching the allowlist.
  if (!rawPath || !allowedTop.test(rawPath) || rawPath.includes('..') || rawPath.includes('\\')) {
    return new Response(JSON.stringify({ error: 'invalid path' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  }

  const cacheKey = `registry_raw:${rawPath}`
  const cacheTimeKey = `${cacheKey}:time`
  const fresh = 1000 * 60 * 60     // 1h
  const stale = 1000 * 60 * 60 * 24 // 24h upper bound

  try {
    const [cached, cacheTimeRaw] = await Promise.all([
      env.KV.get(cacheKey),
      env.KV.get(cacheTimeKey),
    ])
    const cacheTime = parseInt(cacheTimeRaw || '0', 10)
    const age = cacheTime ? Date.now() - cacheTime : Infinity

    if (cached && age < fresh) {
      return new Response(cached, {
        headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'public, max-age=3600', ...cors }
      })
    }

    // Fetch fresh. If it fails and we have a stale cache, serve that instead
    // of returning an error.
    const upstream = await fetch(`https://raw.githubusercontent.com/librefang/librefang-registry/main/${rawPath}`)
    if (!upstream.ok) {
      if (cached && age < stale) {
        return new Response(cached, {
          headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'public, max-age=60', ...cors }
        })
      }
      return new Response(JSON.stringify({ error: `upstream ${upstream.status}` }), {
        status: upstream.status,
        headers: { 'Content-Type': 'application/json', ...cors }
      })
    }

    const body = await upstream.text()
    // 1 MiB cap — individual registry entries should be tiny.
    if (body.length < 1024 * 1024) {
      await Promise.all([
        env.KV.put(cacheKey, body, { expirationTtl: 60 * 60 * 24 * 7 }),
        env.KV.put(cacheTimeKey, String(Date.now()), { expirationTtl: 60 * 60 * 24 * 7 }),
      ])
    }

    return new Response(body, {
      headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'public, max-age=3600', ...cors }
    })
  } catch (e) {
    const cached = await env.KV.get(cacheKey)
    if (cached) {
      return new Response(cached, {
        headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'public, max-age=60', ...cors }
      })
    }
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500,
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}

async function handleGitHubStats(env, cors, forceRefresh = false) {
  const cacheKey = 'github_stats'
  const cacheTimeKey = 'github_stats_time'
  const cacheDuration = 1000 * 60 * 30 // 30 minutes

  try {
    // Check cache (2 KV reads) - skip if force refresh
    if (!forceRefresh) {
      let cached, cacheTime
      try {
        cached = await env.KV.get(cacheKey)
        cacheTime = parseInt(await env.KV.get(cacheTimeKey) || '0', 10)
      } catch (e) {
        console.log('KV get error:', e.message)
      }

      if (cached && cacheTime && (Date.now() - cacheTime < cacheDuration)) {
        return new Response(cached, {
          headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=300', ...cors }
        })
      }
    }

    // Fetch from GitHub (3 API calls)
    const headers = {
      'Accept': 'application/vnd.github.v3+json',
      'User-Agent': 'LibrefangStats/1.0',
    }

    if (env.GITHUB_TOKEN) {
      headers['Authorization'] = `token ${env.GITHUB_TOKEN}`
    }

    const [repoRes, releasesRes, pullsRes] = await Promise.all([
      fetch('https://api.github.com/repos/librefang/librefang', { headers }),
      fetch('https://api.github.com/repos/librefang/librefang/releases?per_page=10', { headers }),
      fetch('https://api.github.com/repos/librefang/librefang/pulls?state=open&per_page=1', { headers }),
    ])

    const repo = repoRes.ok ? await repoRes.json() : {}
    const releases = releasesRes.ok ? await releasesRes.json() : []

    const prLink = pullsRes.headers.get('link')
    let prCount = 0
    if (prLink) {
      const match = prLink.match(/page=(\d+)>.*rel="last"/)
      if (match) prCount = parseInt(match[1], 10)
    }

    const downloads = releases.reduce((sum, rel) => {
      return sum + (rel.assets?.reduce((s, a) => s + (a.download_count || 0), 0) || 0)
    }, 0)

    // Update today in history blob (1 KV read + 1 KV write)
    const today = new Date().toISOString().split('T')[0]
    const todayEntry = {
      date: today,
      stars: repo.stargazers_count || 0,
      forks: repo.forks_count || 0,
      issues: repo.open_issues_count || 0,
      prs: prCount,
    }

    let history = []
    try {
      const raw = await env.KV.get('stats_history')
      if (raw) history = JSON.parse(raw)
    } catch (e) { console.log('KV read error:', e.message) }

    // Run migration if needed
    history = await migrateOldKeys(env, history)

    const idx = history.findIndex(h => h.date === today)
    if (idx >= 0) {
      history[idx] = todayEntry
    } else {
      history.push(todayEntry)
    }
    if (history.length > 90) {
      history = history.slice(-90)
    }

    await env.KV.put('stats_history', JSON.stringify(history))

    // Return last 30 days
    const last30 = history.slice(-30)

    const result = {
      stars: repo.stargazers_count || 0,
      forks: repo.forks_count || 0,
      issues: repo.open_issues_count || 0,
      prs: prCount,
      lastUpdate: repo.updated_at || '',
      createdAt: repo.created_at || '',
      downloads,
      starHistory: last30,
    }

    const json = JSON.stringify(result)

    // Cache result (2 KV writes)
    try {
      await env.KV.put(cacheKey, json)
      await env.KV.put(cacheTimeKey, String(Date.now()))
    } catch (e) {
      console.log('KV put error:', e.message)
    }

    return new Response(json, {
      headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=300', ...cors }
    })
  } catch (e) {
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500,
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}

// ─── Releases proxy with KV cache (30 min) ───
async function handleReleases(env, cors) {
  const cacheKey = 'releases_data'
  const cacheTimeKey = 'releases_data_time'
  const cacheDuration = 1000 * 60 * 30

  try {
    const [cached, cacheTime] = await Promise.all([
      env.KV.get(cacheKey),
      env.KV.get(cacheTimeKey),
    ])
    if (cached && cacheTime && (Date.now() - parseInt(cacheTime, 10) < cacheDuration)) {
      return new Response(cached, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=300', ...cors }
      })
    }

    const headers = {
      'Accept': 'application/vnd.github.v3+json',
      'User-Agent': 'LibrefangStats/1.0',
    }
    if (env.GITHUB_TOKEN) headers['Authorization'] = `token ${env.GITHUB_TOKEN}`

    const res = await fetch('https://api.github.com/repos/librefang/librefang/releases?per_page=20', { headers })
    if (!res.ok) throw new Error(`GitHub API returned ${res.status}`)

    const json = await res.text()
    await Promise.all([
      env.KV.put(cacheKey, json),
      env.KV.put(cacheTimeKey, String(Date.now())),
    ])

    return new Response(json, {
      headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=300', ...cors }
    })
  } catch (e) {
    const stale = await env.KV.get(cacheKey)
    if (stale) {
      return new Response(stale, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=60', ...cors }
      })
    }
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500, headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}

// ─── Registry proxy (stale-while-revalidate) ───
// Fresh KV cache (< FRESH_TTL): served directly.
// Stale KV cache (FRESH_TTL..MAX_AGE): served immediately + triggers a
//   background full refresh via ctx.waitUntil so the NEXT request is fresh.
// Missing cache: do a full refresh inline so the first visitor gets real data
//   instead of a degraded names-only snapshot. Daily cron is now just a
//   safety net for when the site has zero traffic for a long time.
const REGISTRY_API = 'https://api.github.com/repos/librefang/librefang-registry/contents'
const FRESH_TTL = 1000 * 60 * 60        // 1 hour — serve directly from KV
const MAX_STALE = 1000 * 60 * 60 * 24   // beyond this, don't even serve stale

async function handleRegistry(env, cors, ctx, forceRefresh = false) {
  const cacheKey = 'registry_data'
  const cacheTimeKey = 'registry_data_time'

  try {
    const [cached, cacheTimeRaw] = await Promise.all([
      env.KV.get(cacheKey),
      env.KV.get(cacheTimeKey),
    ])
    const cacheTime = parseInt(cacheTimeRaw || '0', 10)
    const age = cacheTime ? Date.now() - cacheTime : Infinity

    // Fresh — return as-is.
    if (cached && !forceRefresh && age < FRESH_TTL) {
      return new Response(cached, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=600', ...cors }
      })
    }

    // Stale but usable — serve immediately, refresh in background.
    if (cached && !forceRefresh && age < MAX_STALE) {
      if (ctx && typeof ctx.waitUntil === 'function') {
        ctx.waitUntil(refreshRegistryCache(env))
      }
      return new Response(cached, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=60', ...cors }
      })
    }

    // Cold start or explicit refresh — do a full refresh inline.
    // This DOES make the first visitor wait, but it's a one-off.
    await refreshRegistryCache(env)
    const fresh = await env.KV.get(cacheKey)
    if (fresh) {
      return new Response(fresh, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=600', ...cors }
      })
    }

    // refreshRegistryCache failed — emit an empty shell so the page can still
    // render the build-time registry.json side of the merge.
    return new Response(JSON.stringify({ error: 'registry unavailable', fetchedAt: new Date().toISOString() }), {
      status: 503,
      headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...cors }
    })
  } catch (e) {
    const stale = await env.KV.get(cacheKey)
    if (stale) {
      return new Response(stale, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=60', ...cors }
      })
    }
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500,
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}

// ─── Registry item commit metadata ───
// Returns { sha, date, message } for the last commit that touched a given
// registry path. Lets detail pages show "Updated 3d ago" without each visitor
// hitting api.github.com directly.
async function handleRegistryCommit(env, cors, rawPath) {
  const allowedTop = /^(hands|channels|providers|workflows|agents|plugins|skills|mcp)\//
  if (!rawPath || !allowedTop.test(rawPath) || rawPath.includes('..')) {
    return new Response(JSON.stringify({ error: 'invalid path' }), {
      status: 400, headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
  const cacheKey = `registry_commit:${rawPath}`
  const cacheTimeKey = `${cacheKey}:time`
  const fresh = 1000 * 60 * 60 * 6 // 6h — commit metadata doesn't move fast

  const [cached, cacheTimeRaw] = await Promise.all([
    env.KV.get(cacheKey),
    env.KV.get(cacheTimeKey),
  ])
  const cacheTime = parseInt(cacheTimeRaw || '0', 10)
  if (cached && (Date.now() - cacheTime < fresh)) {
    return new Response(cached, {
      headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=3600', ...cors }
    })
  }

  const ghHeaders = { 'Accept': 'application/vnd.github.v3+json', 'User-Agent': 'LibrefangStats/1.0' }
  if (env.GITHUB_TOKEN) ghHeaders['Authorization'] = `token ${env.GITHUB_TOKEN}`
  try {
    const apiUrl = `https://api.github.com/repos/librefang/librefang-registry/commits?path=${encodeURIComponent(rawPath)}&per_page=1`
    const upstream = await fetch(apiUrl, { headers: ghHeaders })
    if (!upstream.ok) {
      if (cached) {
        return new Response(cached, {
          headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=60', ...cors }
        })
      }
      return new Response(JSON.stringify({ error: `upstream ${upstream.status}` }), {
        status: upstream.status, headers: { 'Content-Type': 'application/json', ...cors }
      })
    }
    const commits = await upstream.json()
    const first = Array.isArray(commits) && commits.length > 0 ? commits[0] : null
    const result = first ? {
      sha: first.sha,
      date: first.commit?.author?.date || first.commit?.committer?.date || null,
      message: (first.commit?.message || '').split('\n')[0].slice(0, 200),
    } : { sha: null, date: null, message: null }
    const json = JSON.stringify(result)
    await Promise.all([
      env.KV.put(cacheKey, json, { expirationTtl: 60 * 60 * 24 * 7 }),
      env.KV.put(cacheTimeKey, String(Date.now()), { expirationTtl: 60 * 60 * 24 * 7 }),
    ])
    return new Response(json, {
      headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=3600', ...cors }
    })
  } catch (e) {
    if (cached) {
      return new Response(cached, {
        headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=60', ...cors }
      })
    }
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500, headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}

// ─── Registry click tracking ───
// Fire-and-forget POST increments a per-(category,id) counter in a single
// JSON blob per category. We keep one blob per category (max 9) instead of
// one KV key per item because KV list ops are expensive, and 60–300 items per
// category * 9 categories is small enough to keep in one JSON.
const CATEGORIES = ['hands', 'channels', 'providers', 'workflows', 'agents', 'plugins', 'skills', 'mcp']
const ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/i

// Click counts are persisted in N shard blobs per category
// (`registry_clicks:<cat>:<shard>`). Each POST picks a random shard so
// concurrent writers only collide when they land on the same shard —
// collision probability drops from ~1 (single blob) to ~1/N, which is
// enough to make dropped-click telemetry rare without introducing a
// Durable Object binding. Reads union all shards.
//
// KV still has no CAS, so a true zero-loss counter would need a
// Durable Object or a queued aggregator; that's deferred until we
// outgrow the shard-plus-retry model here.
const CLICK_SHARDS = 8

function shardClickKey(category, shard) {
  return `registry_clicks:${category}:${shard}`
}

async function handleRegistryClick(env, cors, request, ctx) {
  let body
  try { body = await request.json() }
  catch { return new Response('invalid json', { status: 400, headers: cors }) }
  const { category, id } = body || {}
  if (!CATEGORIES.includes(category) || !ID_RE.test(id)) {
    return new Response('invalid payload', { status: 400, headers: cors })
  }
  const shard = Math.floor(Math.random() * CLICK_SHARDS)
  const key = shardClickKey(category, shard)
  // Use waitUntil so we don't block the response on the KV write.
  const doUpdate = async () => {
    let counts = {}
    try {
      const raw = await env.KV.get(key)
      if (raw) counts = JSON.parse(raw)
    } catch (_) { counts = {} }
    counts[id] = (counts[id] || 0) + 1
    // Cap each shard so one category's id space can't balloon any
    // single blob beyond KV's value-size comfort zone.
    const entries = Object.entries(counts)
    if (entries.length > 500) {
      entries.sort((a, b) => b[1] - a[1])
      counts = Object.fromEntries(entries.slice(0, 500))
    }
    await env.KV.put(key, JSON.stringify(counts))
  }
  if (ctx && typeof ctx.waitUntil === 'function') ctx.waitUntil(doUpdate())
  else await doUpdate()
  return new Response('{"ok":true}', {
    headers: { 'Content-Type': 'application/json', ...cors }
  })
}

// Union all per-category shards into a single {id: total} map.
async function loadClickTotalsForCategory(env, category) {
  const shardReads = await Promise.all(
    Array.from({ length: CLICK_SHARDS }, (_, i) =>
      env.KV.get(shardClickKey(category, i)).catch(() => null),
    ),
  )
  const totals = {}
  for (const raw of shardReads) {
    if (!raw) continue
    let counts = {}
    try { counts = JSON.parse(raw) } catch (_) { continue }
    for (const [id, n] of Object.entries(counts)) {
      totals[id] = (totals[id] || 0) + (typeof n === 'number' ? n : 0)
    }
  }
  return totals
}

async function handleRegistryTrending(env, cors, category) {
  if (!CATEGORIES.includes(category)) {
    return new Response(JSON.stringify({ error: 'invalid category' }), {
      status: 400, headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
  const counts = await loadClickTotalsForCategory(env, category)
  const top = Object.entries(counts)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 10)
    .map(([id, clicks]) => ({ id, clicks }))
  return new Response(JSON.stringify({ category, top }), {
    headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=600', ...cors }
  })
}

// ─── Registry metrics summary ───
// Aggregates across all categories: total clicks per category + top 5 items
// overall. Used by the /metrics page on the website.
async function handleRegistryMetrics(env, cors) {
  const perCategory = {}
  const allItems = []
  for (const cat of CATEGORIES) {
    const counts = await loadClickTotalsForCategory(env, cat)
    let total = 0
    let items = 0
    for (const [id, n] of Object.entries(counts)) {
      total += n
      items++
      allItems.push({ category: cat, id, clicks: n })
    }
    perCategory[cat] = { total, items }
  }
  allItems.sort((a, b) => b.clicks - a.clicks)
  const result = {
    generatedAt: new Date().toISOString(),
    perCategory,
    topOverall: allItems.slice(0, 10),
    totalClicks: allItems.reduce((s, x) => s + x.clicks, 0),
  }
  return new Response(JSON.stringify(result), {
    headers: { 'Content-Type': 'application/json', 'Cache-Control': 'public, max-age=300', ...cors }
  })
}

// ─── UI error reports ───
// The web app's ErrorBoundary POSTs here when a React subtree throws so we
// can see what's breaking without instrumenting a full error-tracking SaaS.
// We store the most recent 100 reports as a single JSON blob (cheap KV ops).
// Errors are persisted across N shard blobs (`ui_errors:<shard>`). Each
// POST picks a random shard so concurrent reports only collide when they
// land on the same one — collision probability drops from ~1 on the old
// single-blob layout to ~1/N. True zero-loss queuing would need a
// Durable Object or upstream aggregator; the shard model is a pragmatic
// middle ground that keeps low-traffic error telemetry usable.
const ERRORS_KEY_LEGACY = 'ui_errors' // still read so historical data isn't lost
const ERROR_SHARDS = 4
const ERRORS_MAX_TOTAL = 100
const ERRORS_MAX_PER_SHARD = Math.ceil(ERRORS_MAX_TOTAL / ERROR_SHARDS)

function shardErrorKey(shard) {
  return `ui_errors:${shard}`
}

async function loadAllErrors(env) {
  const shardReads = await Promise.all([
    ...Array.from({ length: ERROR_SHARDS }, (_, i) =>
      env.KV.get(shardErrorKey(i)).catch(() => null),
    ),
    env.KV.get(ERRORS_KEY_LEGACY).catch(() => null),
  ])
  const merged = []
  for (const raw of shardReads) {
    if (!raw) continue
    try {
      const arr = JSON.parse(raw)
      if (Array.isArray(arr)) merged.push(...arr)
    } catch (_) { /* skip unparseable shard */ }
  }
  // Entries are `{at: ISO, ...}`. Sort newest first, then truncate to the
  // total cap so the response shape stays stable for the dashboard.
  merged.sort((a, b) => String(b?.at || '').localeCompare(String(a?.at || '')))
  if (merged.length > ERRORS_MAX_TOTAL) merged.length = ERRORS_MAX_TOTAL
  return merged
}

async function handleErrorReport(env, cors, request, ctx) {
  let body
  try { body = await request.json() }
  catch { return new Response('invalid json', { status: 400, headers: cors }) }
  const { message, stack, pathname, lang, ua } = body || {}
  if (typeof message !== 'string' || message.length === 0 || message.length > 2000) {
    return new Response('invalid payload', { status: 400, headers: cors })
  }
  const entry = {
    at: new Date().toISOString(),
    message: String(message).slice(0, 2000),
    stack: typeof stack === 'string' ? String(stack).slice(0, 4000) : undefined,
    pathname: typeof pathname === 'string' ? String(pathname).slice(0, 256) : undefined,
    lang: typeof lang === 'string' ? String(lang).slice(0, 16) : undefined,
    ua: typeof ua === 'string' ? String(ua).slice(0, 256) : undefined,
  }
  const shard = Math.floor(Math.random() * ERROR_SHARDS)
  const shardKey = shardErrorKey(shard)
  const doUpdate = async () => {
    let errors = []
    try {
      const raw = await env.KV.get(shardKey)
      if (raw) errors = JSON.parse(raw)
    } catch (_) { errors = [] }
    errors.unshift(entry)
    if (errors.length > ERRORS_MAX_PER_SHARD) errors.length = ERRORS_MAX_PER_SHARD
    await env.KV.put(shardKey, JSON.stringify(errors))
  }
  if (ctx && typeof ctx.waitUntil === 'function') ctx.waitUntil(doUpdate())
  else await doUpdate()
  return new Response('{"ok":true}', {
    headers: { 'Content-Type': 'application/json', ...cors }
  })
}

async function handleErrorList(env, cors) {
  const errors = await loadAllErrors(env)
  return new Response(JSON.stringify({ errors }), {
    headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...cors }
  })
}

// ─── Scheduled: full registry refresh with TOML details ───
const REGISTRY_RAW = 'https://raw.githubusercontent.com/librefang/librefang-registry/main'

async function refreshRegistryCache(env) {
  const ghHeaders = {
    'Accept': 'application/vnd.github.v3+json',
    'User-Agent': 'LibrefangStats/1.0',
  }
  if (env.GITHUB_TOKEN) {
    ghHeaders['Authorization'] = `token ${env.GITHUB_TOKEN}`
  }

  async function fetchDir(path) {
    const res = await fetch(`${REGISTRY_API}/${path}`, { headers: ghHeaders })
    if (!res.ok) return []
    const items = await res.json()
    return items.filter(f => (f.type === 'dir' || f.name.endsWith('.toml')) && f.name !== 'README.md')
  }

  async function fetchToml(path) {
    const res = await fetch(`${REGISTRY_RAW}/${path}`)
    if (!res.ok) return null
    const text = await res.text()
    const get = (key) => {
      const m = text.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, 'm'))
      return m ? m[1] : ''
    }
    // Parse i18n sections: [i18n.zh], [i18n.ja], etc.
    const i18n = {}
    const i18nRegex = /\[i18n\.([a-zA-Z-]+)\]\s*\n(?:([^[]*?)(?=\n\[|\n*$))/g
    let match
    while ((match = i18nRegex.exec(text)) !== null) {
      const lang = match[1]
      const block = match[2] || ''
      const descMatch = block.match(/description\s*=\s*"([^"]*)"/)
      if (descMatch) {
        i18n[lang] = { description: descMatch[1] }
      }
    }
    const tagsMatch = text.match(/^tags\s*=\s*\[([^\]]*)\]/m)
    const tags = tagsMatch ? tagsMatch[1].match(/"([^"]*)"/g)?.map(s => s.replace(/"/g, '')) : undefined
    const result = { id: get('id'), name: get('name'), description: get('description'), category: get('category'), icon: get('icon') }
    if (tags && tags.length > 0) result.tags = tags
    if (Object.keys(i18n).length > 0) result.i18n = i18n
    return result
  }

  // Skills ship as SKILL.md with YAML frontmatter, not TOML, so `fetchToml`
  // returns empty fields for every skill. Mirrors `fetchSkillMd` in
  // web/scripts/fetch-registry.ts so live and build-time shapes match.
  async function fetchSkillMd(path, fallbackId) {
    const res = await fetch(`${REGISTRY_RAW}/${path}`)
    if (!res.ok) return null
    const text = await res.text()
    const fm = text.match(/^---\s*\n([\s\S]*?)\n---/)
    if (!fm) return null
    const block = fm[1]
    const get = (key) => {
      const m = block.match(new RegExp(`^${key}\\s*:\\s*"?([^"\\n]*?)"?\\s*$`, 'm'))
      return m ? m[1].trim() : ''
    }
    return {
      id: get('id') || fallbackId,
      name: get('name') || fallbackId,
      description: get('description'),
      category: 'skills',
      icon: '',
    }
  }

  try {
    const [handDirs, channelFiles, providerFiles, workflowFiles, agentDirs, pluginFiles, skillDirs, mcpFiles] = await Promise.all([
      fetchDir('hands'),
      fetchDir('channels'),
      fetchDir('providers'),
      fetchDir('workflows'),
      fetchDir('agents'),
      fetchDir('plugins'),
      fetchDir('skills'),
      fetchDir('mcp'),
    ])

    const filter = (items) => items.filter(f => f.name !== 'README.md')
    const hands = filter(handDirs)
    const channels = filter(channelFiles)
    const providers = filter(providerFiles)
    const workflows = filter(workflowFiles)
    const agents = filter(agentDirs)
    const plugins = filter(pluginFiles)
    const skills = filter(skillDirs)
    const mcp = filter(mcpFiles)

    // Cache-invalidation signature: every listed item's name AND git sha.
    // Comparing only counts missed content-only edits (description/tag/i18n
    // updates, one item swapped for another with the same total). With
    // sha-based hashing, any upstream registry change bumps the signature
    // so the manifest fetch actually runs.
    const sigOf = (items) =>
      items.map(i => `${i.name}@${i.sha || ''}`).sort().join(',')
    const signature = [
      `hands=${sigOf(hands)}`,
      `channels=${sigOf(channels)}`,
      `providers=${sigOf(providers)}`,
      `workflows=${sigOf(workflows)}`,
      `agents=${sigOf(agents)}`,
      `plugins=${sigOf(plugins)}`,
      `skills=${sigOf(skills)}`,
      `mcp=${sigOf(mcp)}`,
    ].join('|')

    const cached = await env.KV.get('registry_data')
    if (cached) {
      try {
        const old = JSON.parse(cached)
        if (old.signature === signature) {
          console.log('Registry unchanged (signature match), skipping manifest fetch')
          await env.KV.put('registry_data_time', String(Date.now()))
          return
        }
      } catch (_) { /* parse error, refetch */ }
    }

    // Registry changed — fetch full manifest details in batches of 10.
    // `fetcher` is configurable so skills (YAML frontmatter) route through
    // fetchSkillMd while everything else stays on fetchToml.
    async function fetchBatch(items, pathFn, fetcher = (p, _id) => fetchToml(p)) {
      const results = []
      for (let i = 0; i < items.length; i += 10) {
        const batch = items.slice(i, i + 10)
        const batchResults = await Promise.all(
          batch.map(item => fetcher(pathFn(item), item.name)),
        )
        results.push(...batchResults)
      }
      return results.filter(Boolean)
    }

    // Directory-based: manifest lives inside <dir>/<UPPER>.toml
    // File-based: item name already ends in .toml
    const [handDetails, agentDetails, skillDetails, channelDetails, providerDetails, workflowDetails, pluginDetails, mcpDetails] = await Promise.all([
      fetchBatch(hands, h => `hands/${h.name}/HAND.toml`),
      // `agent.toml` is lowercase, skills ship SKILL.md (YAML frontmatter),
      // plugins are directory-backed — match what fetch-registry.ts uses
      // so the per-item fetch actually resolves and populates descriptions.
      fetchBatch(agents, a => `agents/${a.name}/agent.toml`),
      fetchBatch(skills, s => `skills/${s.name}/SKILL.md`, fetchSkillMd),
      fetchBatch(channels, c => `channels/${c.name}`),
      fetchBatch(providers, p => `providers/${p.name}`),
      fetchBatch(workflows, w => `workflows/${w.name}`),
      fetchBatch(plugins, p => `plugins/${p.name}/plugin.toml`),
      fetchBatch(mcp, m => m.name.endsWith('.toml') ? `mcp/${m.name}` : `mcp/${m.name}/MCP.toml`),
    ])

    const result = {
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
      // Persisted so the next refresh can compare against it and skip
      // the manifest fetch when nothing has changed upstream.
      signature,
    }

    const json = JSON.stringify(result)
    await Promise.all([
      env.KV.put('registry_data', json),
      env.KV.put('registry_data_time', String(Date.now())),
    ])
    console.log('Registry refreshed:',
      hands.length, 'hands,',
      channels.length, 'channels,',
      agents.length, 'agents,',
      providers.length, 'providers,',
      workflows.length, 'workflows,',
      plugins.length, 'plugins,',
      skills.length, 'skills,',
      mcp.length, 'mcp')
  } catch (e) {
    console.error('Registry refresh failed:', e.message)
  }
}
