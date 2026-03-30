// Cloudflare Pages Advanced Mode worker
// Handles SPA fallback routing + security headers
// Note: _redirects and _headers are ignored when _worker.js is present

const SECURITY_HEADERS = {
  'X-Content-Type-Options': 'nosniff',
  'X-Frame-Options': 'DENY',
  'X-XSS-Protection': '1; mode=block',
  'Referrer-Policy': 'strict-origin-when-cross-origin',
  'Permissions-Policy': 'camera=(), microphone=(), geolocation=()',
  'Content-Security-Policy': "default-src 'self'; script-src 'self' 'unsafe-inline' https://www.googletagmanager.com https://static.cloudflareinsights.com https://librefang-counter.suzukaze-haduki.workers.dev https://counter.librefang.ai; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data: https:; connect-src 'self' https://api.github.com https://fonts.googleapis.com https://fonts.gstatic.com https://www.google-analytics.com https://librefang-counter.suzukaze-haduki.workers.dev https://counter.librefang.ai https://stats.librefang.ai; frame-src 'none'",
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

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

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
