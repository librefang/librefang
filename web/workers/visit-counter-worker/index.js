// Visit Counter Worker

export default {
  async fetch(request, env) {
    const url = new URL(request.url)
    const path = url.pathname
    const method = request.method

    const cors = {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type',
    }

    if (method === 'OPTIONS') {
      return new Response(null, { headers: cors })
    }

    // POST /api/track - track page visit
    if (path === '/api/track' && method === 'POST') {
      return handleTrack(request, env, cors)
    }

    // GET /api - get visit stats
    if (path === '/' || path === '/api') {
      return handleVisits(env, cors)
    }

    // GET /script.js - tracking script
    if (path === '/script.js' && method === 'GET') {
      return handleScript(cors)
    }

    return new Response('Not Found', { status: 404 })
  },
}

async function handleTrack(request, env, cors) {
  try {
    await request.text()

    const today = new Date().toISOString().split('T')[0]

    let total = parseInt(await env.VISIT_COUNTER.get('total') || '0', 10) || 0
    let todayCount = parseInt(await env.VISIT_COUNTER.get('today_' + today) || '0', 10) || 0

    total++
    todayCount++

    await env.VISIT_COUNTER.put('total', String(total))
    await env.VISIT_COUNTER.put('today_' + today, String(todayCount))

    return new Response(JSON.stringify({ success: true, total }), {
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  } catch (e) {
    return new Response(JSON.stringify({ error: e.message }), {
      status: 500,
      headers: { 'Content-Type': 'application/json', ...cors }
    })
  }
}

async function handleVisits(env, cors) {
  const today = new Date().toISOString().split('T')[0]
  const total = parseInt(await env.VISIT_COUNTER.get('total') || '0', 10) || 0
  const todayCount = parseInt(await env.VISIT_COUNTER.get('today_' + today) || '0', 10) || 0

  return new Response(JSON.stringify({ total, today: todayCount, date: today }), {
    headers: { 'Content-Type': 'application/json', ...cors }
  })
}

function handleScript(cors) {
  const script = `(function() {
  var page = window.location.pathname || 'home';
  fetch('https://counter.librefang.ai/api/track', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ page: page }),
    keepalive: true
  }).catch(function() {});
})();`

  return new Response(script, {
    headers: { 'Content-Type': 'application/javascript', ...cors }
  })
}
