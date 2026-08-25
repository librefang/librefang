const CACHE_NAME = "librefang-v1";
const PRECACHE = ["/dashboard/"];
const MAX_CACHE_ENTRIES = 200;

async function trimCache(cache) {
  const keys = await cache.keys();
  const excess = keys.length - MAX_CACHE_ENTRIES;
  if (excess <= 0) return;
  await Promise.all(keys.slice(0, excess).map((request) => cache.delete(request)));
}

async function precache() {
  try {
    const cache = await caches.open(CACHE_NAME);
    await Promise.allSettled(
      PRECACHE.map(async (url) => {
        const response = await fetch(url, { cache: "reload" });
        if (response.ok) await cache.put(url, response);
      }),
    );
  } catch (error) {
    console.warn("Service worker precache failed", error);
  }
}

self.addEventListener("install", (e) => {
  e.waitUntil(precache());
});

// Let an explicit update prompt activate a waiting worker. Do not take over
// open tabs automatically while they may still reference the prior build.
self.addEventListener("message", (e) => {
  if (e.data?.type === "SKIP_WAITING") self.skipWaiting();
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then(async (names) => {
      await Promise.all(
        names
          .filter((n) => n !== CACHE_NAME)
          .map((n) => caches.delete(n)),
      );
      const cache = await caches.open(CACHE_NAME);
      await trimCache(cache);
    }),
  );
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);

  // Only handle http(s) requests
  if (!url.protocol.startsWith("http")) return;

  // API requests: network only
  if (url.pathname.startsWith("/api/")) return;

  // Only cache GET requests (Cache API does not support POST)
  if (e.request.method !== "GET") return;

  // Static assets: stale-while-revalidate
  e.respondWith(
    caches.open(CACHE_NAME).then(async (cache) => {
      const cached = await cache.match(e.request);
      const fetched = fetch(e.request)
        .then(async (resp) => {
          if (resp.ok) {
            await cache.put(e.request, resp.clone());
            await trimCache(cache);
          }
          return resp;
        });
      if (cached) {
        e.waitUntil(fetched.catch(() => undefined));
        return cached;
      }
      return fetched.catch(() => Response.error());
    }),
  );
});
