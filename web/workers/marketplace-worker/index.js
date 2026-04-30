// FangHub Marketplace Worker
// Storage: Cloudflare D1 (SQLite)
// Auth: GitHub OAuth (stateless JWT in cookie)

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
}

const JSON_HEADERS = { ...CORS, 'Content-Type': 'application/json' }

export default {
  async fetch(request, env, ctx) {
    if (request.method === 'OPTIONS') {
      return new Response(null, { headers: CORS })
    }

    const url = new URL(request.url)
    const path = url.pathname

    try {
      // Auth routes
      if (path === '/auth/github')          return handleGithubLogin(request, env)
      if (path === '/auth/github/callback') return handleGithubCallback(request, env)
      if (path === '/auth/me')              return handleMe(request, env)
      if (path === '/auth/logout')          return handleLogout()

      // Package routes
      if (path === '/v1/packages' && request.method === 'GET')
        return handleListPackages(request, env)
      if (path === '/v1/packages' && request.method === 'POST')
        return handleCreatePackage(request, env)

      const pkgMatch = path.match(/^\/v1\/packages\/([^/]+)$/)
      if (pkgMatch) {
        const slug = pkgMatch[1]
        if (request.method === 'GET')    return handleGetPackage(slug, env)
        if (request.method === 'PUT')    return handleUpdatePackage(slug, request, env)
        if (request.method === 'DELETE') return handleDeletePackage(slug, request, env)
      }

      const versionsMatch = path.match(/^\/v1\/packages\/([^/]+)\/versions$/)
      if (versionsMatch) {
        const slug = versionsMatch[1]
        if (request.method === 'GET')  return handleListVersions(slug, env)
        if (request.method === 'POST') return handlePublishVersion(slug, request, env)
      }

      const downloadMatch = path.match(/^\/v1\/download\/([^/]+)\/([^/]+)$/)
      if (downloadMatch) {
        return handleDownload(downloadMatch[1], downloadMatch[2], env, ctx)
      }

      const starMatch = path.match(/^\/v1\/packages\/([^/]+)\/star$/)
      if (starMatch) {
        return handleStar(starMatch[1], request, env)
      }

      return json({ error: 'Not Found' }, 404)
    } catch (err) {
      console.error(err)
      return json({ error: 'Internal Server Error' }, 500)
    }
  },

  // Weekly cron: flush pending download counts into totals
  async scheduled(_event, env) {
    await flushDownloadCounts(env)
  },
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

function handleGithubLogin(_request, env) {
  const params = new URLSearchParams({
    client_id: env.GITHUB_CLIENT_ID,
    scope: 'read:user',
    redirect_uri: env.GITHUB_REDIRECT_URI,
  })
  return Response.redirect(`https://github.com/login/oauth/authorize?${params}`, 302)
}

async function handleGithubCallback(request, env) {
  const url = new URL(request.url)
  const code = url.searchParams.get('code')
  if (!code) return json({ error: 'Missing code' }, 400)

  // Exchange code for token
  const tokenRes = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    body: JSON.stringify({
      client_id: env.GITHUB_CLIENT_ID,
      client_secret: env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: env.GITHUB_REDIRECT_URI,
    }),
  })
  const tokenData = await tokenRes.json()
  if (!tokenData.access_token) return json({ error: 'OAuth failed' }, 400)

  // Fetch GitHub user
  const userRes = await fetch('https://api.github.com/user', {
    headers: {
      Authorization: `Bearer ${tokenData.access_token}`,
      'User-Agent': 'librefang-marketplace',
    },
  })
  const ghUser = await userRes.json()

  const userId = `github:${ghUser.id}`
  const now = Math.floor(Date.now() / 1000)

  await env.DB.prepare(`
    INSERT INTO users (id, github_id, handle, display_name, avatar_url, created_at)
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(id) DO UPDATE SET
      handle = excluded.handle,
      display_name = excluded.display_name,
      avatar_url = excluded.avatar_url
  `).bind(userId, ghUser.id, ghUser.login, ghUser.name || ghUser.login, ghUser.avatar_url, now).run()

  // Sign a simple JWT (HS256)
  const jwt = await signJwt({ sub: userId, handle: ghUser.login }, env.JWT_SECRET)

  const redirectTo = env.AUTH_SUCCESS_REDIRECT || 'https://librefang.ai/marketplace'
  return new Response(null, {
    status: 302,
    headers: {
      Location: redirectTo,
      'Set-Cookie': `mp_token=${jwt}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000`,
    },
  })
}

async function handleMe(request, env) {
  const user = await authenticate(request, env)
  if (!user) return json({ error: 'Unauthorized' }, 401)
  return json(user)
}

function handleLogout() {
  return new Response(null, {
    status: 302,
    headers: {
      Location: 'https://librefang.ai/marketplace',
      'Set-Cookie': 'mp_token=; Path=/; HttpOnly; Secure; Max-Age=0',
    },
  })
}

// ---------------------------------------------------------------------------
// Packages
// ---------------------------------------------------------------------------

async function handleListPackages(request, env) {
  const url = new URL(request.url)
  const kind = url.searchParams.get('kind')
  const sort = url.searchParams.get('sort') || 'downloads'
  const q = url.searchParams.get('q') || ''
  const limit = Math.min(parseInt(url.searchParams.get('limit') || '20'), 100)
  const offset = parseInt(url.searchParams.get('offset') || '0')

  const sortCol = sort === 'updated' ? 'updated_at' : sort === 'stars' ? 'stars' : 'total_downloads'

  let where = '1=1'
  const binds = []

  if (kind) {
    where += ' AND kind = ?'
    binds.push(kind)
  }
  if (q) {
    where += ' AND (name LIKE ? OR description LIKE ?)'
    binds.push(`%${q}%`, `%${q}%`)
  }

  binds.push(limit, offset)

  const rows = await env.DB.prepare(
    `SELECT p.*, u.handle as author_handle, u.avatar_url as author_avatar
     FROM packages p
     JOIN users u ON u.id = p.author_id
     WHERE ${where}
     ORDER BY ${sortCol} DESC
     LIMIT ? OFFSET ?`
  ).bind(...binds).all()

  const countRow = await env.DB.prepare(
    `SELECT COUNT(*) as total FROM packages WHERE ${where.replace(/LIMIT.*/, '')}`
  ).bind(...binds.slice(0, -2)).first()

  return json({ packages: rows.results.map(formatPackage), total: countRow.total })
}

async function handleGetPackage(slug, env) {
  const row = await env.DB.prepare(
    `SELECT p.*, u.handle as author_handle, u.avatar_url as author_avatar
     FROM packages p
     JOIN users u ON u.id = p.author_id
     WHERE p.id = ?`
  ).bind(slug).first()

  if (!row) return json({ error: 'Not Found' }, 404)
  return json(formatPackage(row))
}

async function handleCreatePackage(request, env) {
  const user = await authenticate(request, env)
  if (!user) return json({ error: 'Unauthorized' }, 401)

  const body = await request.json()
  const { slug, name, kind, description, github_repo, homepage, tags } = body

  if (!slug || !name || !kind) return json({ error: 'slug, name, kind required' }, 400)
  if (!['skill', 'hand', 'extension', 'mcp'].includes(kind))
    return json({ error: 'Invalid kind' }, 400)
  if (!/^[a-z0-9-]+$/.test(slug)) return json({ error: 'slug must be lowercase alphanumeric with hyphens' }, 400)

  const now = Math.floor(Date.now() / 1000)

  try {
    await env.DB.prepare(
      `INSERT INTO packages (id, name, kind, description, author_id, github_repo, homepage, tags, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    ).bind(slug, name, kind, description || '', user.sub, github_repo || null, homepage || null, JSON.stringify(tags || []), now, now).run()
  } catch (e) {
    if (e.message?.includes('UNIQUE')) return json({ error: 'Slug already taken' }, 409)
    throw e
  }

  return json({ slug }, 201)
}

async function handleUpdatePackage(slug, request, env) {
  const user = await authenticate(request, env)
  if (!user) return json({ error: 'Unauthorized' }, 401)

  const pkg = await env.DB.prepare('SELECT author_id FROM packages WHERE id = ?').bind(slug).first()
  if (!pkg) return json({ error: 'Not Found' }, 404)
  if (pkg.author_id !== user.sub) return json({ error: 'Forbidden' }, 403)

  const body = await request.json()
  const now = Math.floor(Date.now() / 1000)

  await env.DB.prepare(
    `UPDATE packages SET
       name = COALESCE(?, name),
       description = COALESCE(?, description),
       github_repo = COALESCE(?, github_repo),
       homepage = COALESCE(?, homepage),
       tags = COALESCE(?, tags),
       updated_at = ?
     WHERE id = ?`
  ).bind(
    body.name || null,
    body.description || null,
    body.github_repo || null,
    body.homepage || null,
    body.tags ? JSON.stringify(body.tags) : null,
    now, slug
  ).run()

  return json({ ok: true })
}

async function handleDeletePackage(slug, request, env) {
  const user = await authenticate(request, env)
  if (!user) return json({ error: 'Unauthorized' }, 401)

  const pkg = await env.DB.prepare('SELECT author_id FROM packages WHERE id = ?').bind(slug).first()
  if (!pkg) return json({ error: 'Not Found' }, 404)
  if (pkg.author_id !== user.sub) return json({ error: 'Forbidden' }, 403)

  await env.DB.prepare('DELETE FROM package_versions WHERE package_id = ?').bind(slug).run()
  await env.DB.prepare('DELETE FROM stars WHERE package_id = ?').bind(slug).run()
  await env.DB.prepare('DELETE FROM packages WHERE id = ?').bind(slug).run()

  return json({ ok: true })
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

async function handleListVersions(slug, env) {
  const exists = await env.DB.prepare('SELECT id FROM packages WHERE id = ?').bind(slug).first()
  if (!exists) return json({ error: 'Not Found' }, 404)

  const rows = await env.DB.prepare(
    'SELECT * FROM package_versions WHERE package_id = ? ORDER BY created_at DESC'
  ).bind(slug).all()

  return json({ versions: rows.results })
}

async function handlePublishVersion(slug, request, env) {
  const user = await authenticate(request, env)
  if (!user) return json({ error: 'Unauthorized' }, 401)

  const pkg = await env.DB.prepare('SELECT author_id FROM packages WHERE id = ?').bind(slug).first()
  if (!pkg) return json({ error: 'Not Found' }, 404)
  if (pkg.author_id !== user.sub) return json({ error: 'Forbidden' }, 403)

  const body = await request.json()
  const { version, bundle_url, bundle_sha256, changelog } = body
  if (!version || !bundle_url || !bundle_sha256) return json({ error: 'version, bundle_url, bundle_sha256 required' }, 400)

  const versionId = `${slug}@${version}`
  const now = Math.floor(Date.now() / 1000)

  try {
    await env.DB.prepare(
      `INSERT INTO package_versions (id, package_id, version, changelog, bundle_url, bundle_sha256, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)`
    ).bind(versionId, slug, version, changelog || '', bundle_url, bundle_sha256, now).run()
  } catch (e) {
    if (e.message?.includes('UNIQUE')) return json({ error: 'Version already exists' }, 409)
    throw e
  }

  await env.DB.prepare(
    'UPDATE packages SET latest_version = ?, updated_at = ? WHERE id = ?'
  ).bind(version, now, slug).run()

  return json({ id: versionId }, 201)
}

// ---------------------------------------------------------------------------
// Download (redirect + async count)
// ---------------------------------------------------------------------------

async function handleDownload(slug, version, env, ctx) {
  const versionId = version === 'latest' ? null : `${slug}@${version}`

  let row
  if (versionId) {
    row = await env.DB.prepare('SELECT * FROM package_versions WHERE id = ?').bind(versionId).first()
  } else {
    const pkg = await env.DB.prepare('SELECT latest_version FROM packages WHERE id = ?').bind(slug).first()
    if (!pkg?.latest_version) return json({ error: 'Not Found' }, 404)
    row = await env.DB.prepare('SELECT * FROM package_versions WHERE id = ?').bind(`${slug}@${pkg.latest_version}`).first()
  }

  if (!row) return json({ error: 'Not Found' }, 404)

  // Async count increment (upsert into pending table, flushed weekly by cron)
  const week = getIsoWeek()
  ctx.waitUntil(
    env.DB.prepare(
      `INSERT INTO download_counts_pending (package_id, version_id, count, week)
       VALUES (?, ?, 1, ?)
       ON CONFLICT(package_id, version_id, week) DO UPDATE SET count = count + 1`
    ).bind(slug, row.id, week).run()
  )

  return Response.redirect(row.bundle_url, 302)
}

// ---------------------------------------------------------------------------
// Stars
// ---------------------------------------------------------------------------

async function handleStar(slug, request, env) {
  const user = await authenticate(request, env)
  if (!user) return json({ error: 'Unauthorized' }, 401)

  const pkg = await env.DB.prepare('SELECT id FROM packages WHERE id = ?').bind(slug).first()
  if (!pkg) return json({ error: 'Not Found' }, 404)

  const now = Math.floor(Date.now() / 1000)

  if (request.method === 'POST') {
    try {
      await env.DB.prepare('INSERT INTO stars (user_id, package_id, created_at) VALUES (?, ?, ?)').bind(user.sub, slug, now).run()
      await env.DB.prepare('UPDATE packages SET stars = stars + 1 WHERE id = ?').bind(slug).run()
    } catch (e) {
      if (!e.message?.includes('UNIQUE')) throw e
    }
    return json({ ok: true })
  }

  if (request.method === 'DELETE') {
    const res = await env.DB.prepare('DELETE FROM stars WHERE user_id = ? AND package_id = ?').bind(user.sub, slug).run()
    if (res.meta.changes > 0) {
      await env.DB.prepare('UPDATE packages SET stars = MAX(0, stars - 1) WHERE id = ?').bind(slug).run()
    }
    return json({ ok: true })
  }

  return json({ error: 'Method Not Allowed' }, 405)
}

// ---------------------------------------------------------------------------
// Cron: flush download counts
// ---------------------------------------------------------------------------

async function flushDownloadCounts(env) {
  const rows = await env.DB.prepare('SELECT * FROM download_counts_pending').all()
  if (!rows.results.length) return

  // Reset weekly_downloads first so the per-row increments below reflect only
  // this week's pending counts (not a cumulative total that gets overwritten).
  await env.DB.prepare('UPDATE packages SET weekly_downloads = 0').run()

  for (const row of rows.results) {
    await env.DB.prepare(
      'UPDATE packages SET total_downloads = total_downloads + ?, weekly_downloads = weekly_downloads + ? WHERE id = ?'
    ).bind(row.count, row.count, row.package_id).run()

    await env.DB.prepare(
      'UPDATE package_versions SET downloads = downloads + ? WHERE id = ?'
    ).bind(row.count, row.version_id).run()
  }

  await env.DB.prepare('DELETE FROM download_counts_pending').run()
}

// ---------------------------------------------------------------------------
// JWT (HS256, Web Crypto)
// ---------------------------------------------------------------------------

async function signJwt(payload, secret) {
  const header = b64url(JSON.stringify({ alg: 'HS256', typ: 'JWT' }))
  const body = b64url(JSON.stringify({ ...payload, iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 2592000 }))
  const data = `${header}.${body}`

  const key = await crypto.subtle.importKey(
    'raw', new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']
  )
  const sig = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(data))
  return `${data}.${b64url(new Uint8Array(sig))}`
}

async function verifyJwt(token, secret) {
  const parts = token.split('.')
  if (parts.length !== 3) return null

  const data = `${parts[0]}.${parts[1]}`
  const key = await crypto.subtle.importKey(
    'raw', new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' }, false, ['verify']
  )

  const sigBytes = Uint8Array.from(atob(parts[2].replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0))
  const valid = await crypto.subtle.verify('HMAC', key, sigBytes, new TextEncoder().encode(data))
  if (!valid) return null

  const payload = JSON.parse(atob(parts[1].replace(/-/g, '+').replace(/_/g, '/')))
  if (payload.exp < Math.floor(Date.now() / 1000)) return null
  return payload
}

function b64url(data) {
  const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '')
}

async function authenticate(request, env) {
  const cookie = request.headers.get('Cookie') || ''
  const match = cookie.match(/mp_token=([^;]+)/)
  if (!match) return null
  return verifyJwt(match[1], env.JWT_SECRET)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function json(data, status = 200) {
  return new Response(JSON.stringify(data), { status, headers: JSON_HEADERS })
}

function formatPackage(row) {
  return {
    ...row,
    tags: JSON.parse(row.tags || '[]'),
    is_verified: !!row.is_verified,
    is_featured: !!row.is_featured,
  }
}

function getIsoWeek() {
  const now = new Date()
  const start = new Date(now.getFullYear(), 0, 1)
  const week = Math.ceil(((now - start) / 86400000 + start.getDay() + 1) / 7)
  return `${now.getFullYear()}-W${String(week).padStart(2, '0')}`
}
