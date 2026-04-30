// GitHub Stats Worker
// Tracks GitHub repo metrics (stars, forks, issues, PRs, releases).
// Storage: D1 (github_stats_history + kv_store). Cache API for HTTP responses.

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

  const today = isoDate()
  const entry = {
    stars: repo.stargazers_count || 0,
    forks: repo.forks_count || 0,
    issues: repo.open_issues_count || 0,
    prs: prCount,
  }

  await upsertTodayStats(env, today, entry)
  const history = await getLast30Days(env)

  const body = JSON.stringify({
    ...entry,
    lastUpdate: repo.updated_at || '',
    createdAt: repo.created_at || '',
    downloads,
    starHistory: history,
  })

  const response = jsonResponse(body, 1800)
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
  await upsertTodayStats(env, isoDate(), {
    stars: repo.stargazers_count || 0,
    forks: repo.forks_count || 0,
    issues: repo.open_issues_count || 0,
    prs: parseLinkHeaderCount(pullsRes.headers.get('link')),
  })
  console.log('Recorded daily stats:', isoDate(), 'stars:', repo.stargazers_count)
}

// ---------------------------------------------------------------------------
// D1 helpers
// ---------------------------------------------------------------------------

async function upsertTodayStats(env, date, { stars, forks, issues, prs }) {
  await env.DB.prepare(
    `INSERT INTO github_stats_history (date, stars, forks, issues, prs)
     VALUES (?, ?, ?, ?, ?)
     ON CONFLICT(date) DO UPDATE SET
       stars = excluded.stars, forks = excluded.forks,
       issues = excluded.issues, prs = excluded.prs`,
  ).bind(date, stars, forks, issues, prs).run()
}

async function getLast30Days(env) {
  const rows = await env.DB.prepare(
    `SELECT date, stars, forks, issues, prs
     FROM github_stats_history ORDER BY date DESC LIMIT 30`,
  ).all()
  return rows.results.reverse()
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

function isoDate() {
  return new Date().toISOString().split('T')[0]
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
