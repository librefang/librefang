const MAX_CACHE_ENTRIES = 50;
const CACHE_TTL_MS = 30 * 60 * 1000;

const sessionCache = new Map<string, { messages: unknown[]; expiresAt: number }>();

export const chatSessionCacheKey = (agentId: string, sessionId: string | null): string =>
  `${agentId}:${sessionId ?? ""}`;

const EVICT_TARGET = Math.floor(MAX_CACHE_ENTRIES * 0.8);

function evictCacheIfNeeded() {
  if (sessionCache.size < MAX_CACHE_ENTRIES) return;
  const now = Date.now();
  for (const [key, value] of sessionCache) {
    if (value.expiresAt <= now) sessionCache.delete(key);
  }
  if (sessionCache.size >= MAX_CACHE_ENTRIES) {
    // Evict oldest entries (insertion-order) down to 80 % capacity
    // to avoid thrashing on every insert under continuous churn.
    const excess = sessionCache.size - EVICT_TARGET;
    let removed = 0;
    for (const key of sessionCache.keys()) {
      if (removed >= excess) break;
      sessionCache.delete(key);
      removed++;
    }
  }
}

export function getCachedChatMessages<T>(key: string): T[] | undefined {
  const entry = sessionCache.get(key);
  if (!entry) return undefined;
  if (entry.expiresAt <= Date.now()) {
    sessionCache.delete(key);
    return undefined;
  }
  return entry.messages as T[];
}

export function setCachedChatMessages<T>(key: string, messages: T[]) {
  evictCacheIfNeeded();
  sessionCache.set(key, { messages, expiresAt: Date.now() + CACHE_TTL_MS });
}

export function deleteCachedChatMessages(key: string) {
  sessionCache.delete(key);
}

export function clearChatSessionCacheForAgent(agentId: string) {
  const prefix = `${agentId}:`;
  for (const key of sessionCache.keys()) {
    if (key.startsWith(prefix)) sessionCache.delete(key);
  }
}
