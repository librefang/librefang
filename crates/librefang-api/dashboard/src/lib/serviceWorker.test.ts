import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

type WorkerHandler = (event: Record<string, unknown>) => void;

class MemoryCache {
  entries = new Map<unknown, Response>();

  async match(request: unknown): Promise<Response | undefined> {
    return this.entries.get(request);
  }

  async put(request: unknown, response: Response): Promise<void> {
    this.entries.set(request, response);
  }

  async keys(): Promise<unknown[]> {
    return [...this.entries.keys()];
  }

  async delete(request: unknown): Promise<boolean> {
    return this.entries.delete(request);
  }
}

function loadWorker(fetchMock: typeof fetch, cache = new MemoryCache()) {
  const handlers = new Map<string, WorkerHandler>();
  const skipWaiting = vi.fn();
  const worker = {
    addEventListener: (type: string, handler: WorkerHandler) => handlers.set(type, handler),
    skipWaiting,
  };
  const caches = {
    open: vi.fn(async () => cache),
    keys: vi.fn(async () => ["librefang-v0", "librefang-v1"]),
    delete: vi.fn(async () => true),
  };
  const source = readFileSync(join(__dirname, "../../public/sw.js"), "utf8");
  runInNewContext(source, {
    self: worker,
    caches,
    fetch: fetchMock,
    Response,
    URL,
    console,
  });
  return { cache, caches, handlers, skipWaiting };
}

describe("dashboard service worker", () => {
  it("returns an error Response when both cache and network miss", async () => {
    const fetchMock = vi.fn(async () => { throw new Error("offline"); }) as unknown as typeof fetch;
    const { handlers } = loadWorker(fetchMock);
    let responsePromise: Promise<Response> | undefined;
    handlers.get("fetch")?.({
      request: { url: "https://example.test/dashboard/missing.js", method: "GET" },
      respondWith: (promise: Promise<Response>) => { responsePromise = promise; },
      waitUntil: vi.fn(),
    });
    const response = await responsePromise;
    expect(response).toBeInstanceOf(Response);
    expect(response?.type).toBe("error");
  });

  it("keeps a cached response while refreshing it in the event lifetime", async () => {
    const request = { url: "https://example.test/dashboard/app.js", method: "GET" };
    const cached = new Response("cached");
    const fresh = new Response("fresh");
    const cache = new MemoryCache();
    cache.entries.set(request, cached);
    const { handlers } = loadWorker(vi.fn(async () => fresh) as unknown as typeof fetch, cache);
    let responsePromise: Promise<Response> | undefined;
    let refreshPromise: Promise<unknown> | undefined;
    handlers.get("fetch")?.({
      request,
      respondWith: (promise: Promise<Response>) => { responsePromise = promise; },
      waitUntil: (promise: Promise<unknown>) => { refreshPromise = promise; },
    });
    expect(await responsePromise).toBe(cached);
    await refreshPromise;
    expect(await cache.entries.get(request)?.text()).toBe("fresh");
  });

  it("bounds the runtime cache after a successful network response", async () => {
    const cache = new MemoryCache();
    for (let i = 0; i < 205; i++) cache.entries.set(`old-${i}`, new Response(String(i)));
    const request = { url: "https://example.test/dashboard/new.js", method: "GET" };
    const { handlers } = loadWorker(vi.fn(async () => new Response("new")) as unknown as typeof fetch, cache);
    let responsePromise: Promise<Response> | undefined;
    handlers.get("fetch")?.({
      request,
      respondWith: (promise: Promise<Response>) => { responsePromise = promise; },
      waitUntil: vi.fn(),
    });
    await responsePromise;
    expect(cache.entries.size).toBe(200);
    expect(cache.entries.has(request)).toBe(true);
  });

  it("settles a failed precache and waits for explicit activation", async () => {
    const fetchMock = vi.fn(async () => { throw new Error("offline"); }) as unknown as typeof fetch;
    const { handlers, skipWaiting } = loadWorker(fetchMock);
    let installPromise: Promise<unknown> | undefined;
    handlers.get("install")?.({
      waitUntil: (promise: Promise<unknown>) => { installPromise = promise; },
    });
    await expect(installPromise).resolves.toBeUndefined();
    expect(skipWaiting).not.toHaveBeenCalled();
    handlers.get("message")?.({ data: { type: "SKIP_WAITING" } });
    expect(skipWaiting).toHaveBeenCalledOnce();
  });
});
