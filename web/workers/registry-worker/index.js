// Registry Worker
// Proxies librefang-registry (GitHub) with Cache API for HTTP responses.
// Storage: D1 (registry_clicks, kv_store, ui_errors). No KV dependency.
//
// Plugin signing (#3805 + PR #4600 hardening):
//
// The Ed25519 private key lives ONLY in the librefang-registry GitHub
// Actions secret, never in this worker. The CI workflow there
// (.github/workflows/refresh-cache.yml) builds plugins-index.json,
// signs it locally, and commits both `plugins-index.json` and
// `plugins-index.json.sig` to the repo root before poking the worker.
// This handler is now a pure transport: it fetches the committed
// bytes (3 raw subrequests — plugins json + sig + registry json) and
// stores them in D1 verbatim. No bytes get re-signed by anything
// holding registry-controlled key material.
//
// Daemon contract (unchanged):
//   GET /api/registry/index.json     — canonical bytes that were signed
//                                      (flat JSON array of plugin entries)
//   GET /api/registry/index.json.sig — base64 Ed25519 signature
//   GET /.well-known/registry-pubkey — base64 raw 32-byte Ed25519 pubkey
//
// The dashboard's /api/registry endpoint continues to return the
// dict-shaped payload (hands/channels/plugins/skills/...) for the
// marketplace UI. The two formats are stored separately in D1
// (registry_data vs plugins_index). See web/workers/SIGNING.md.
//
// Why moved out of the worker (PR review CRITICAL #1): the previous
// design treated the worker as a sign-anything oracle reachable via
// REGISTRY_REFRESH_TOKEN — anyone with that token could push arbitrary
// bytes to `main` and have them signed. The new flow ties signing to
// the registry repo's CI identity, so the trust root is the repo's
// branch protection + Actions secret rather than the worker.

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

    // Both paths return the same bytes. The custom domain (stats.librefang.ai)
    // routes only /api/* to the worker, so the daemon's HTTP-rotation
    // probe needs an /api/* alias when the embedded constant is absent
    // (self-hosted forks). The /.well-known/ form stays for direct
    // workers.dev clients.
    if ((path === '/.well-known/registry-pubkey' || path === '/api/registry/pubkey')
        && request.method === 'GET')
      return handlePubkey(env)

    // Operator-only path used by the librefang-registry GitHub Action to
    // rebuild the D1 cache + signed index right after a push to main, so
    // dashboard / daemon don't have to wait for the next 02:00 UTC cron
    // tick. Auth is a shared bearer token deployed as a worker secret;
    // until it is set, the route returns 503 and stays inert. See
    // `web/workers/SIGNING.md` § "Forced refresh from the registry repo".
    if (path === '/api/registry/refresh' && request.method === 'POST')
      return handleForcedRefresh(request, env)

    return new Response('Not Found', { status: 404 })
  },

  // Cron is now a backstop for the GH Action — the registry repo's CI
  // is the primary path, this just guards against CI outages by
  // re-pulling the same in-repo files daily. Auth-free because the
  // worker invokes itself.
  async scheduled(_event, env) {
    await doSyncFromRepo(env, { requireAuth: false })
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

  // The fresh-fetch path makes ~30+ GitHub subrequests, which exceeds the
  // Workers Free 50-subrequest-per-request budget. Cron runs (02:00 UTC)
  // get the higher quota, so we keep the staleness window honored even
  // when `?refresh=1` is set — operators wanting an immediate rebuild
  // should let the cron fire.
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
// Signed-index endpoints (Ed25519)
// ---------------------------------------------------------------------------


async function handleSignedIndex(env) {
  const row = await env.DB.prepare(
    `SELECT value FROM kv_store WHERE key = 'plugins_index'`,
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
    `SELECT value FROM kv_store WHERE key = 'plugins_index_sig'`,
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

// Forced refresh — invoked by the librefang-registry GitHub Action after a
// push that touched any registry content. Constant-time bearer-token auth
// against `REGISTRY_REFRESH_TOKEN`; 503 until set so the endpoint can't
// be probed.
//
// PURE TRANSPORT: this handler does NOT sign anything. It fetches three
// files from raw.githubusercontent.com (committed by the registry repo's
// CI, which holds the Ed25519 private key) and writes them to D1
// verbatim. The bytes the daemon eventually verifies are byte-identical
// to the bytes the CI signed. Any byte-mutation between sign and serve
// would be detectable.
async function handleForcedRefresh(request, env) {
  return doSyncFromRepo(env, { requireAuth: true, request })
}

async function doSyncFromRepo(env, { requireAuth, request }) {
  if (requireAuth) {
    const expected = (env.REGISTRY_REFRESH_TOKEN || '').trim()
    if (!expected) return errorResponse('refresh endpoint not configured', 503)
    const auth = request.headers.get('authorization') || ''
    const m = auth.match(/^Bearer\s+(.+)$/i)
    const provided = m ? m[1].trim() : ''
    if (!constantTimeEqual(provided, expected)) {
      return errorResponse('unauthorized', 401)
    }
  }

  const RAW_BASE =
    'https://raw.githubusercontent.com/librefang/librefang-registry/main'
  const pluginsUrl = `${RAW_BASE}/plugins-index.json`
  const pluginsSigUrl = `${RAW_BASE}/plugins-index.json.sig`
  const registryUrl = `${RAW_BASE}/registry-index.json`

  // Fetch all three in parallel — 3 subrequests, constant in registry size.
  const [pluginsResp, pluginsSigResp, registryResp] = await Promise.all([
    fetch(pluginsUrl, { cf: { cacheTtl: 0 } }),
    fetch(pluginsSigUrl, { cf: { cacheTtl: 0 } }),
    fetch(registryUrl, { cf: { cacheTtl: 0 } }),
  ])
  if (!pluginsResp.ok) {
    return errorResponse(
      `failed to fetch plugins-index.json: HTTP ${pluginsResp.status}`,
      502,
    )
  }
  if (!pluginsSigResp.ok) {
    return errorResponse(
      `failed to fetch plugins-index.json.sig: HTTP ${pluginsSigResp.status} ` +
      `— registry CI must commit the signature alongside the index`,
      502,
    )
  }
  if (!registryResp.ok) {
    return errorResponse(
      `failed to fetch registry-index.json: HTTP ${registryResp.status}`,
      502,
    )
  }
  const pluginsText = await pluginsResp.text()
  const pluginsSigText = (await pluginsSigResp.text()).trim()
  const registryText = await registryResp.text()

  // Validate shapes / sig length. We refuse to ingest unparseable bytes
  // or a sig that's the wrong length, since the daemon would just
  // hard-fail on those anyway and we'd rather catch it here.
  try {
    const parsed = JSON.parse(pluginsText)
    if (!Array.isArray(parsed)) throw new Error('plugins-index.json is not an array')
  } catch (e) {
    return errorResponse(`plugins-index.json invalid: ${e.message}`, 502)
  }
  try {
    const parsed = JSON.parse(registryText)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      throw new Error('registry-index.json is not an object')
  } catch (e) {
    return errorResponse(`registry-index.json invalid: ${e.message}`, 502)
  }
  // Ed25519 sig is exactly 64 bytes → 88 base64 chars (with `==` padding)
  // or 86 chars unpadded. Anything else is wrong-shape garbage that the
  // daemon would reject; better to bounce here than poison D1 with it
  // (PR re-review MEDIUM-NEW-F).
  if (
    (pluginsSigText.length !== 88 && pluginsSigText.length !== 86) ||
    !/^[A-Za-z0-9+/]+(?:==)?$/.test(pluginsSigText)
  ) {
    return errorResponse(
      `plugins-index.json.sig is not a valid Ed25519 signature ` +
      `(expected 86 or 88 base64 chars; got ${pluginsSigText.length})`,
      502,
    )
  }

  const now = Math.floor(Date.now() / 1000)
  await env.DB.batch([
    env.DB
      .prepare(
        `INSERT INTO kv_store (key, value, updated_at) VALUES ('plugins_index', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
      )
      .bind(pluginsText, now),
    env.DB
      .prepare(
        `INSERT INTO kv_store (key, value, updated_at) VALUES ('plugins_index_sig', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
      )
      .bind(pluginsSigText, now),
    env.DB
      .prepare(
        `INSERT INTO kv_store (key, value, updated_at) VALUES ('registry_data', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
      )
      .bind(registryText, now),
  ])

  // Purge the Cache-API entry the dashboard hits — without this,
  // /api/registry serves the previous cached payload for up to 1h. PR
  // re-review MEDIUM-NEW-G: surface the outcome so the GH Action can
  // see partial-success runs (D1 stored, cache stale) instead of
  // assuming fresh-everywhere on a 200.
  let cachePurged = false
  let cachePurgeError = null
  try {
    cachePurged = await caches.default.delete(new Request('https://internal/registry_data'))
  } catch (e) {
    cachePurgeError = e?.message || String(e)
    console.error('Cache purge failed:', cachePurgeError)
  }

  return new Response(
    JSON.stringify({
      ok: true,
      refreshed_at: now,
      plugins_bytes: pluginsText.length,
      sig_bytes: pluginsSigText.length,
      registry_bytes: registryText.length,
      cache_purged: cachePurged,
      ...(cachePurgeError ? { cache_purge_error: cachePurgeError } : {}),
    }),
    { headers: { 'Content-Type': 'application/json', ...CORS } },
  )
}

// Constant-time string comparison hardened against length leak via early
// return (PR review MEDIUM #11). Iterates up to max(a.length, b.length)
// and folds the length delta into the mismatch accumulator so timing
// reveals at most "they're not the same" — not "yours is shorter than
// mine". Multi-byte chars are handled via charCodeAt (no throw on
// surrogate halves; XOR semantics are still well-defined).
function constantTimeEqual(a, b) {
  const len = Math.max(a.length, b.length)
  let mismatch = a.length ^ b.length
  for (let i = 0; i < len; i++) {
    const ca = i < a.length ? a.charCodeAt(i) : 0
    const cb = i < b.length ? b.charCodeAt(i) : 0
    mismatch |= ca ^ cb
  }
  return mismatch === 0
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
