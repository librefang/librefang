// Cloudflare Pages Advanced Mode worker
// Handles SPA fallback routing + security headers
// Note: _redirects and _headers are ignored when _worker.js is present

const SECURITY_HEADERS = {
  'X-Content-Type-Options': 'nosniff',
  'X-Frame-Options': 'DENY',
  'X-XSS-Protection': '1; mode=block',
  'Referrer-Policy': 'strict-origin-when-cross-origin',
  'Permissions-Policy': 'camera=(), microphone=(), geolocation=()',
  'Content-Security-Policy': "default-src 'self'; script-src 'self' 'unsafe-inline' https://www.googletagmanager.com https://static.cloudflareinsights.com https://librefang-counter.suzukaze-haduki.workers.dev https://counter.librefang.ai; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data: https:; connect-src 'self' https://api.github.com https://fonts.googleapis.com https://fonts.gstatic.com https://www.google-analytics.com https://librefang-counter.suzukaze-haduki.workers.dev https://counter.librefang.ai https://stats.librefang.ai https://marketplace.librefang.ai; frame-src 'none'",
};

const IMMUTABLE_CACHE = 'public, max-age=31536000, immutable';

// Plugin registry public key (raw 32-byte Ed25519, base64). Served at
// /.well-known/registry-pubkey for the LibreFang daemon's TOFU resolver
// (see crates/librefang-runtime/src/plugin_manager.rs::resolve_registry_pubkey
// and docs/architecture/plugin-signing.md).
//
// Mirror of REGISTRY_PUBLIC_KEY in web/workers/{registry,marketplace}-worker/
// wrangler.toml. Rotation: regenerate keypair via web/workers/keygen.mjs,
// update both wrangler.toml files, the daemon embedded active key, and this
// constant in lockstep; scripts/check-pubkey-lockstep.sh enforces the set.
const REGISTRY_PUBLIC_KEY = 'joY8IYrUbbACfKRyp2CTcEbcEty8wcBwP1MTxU+vjaM=';

function addHeaders(response, url, cacheAssets = true) {
  const headers = new Headers(response.headers);

  // Security headers for all responses
  for (const [key, value] of Object.entries(SECURITY_HEADERS)) {
    headers.set(key, value);
  }

  // Cache headers for hashed static assets
  const path = url.pathname;
  if (cacheAssets && path.startsWith('/assets/')) {
    headers.set('Cache-Control', IMMUTABLE_CACHE);
  }

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

const LOCALES = ['zh-TW', 'zh', 'ja', 'ko', 'de', 'es', 'pl', 'uk'];
const TRAILING_SLASH_ROOTS = new Set([
  ...LOCALES.map((locale) => '/' + locale),
  '/deploy',
  '/changelog',
  '/privacy',
]);

// Pretty installer URL → actual asset, chosen by client User-Agent so that
// `curl -fsSL https://librefang.ai/install | sh` and the PowerShell one-liner
// both work without the file extension. Browsers without a CLI-shaped UA fall
// through to the SPA so a human pasting /install into the address bar still
// sees a page instead of a shell script.
const CLI_INSTALLER_UA = /(curl|wget|fetch|libfetch|httpie)/i;
const POWERSHELL_INSTALLER_UA = /(powershell|pwsh)/i;

function installerAssetFor(pathname, userAgent) {
  if (pathname !== '/install') return null;
  if (POWERSHELL_INSTALLER_UA.test(userAgent)) return '/install.ps1';
  if (CLI_INSTALLER_UA.test(userAgent)) return '/install.sh';
  return null;
}

// Canonicalize URLs: published roots get a trailing slash ( /zh → /zh/ ),
// while sub-paths stay un-slashed ( /zh/skills/ → /zh/skills ). Returns the
// canonical pathname, or null if the request is already canonical.
function canonicalPath(pathname) {
  if (pathname === '/') return null;

  if (TRAILING_SLASH_ROOTS.has(pathname)) return pathname + '/';

  // Normalize repeated root slashes, or strip them from sub-paths.
  if (pathname.length > 1 && pathname.endsWith('/')) {
    const withoutTrailingSlash = pathname.replace(/\/+$/, '');
    if (TRAILING_SLASH_ROOTS.has(withoutTrailingSlash)) {
      return pathname === withoutTrailingSlash + '/' ? null : withoutTrailingSlash + '/';
    }
    return withoutTrailingSlash;
  }
  return null;
}

function internalErrorResponse(url) {
  return addHeaders(
    new Response('Internal Server Error', {
      status: 500,
      headers: {
        'Cache-Control': 'no-store',
        'Content-Type': 'text/plain; charset=utf-8',
      },
    }),
    url,
    false,
  );
}

async function handleRequest(request, env) {
  const url = new URL(request.url);

  // Serve the registry pubkey BEFORE asset/SPA fallback — otherwise the SPA
  // catch-all hands daemons an HTML page that fails base64 validation.
  if (url.pathname === '/.well-known/registry-pubkey') {
    return new Response(REGISTRY_PUBLIC_KEY + '\n', {
      headers: {
        'Content-Type': 'text/plain; charset=utf-8',
        'Cache-Control': 'public, max-age=86400',
        ...SECURITY_HEADERS,
      },
    });
  }

  // 301 redirect to the canonical URL before serving. Preserves the query.
  const canonical = canonicalPath(url.pathname);
  if (canonical !== null) {
    const target = canonical + url.search;
    return Response.redirect(new URL(target, url).toString(), 301);
  }

  // Rewrite /install → /install.sh or /install.ps1 for CLI clients so the
  // suffix-less install one-liners work. Must happen before the asset fetch
  // (which would 404 on /install) and before the SPA fallback (which would
  // hand the CLI an HTML page, causing `sh: newline unexpected`).
  const installerAsset = installerAssetFor(
    url.pathname,
    request.headers.get('user-agent') || '',
  );
  if (installerAsset) {
    const installerUrl = new URL(installerAsset, request.url);
    const installerResponse = await env.ASSETS.fetch(new Request(installerUrl, request));
    return addHeaders(installerResponse, url);
  }

  // Try serving static asset first
  const assetResponse = await env.ASSETS.fetch(request);

  // Static asset found — return with headers
  if (assetResponse.status !== 404) {
    return addHeaders(assetResponse, url);
  }

  // SPA fallback — serve index.html for navigation requests.
  const indexResponse = await env.ASSETS.fetch(new URL('/', request.url));
  if (!indexResponse.ok) {
    console.error('SPA fallback index failed with status', indexResponse.status);
    return internalErrorResponse(url);
  }
  return addHeaders(indexResponse, url, false);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    try {
      return await handleRequest(request, env);
    } catch (error) {
      console.error('Cloudflare Pages asset request failed', error);
      return internalErrorResponse(url);
    }
  },
};
