// GitHub Stats Worker
// Optimized: stores history as single JSON blob to minimize KV operations
// Includes one-time migration from old individual KV keys (stars_YYYY-MM-DD)

export default {
  async fetch(request, env) {
    return handleFetch(request, env)
  },

  async scheduled(event, env, ctx) {
    ctx.waitUntil(recordDailyStats(env))
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
    const forceRefresh = url.searchParams.has('refresh')
    return handleGitHubStats(env, cors, forceRefresh)
  }

  return new Response('Not Found', { status: 404 })
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
