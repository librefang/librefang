// GitHub Stats Worker

export default {
  async fetch(request, env) {
    return handleFetch(request, env)
  },

  async scheduled(event, env, ctx) {
    ctx.waitUntil(recordDailyStars(env))
  },
}

async function recordDailyStars(env) {
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

      // Get PR count from header
      const prLink = pullsRes.headers.get('link')
      let prCount = 0
      if (prLink) {
        const match = prLink.match(/page=(\d+)>.*rel="last"/)
        if (match) prCount = parseInt(match[1], 10)
      }

      // Record all metrics
      await env.KV.put('stars_' + today, String(data.stargazers_count || 0))
      await env.KV.put('forks_' + today, String(data.forks_count || 0))
      await env.KV.put('issues_' + today, String(data.open_issues_count || 0))
      await env.KV.put('prs_' + today, String(prCount))
      await env.KV.put('downloads_' + today, '0') // Downloads recorded separately from releases

      console.log('Recorded:', today, 'stars:', data.stargazers_count, 'forks:', data.forks_count, 'issues:', data.open_issues_count, 'PRs:', prCount)
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

  // GET /api/github
  if (path === '/api/github' && request.method === 'GET') {
    return handleGitHubStats(env, cors)
  }

  return new Response('Not Found', { status: 404 })
}

async function handleGitHubStats(env, cors) {
  const cacheKey = 'github_stats'
  const cacheTimeKey = 'github_stats_time'
  const cacheDuration = 1000 * 60 * 30 // 30 minutes

  console.log('GITHUB_TOKEN exists:', !!env.GITHUB_TOKEN)

  try {
    // Check cache
    let cached, cacheTime
    try {
      cached = await env.KV.get(cacheKey)
      cacheTime = parseInt(await env.KV.get(cacheTimeKey) || '0', 10)
    } catch (e) {
      console.log('KV get error:', e.message)
    }

    if (cached && cacheTime && (Date.now() - cacheTime < cacheDuration)) {
      console.log('Returning cached response')
      return new Response(cached, {
        headers: {
          'Content-Type': 'application/json',
          'Cache-Control': 'no-cache',
          ...cors
        }
      })
    } else {
      console.log('Cache miss or expired, fetching from GitHub')
    }

    // Fetch from GitHub
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

    // Get PR count from header
    const prLink = pullsRes.headers.get('link')
    let prCount = 0
    if (prLink) {
      const match = prLink.match(/page=(\d+)>.*rel="last"/)
      if (match) prCount = parseInt(match[1], 10)
    }

    const downloads = releases.reduce((sum, rel) => {
      return sum + (rel.assets?.reduce((s, a) => s + (a.download_count || 0), 0) || 0)
    }, 0)

    // Record today's all metrics
    const today = new Date().toISOString().split('T')[0]
    await env.KV.put('stars_' + today, String(repo.stargazers_count || 0))
    await env.KV.put('forks_' + today, String(repo.forks_count || 0))
    await env.KV.put('issues_' + today, String(repo.open_issues_count || 0))
    await env.KV.put('prs_' + today, String(prCount))

    // Get last 30 days history
    const history = []
    for (let i = 0; i < 30; i++) {
      const d = new Date()
      d.setDate(d.getDate() - i)
      const dateStr = d.toISOString().split('T')[0]
      const stars = await env.KV.get('stars_' + dateStr)
      const forks = await env.KV.get('forks_' + dateStr)
      const issues = await env.KV.get('issues_' + dateStr)
      const prs = await env.KV.get('prs_' + dateStr)
      if (stars) {
        history.push({
          date: dateStr,
          stars: parseInt(stars, 10),
          forks: forks ? parseInt(forks, 10) : 0,
          issues: issues ? parseInt(issues, 10) : 0,
          prs: prs ? parseInt(prs, 10) : 0
        })
      }
    }

    const result = {
      stars: repo.stargazers_count || 0,
      forks: repo.forks_count || 0,
      issues: repo.open_issues_count || 0,
      prs: prCount,
      lastUpdate: repo.updated_at || '',
      createdAt: repo.created_at || '',
      downloads,
      starHistory: history.reverse(),
    }

    const json = JSON.stringify(result)

    // Cache
    try {
      await env.KV.put(cacheKey, json)
      await env.KV.put(cacheTimeKey, String(Date.now()))
      console.log('Cached response, KV write successful')
    } catch (e) {
      console.log('KV put error:', e.message, e.stack)
    }

    return new Response(json, {
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  } catch (e) {
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500,
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}
