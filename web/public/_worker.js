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

function addHeaders(response, url) {
  const headers = new Headers(response.headers);

  // Security headers for all responses
  for (const [key, value] of Object.entries(SECURITY_HEADERS)) {
    headers.set(key, value);
  }

  // Cache headers for hashed static assets
  const path = url.pathname;
  if (path.startsWith('/assets/')) {
    headers.set('Cache-Control', IMMUTABLE_CACHE);
  }

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

const LOCALES = ['zh-TW', 'zh', 'ja', 'ko', 'de', 'es'];

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

// Canonicalize URLs: locale roots get a trailing slash ( /zh → /zh/ ), while
// sub-paths stay un-slashed ( /zh/skills/ → /zh/skills ). Returns the
// canonical pathname, or null if the request is already canonical.
function canonicalPath(pathname) {
  if (pathname === '/') return null;
  // Locale root without trailing slash — add one.
  for (const loc of LOCALES) {
    if (pathname === '/' + loc) return '/' + loc + '/';
  }
  // Anything else with a trailing slash and more than one segment — strip it.
  // Keeps /zh/ and /deploy/ alone but redirects /zh/skills/ → /zh/skills.
  if (pathname.length > 1 && pathname.endsWith('/')) {
    const segs = pathname.split('/').filter(Boolean);
    const isLocaleRoot = segs.length === 1 && LOCALES.includes(segs[0]);
    const isBlessedTrailingSlashPath = /^\/(deploy|changelog|privacy)\/?$/.test(pathname);
    if (!isLocaleRoot && !isBlessedTrailingSlashPath) {
      return pathname.replace(/\/+$/, '');
    }
  }
  return null;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // 301 redirect to canonical URL before serving. Preserves query + hash.
    const canonical = canonicalPath(url.pathname);
    if (canonical !== null) {
      const target = canonical + url.search + url.hash;
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

    // SPA fallback — serve index.html for navigation requests
    const indexResponse = await env.ASSETS.fetch(new URL('/', request.url));
    const headers = new Headers(indexResponse.headers);
    for (const [key, value] of Object.entries(SECURITY_HEADERS)) {
      headers.set(key, value);
    }

    return new Response(indexResponse.body, {
      status: 200,
      headers,
    });
  },
};
