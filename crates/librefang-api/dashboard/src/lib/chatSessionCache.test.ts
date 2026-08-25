import { afterEach, describe, expect, it } from "vitest";

import {
  CACHE_TTL_MS,
  MAX_CACHE_ENTRIES,
  chatSessionCacheKey,
  clearChatSessionCacheForAgent,
  getCachedChatMessages,
  setCachedChatMessages,
} from "./chatSessionCache";

const AGENT_ID = "cache-contract-agent";

function key(index: number): string {
  return chatSessionCacheKey(AGENT_ID, `session-${index}`);
}

afterEach(() => clearChatSessionCacheForAgent(AGENT_ID));

describe("chat session cache", () => {
  it("exports its capacity and TTL defaults", () => {
    expect(MAX_CACHE_ENTRIES).toBe(50);
    expect(CACHE_TTL_MS).toBe(30 * 60 * 1000);
  });

  it("updates a full cache without evicting and refreshes LRU order", () => {
    for (let index = 0; index < MAX_CACHE_ENTRIES; index++) {
      setCachedChatMessages(key(index), [`value-${index}`]);
    }

    setCachedChatMessages(key(0), ["hot"]);

    expect(getCachedChatMessages(key(0))).toEqual(["hot"]);
    expect(getCachedChatMessages(key(1))).toEqual(["value-1"]);

    setCachedChatMessages(key(MAX_CACHE_ENTRIES), ["new"]);

    expect(getCachedChatMessages(key(0))).toEqual(["hot"]);
    expect(getCachedChatMessages(key(1))).toBeUndefined();
    expect(getCachedChatMessages(key(10))).toBeUndefined();
    expect(getCachedChatMessages(key(11))).toEqual(["value-11"]);
    expect(getCachedChatMessages(key(MAX_CACHE_ENTRIES))).toEqual(["new"]);
  });
});
