// GitHub Stats Worker
// Optimized: stores history as single JSON blob to minimize KV operations
// Before: ~128 KV ops per uncached request (120 reads + 8 writes)
// After: ~6 KV ops per uncached request (2 reads + 4 writes)

export default {
  async fetch(request, env) {
    return handleFetch(request, env)
  },

  async scheduled(event, env, ctx) {
    ctx.waitUntil(recordDailyStats(env))
  },
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
      } catch {}

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

function handleFetch(request, env) {
  const url = new URL(request.url)
  const path = url.pathname

  const cors = {
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Methods': 'GET, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type',
  }

  if (request.method === 'OPTIONS') {
    return new Response(null, { headers: cors })
  }

  if (path === '/api/github' && request.method === 'GET') {
    return handleGitHubStats(env, cors)
  }

  return new Response('Not Found', { status: 404 })
}

async function handleGitHubStats(env, cors) {
  const cacheKey = 'github_stats'
  const cacheTimeKey = 'github_stats_time'
  const cacheDuration = 1000 * 60 * 30 // 30 minutes

  try {
    // Check cache (2 KV reads)
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
    } catch {}

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
