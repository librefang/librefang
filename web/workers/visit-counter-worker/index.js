// Visit Counter Worker
// Storage: D1 (visit_counts table)

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
}

export default {
  async fetch(request, env) {
    if (request.method === 'OPTIONS') return new Response(null, { headers: CORS })

    const url = new URL(request.url)
    const path = url.pathname

    if (path === '/api/track' && request.method === 'POST')
      return handleTrack(request, env)

    if ((path === '/' || path === '/api') && request.method === 'GET')
      return handleVisits(env)

    if (path === '/script.js' && request.method === 'GET')
      return handleScript()

    return new Response('Not Found', { status: 404 })
  },
}

async function handleTrack(request, env) {
  await request.text()

  const today = isoDate()
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO visit_counts (date, count) VALUES (?, 1)
       ON CONFLICT(date) DO UPDATE SET count = count + 1`,
    ).bind(today),
    env.DB.prepare(
      `INSERT INTO visit_counts (date, count) VALUES ('__total__', 1)
       ON CONFLICT(date) DO UPDATE SET count = count + 1`,
    ),
  ])

  const row = await env.DB.prepare(
    `SELECT count FROM visit_counts WHERE date = '__total__'`,
  ).first()

  return json({ success: true, total: row?.count ?? 0 })
}

async function handleVisits(env) {
  const today = isoDate()
  const [totalRow, todayRow] = await Promise.all([
    env.DB.prepare(`SELECT count FROM visit_counts WHERE date = '__total__'`).first(),
    env.DB.prepare(`SELECT count FROM visit_counts WHERE date = ?`).bind(today).first(),
  ])

  return json({ total: totalRow?.count ?? 0, today: todayRow?.count ?? 0, date: today })
}

function handleScript() {
  const script = `(function() {
  fetch('https://counter.librefang.ai/api/track', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ page: window.location.pathname || 'home' }),
    keepalive: true
  }).catch(function() {});
})();`
  return new Response(script, {
    headers: { 'Content-Type': 'application/javascript', ...CORS },
  })
}

function isoDate() {
  return new Date().toISOString().split('T')[0]
}

function json(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS },
  })
}
