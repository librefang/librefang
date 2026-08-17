// Service worker: offline-fallback for a small set of fixed resources only.
// IMPORTANT: we deliberately do NOT cache /assets/*.js / /assets/*.css.
// Vite emits those with content-hashed filenames, so the browser HTTP cache
// already handles them perfectly. Caching them in the SW on top of that led
// to a "two React copies" situation after a deploy: stale cached vendor-react
// chunk alongside fresh RegistryPage chunk, triggering "Cannot read
// properties of null (reading 'useCallback')". Don't do that.
const CACHE_NAME = 'librefang-v3'
const ROOT_HTML = '/'
const STATIC_ASSETS = [
  ROOT_HTML,
  '/logo.png',
  '/favicon.svg',
  '/og-image.svg',
  '/manifest.webmanifest',
]

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(STATIC_ASSETS))
      .then(() => self.skipWaiting())
  )
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((k) => k !== CACHE_NAME).map((k) => caches.delete(k)))
      )
      .then(() => self.clients.claim())
  )
})

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url)
  if (event.request.method !== 'GET' || url.origin !== self.location.origin) return

  // Hashed assets: don't touch — browser cache is enough and the SW cache can
  // desync chunks across deploys.
  if (url.pathname.startsWith('/assets/')) return

  // Registry data: always network, since the SW can't know when a new item
  // lands and we don't want stale category counts.
  if (url.pathname === '/registry.json' || url.pathname === '/feed.xml') return

  // Network-first for HTML so deploys propagate immediately; fall back to
  // the latest cached SPA shell when offline. All navigation paths receive
  // the same index.html from the Pages worker, so keep one bounded root entry
  // instead of growing the cache with arbitrary paths and query strings.
  if (event.request.headers.get('accept')?.includes('text/html')) {
    const networkResponse = fetch(event.request)
    event.waitUntil(
      networkResponse
        .then((response) => {
          if (!response.ok) return
          return caches.open(CACHE_NAME).then((cache) =>
            cache.put(ROOT_HTML, response.clone())
          )
        })
        .catch(() => undefined)
    )
    event.respondWith(
      networkResponse
        .catch(() =>
          caches.match(event.request).then((cached) => cached || caches.match(ROOT_HTML))
        )
    )
    return
  }

  // Cache-first only for the tiny allowlisted set above.
  if (STATIC_ASSETS.includes(url.pathname)) {
    event.respondWith(
      caches.match(event.request).then((cached) => {
        if (cached) return cached
        return fetch(event.request).then((response) => {
          if (response.ok) {
            event.waitUntil(
              caches
                .open(CACHE_NAME)
                .then((cache) => cache.put(event.request, response.clone()))
                .catch(() => undefined)
            )
          }
          return response
        })
      })
    )
    return
  }

  // Everything else: plain network, no SW involvement.
})
