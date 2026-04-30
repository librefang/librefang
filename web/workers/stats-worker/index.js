// GitHub Stats Worker
// Tracks GitHub repo metrics (stars, forks, issues, PRs, releases).
// HTTP Cache API handles response caching — KV is only used for the 90-day
// history blob and the one-time migration flag (both require persistence
// across cache evictions and cron writes).

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
}

const GH_HEADERS = {
  Accept: 'application/vnd.github.v3+json',
  'User-Agent': 'LibrefangStats/1.0',
}

export default {
  async fetch(request, env, ctx) {
    if (request.method === 'OPTIONS') return new Response(null, { headers: CORS })

    const url = new URL(request.url)

    if (url.pathname === '/api/github' && request.method === 'GET')
      return handleGitHubStats(request, env, ctx, url.searchParams.has('refresh'))

    if (url.pathname === '/api/releases' && request.method === 'GET')
      return handleReleases(request, env, ctx)

    return new Response('Not Found', { status: 404 })
  },

  async scheduled(_event, env) {
    await recordDailyStats(env)
  },
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

async function handleGitHubStats(request, env, ctx, forceRefresh) {
  const cacheKey = new Request('https://internal/github_stats', request)

  if (!forceRefresh) {
    const cached = await caches.default.match(cacheKey)
    if (cached) return addCors(cached)
  }

  const headers = ghHeaders(env)
  const [repoRes, releasesRes, pullsRes] = await Promise.all([
    fetch('https://api.github.com/repos/librefang/librefang', { headers }),
    fetch('https://api.github.com/repos/librefang/librefang/releases?per_page=10', { headers }),
    fetch('https://api.github.com/repos/librefang/librefang/pulls?state=open&per_page=1', { headers }),
  ])

  const repo = repoRes.ok ? await repoRes.json() : {}
  const releases = releasesRes.ok ? await releasesRes.json() : []
  const prCount = parseLinkHeaderCount(pullsRes.headers.get('link'))

  const downloads = releases.reduce(
    (sum, rel) => sum + (rel.assets?.reduce((s, a) => s + (a.download_count || 0), 0) ?? 0),
    0,
  )

  const history = await appendTodayToHistory(env, {
    stars: repo.stargazers_count || 0,
    forks: repo.forks_count || 0,
    issues: repo.open_issues_count || 0,
    prs: prCount,
  })

  const body = JSON.stringify({
    stars: repo.stargazers_count || 0,
    forks: repo.forks_count || 0,
    issues: repo.open_issues_count || 0,
    prs: prCount,
    lastUpdate: repo.updated_at || '',
    createdAt: repo.created_at || '',
    downloads,
    starHistory: history.slice(-30),
  })

  const response = jsonResponse(body, 1800) // 30 min
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

async function handleReleases(request, env, ctx) {
  const cacheKey = new Request('https://internal/releases', request)
  const cached = await caches.default.match(cacheKey)
  if (cached) return addCors(cached)

  const res = await fetch(
    'https://api.github.com/repos/librefang/librefang/releases?per_page=20',
    { headers: ghHeaders(env) },
  )

  if (!res.ok) return errorResponse(`GitHub API returned ${res.status}`, 502)

  const body = await res.text()
  const response = jsonResponse(body, 1800)
  ctx.waitUntil(caches.default.put(cacheKey, response.clone()))
  return response
}

// ---------------------------------------------------------------------------
// Cron
// ---------------------------------------------------------------------------

async function recordDailyStats(env) {
  const headers = ghHeaders(env)
  const [repoRes, pullsRes] = await Promise.all([
    fetch('https://api.github.com/repos/librefang/librefang', { headers }),
    fetch('https://api.github.com/repos/librefang/librefang/pulls?state=open&per_page=1', { headers }),
  ])

  if (!repoRes.ok) {
    console.error('recordDailyStats: GitHub API returned', repoRes.status)
    return
  }

  const repo = await repoRes.json()
  await appendTodayToHistory(env, {
    stars: repo.stargazers_count || 0,
    forks: repo.forks_count || 0,
    issues: repo.open_issues_count || 0,
    prs: parseLinkHeaderCount(pullsRes.headers.get('link')),
  })

  console.log('Recorded daily stats:', new Date().toISOString().split('T')[0], 'stars:', repo.stargazers_count)
}

// ---------------------------------------------------------------------------
// History (KV — must survive cache eviction and be written by cron)
// ---------------------------------------------------------------------------

async function appendTodayToHistory(env, todayStats) {
  const today = new Date().toISOString().split('T')[0]
  let history = []

  try {
    const raw = await env.KV.get('stats_history')
    if (raw) history = JSON.parse(raw)
  } catch (e) {
    console.error('KV read error:', e.message)
  }

  history = await migrateOldKeys(env, history)

  const entry = { date: today, ...todayStats }
  const idx = history.findIndex(h => h.date === today)
  if (idx >= 0) history[idx] = entry
  else history.push(entry)

  if (history.length > 90) history = history.slice(-90)
  await env.KV.put('stats_history', JSON.stringify(history))
  return history
}

// One-time migration from old per-day KV keys to single history blob.
async function migrateOldKeys(env, history) {
  if (history.length >= 7) return history
  if (await env.KV.get('stats_migration_done')) return history

  const existingDates = new Set(history.map(h => h.date))
  const newEntries = []

  for (let i = 0; i < 90; i++) {
    const d = new Date()
    d.setDate(d.getDate() - i)
    const dateStr = d.toISOString().split('T')[0]
    if (existingDates.has(dateStr)) continue

    const stars = await env.KV.get('stars_' + dateStr)
    if (!stars) continue

    const [forks, issues, prs] = await Promise.all([
      env.KV.get('forks_' + dateStr),
      env.KV.get('issues_' + dateStr),
      env.KV.get('prs_' + dateStr),
    ])
    newEntries.push({
      date: dateStr,
      stars: parseInt(stars, 10),
      forks: forks ? parseInt(forks, 10) : 0,
      issues: issues ? parseInt(issues, 10) : 0,
      prs: prs ? parseInt(prs, 10) : 0,
    })
  }

  if (newEntries.length > 0) {
    history = [...history, ...newEntries]
    history.sort((a, b) => a.date.localeCompare(b.date))
    const seen = new Map()
    for (const e of history) seen.set(e.date, e)
    history = Array.from(seen.values()).slice(-90)
    await env.KV.put('stats_history', JSON.stringify(history))
  }

  await env.KV.put('stats_migration_done', '1')
  return history
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function ghHeaders(env) {
  const h = { ...GH_HEADERS }
  if (env.GITHUB_TOKEN) h['Authorization'] = `token ${env.GITHUB_TOKEN}`
  return h
}

function parseLinkHeaderCount(link) {
  if (!link) return 0
  const m = link.match(/page=(\d+)>.*rel="last"/)
  return m ? parseInt(m[1], 10) : 0
}

function jsonResponse(body, maxAge = 300) {
  return new Response(body, {
    headers: {
      'Content-Type': 'application/json',
      'Cache-Control': `public, max-age=${maxAge}`,
      ...CORS,
    },
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
