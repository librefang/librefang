const MAX_CACHE_ENTRIES = 50;
const CACHE_TTL_MS = 30 * 60 * 1000;

const sessionCache = new Map<string, { messages: unknown[]; expiresAt: number }>();

export const chatSessionCacheKey = (agentId: string, sessionId: string | null): string =>
  `${agentId}:${sessionId ?? ""}`;

function evictCacheIfNeeded() {
  if (sessionCache.size < MAX_CACHE_ENTRIES) return;
  const now = Date.now();
  for (const [key, value] of sessionCache) {
    if (value.expiresAt <= now) sessionCache.delete(key);
  }
  if (sessionCache.size >= MAX_CACHE_ENTRIES) {
    const firstKey = sessionCache.keys().next().value;
    if (firstKey !== undefined) sessionCache.delete(firstKey);
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
