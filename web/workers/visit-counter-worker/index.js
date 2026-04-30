// Visit Counter Worker
// Storage: D1 (visit_counts table). KV read on first boot for migration.

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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async function handleTrack(request, env) {
  await request.text()
  await ensureMigrated(env)

  const today = isoDate()

  await env.DB.prepare(
    `INSERT INTO visit_counts (date, count) VALUES (?, 1)
     ON CONFLICT(date) DO UPDATE SET count = count + 1`,
  ).bind(today).run()

  await env.DB.prepare(
    `INSERT INTO visit_counts (date, count) VALUES ('__total__', 1)
     ON CONFLICT(date) DO UPDATE SET count = count + 1`,
  ).bind().run()

  const row = await env.DB.prepare(
    `SELECT count FROM visit_counts WHERE date = '__total__'`,
  ).first()

  return json({ success: true, total: row?.count ?? 0 })
}

async function handleVisits(env) {
  await ensureMigrated(env)

  const today = isoDate()
  const [totalRow, todayRow] = await Promise.all([
    env.DB.prepare(`SELECT count FROM visit_counts WHERE date = '__total__'`).first(),
    env.DB.prepare(`SELECT count FROM visit_counts WHERE date = ?`).bind(today).first(),
  ])

  return json({ total: totalRow?.count ?? 0, today: todayRow?.count ?? 0, date: today })
}

function handleScript() {
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
    headers: { 'Content-Type': 'application/javascript', ...CORS },
  })
}

// ---------------------------------------------------------------------------
// One-time KV → D1 migration
// ---------------------------------------------------------------------------

async function ensureMigrated(env) {
  const done = await env.DB.prepare(
    `SELECT count FROM visit_counts WHERE date = '__migrated__'`,
  ).first()
  if (done) return

  // Read total from KV
  const kvTotal = parseInt(await env.VISIT_COUNTER.get('total') || '0', 10) || 0

  // Read today's count from KV
  const today = isoDate()
  const kvToday = parseInt(await env.VISIT_COUNTER.get('today_' + today) || '0', 10) || 0

  const stmts = [
    env.DB.prepare(
      `INSERT INTO visit_counts (date, count) VALUES ('__total__', ?)
       ON CONFLICT(date) DO UPDATE SET count = excluded.count`,
    ).bind(kvTotal),
    env.DB.prepare(
      `INSERT INTO visit_counts (date, count) VALUES (?, ?)
       ON CONFLICT(date) DO UPDATE SET count = excluded.count`,
    ).bind(today, kvToday),
    env.DB.prepare(
      `INSERT INTO visit_counts (date, count) VALUES ('__migrated__', 1)
       ON CONFLICT(date) DO NOTHING`,
    ),
  ]

  await env.DB.batch(stmts)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isoDate() {
  return new Date().toISOString().split('T')[0]
}

function json(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS },
  })
}
