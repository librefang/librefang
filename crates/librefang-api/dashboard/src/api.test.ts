import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildAuthenticatedWebSocketUrl,
  getMetricsText,
  listTools,
  patchAgentConfig,
  setApiKey,
  verifyStoredAuth,
} from "./api";

class LocalStorageMock {
  private store = new Map<string, string>();

  clear() {
    this.store.clear();
  }

  getItem(key: string) {
    return this.store.has(key) ? this.store.get(key)! : null;
  }

  removeItem(key: string) {
    this.store.delete(key);
  }

  setItem(key: string, value: string) {
    this.store.set(key, value);
  }
}

describe("dashboard auth helpers", () => {
  const fetchMock = vi.fn();
  const localStorageMock = new LocalStorageMock();

  beforeEach(() => {
    fetchMock.mockReset();
    localStorageMock.clear();

    Object.defineProperty(globalThis, "fetch", {
      configurable: true,
      value: fetchMock,
    });
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: localStorageMock,
    });
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: { language: "en-US" },
    });
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: {
        location: {
          protocol: "http:",
          host: "127.0.0.1:4545",
        },
      },
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("adds the stored token to websocket URLs", () => {
    setApiKey("secret-token");

    expect(
      buildAuthenticatedWebSocketUrl("/api/agents/abc/ws"),
    ).toBe("ws://127.0.0.1:4545/api/agents/abc/ws?token=secret-token");
  });

  it("clears stale stored auth when the protected probe returns 401", async () => {
    setApiKey("expired-token");
    fetchMock.mockResolvedValue(new Response("", { status: 401 }));

    await expect(verifyStoredAuth()).resolves.toBe(false);
    expect(localStorageMock.getItem("librefang-api-key")).toBeNull();
  });

  it("sends the bearer token on protected helper requests", async () => {
    setApiKey("secret-token");
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ tools: [] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    fetchMock.mockResolvedValueOnce(
      new Response("metric 1\n", {
        status: 200,
        headers: { "Content-Type": "text/plain" },
      }),
    );

    await expect(listTools()).resolves.toEqual([]);
    await expect(getMetricsText()).resolves.toBe("metric 1\n");

    const listToolsHeaders = fetchMock.mock.calls[0][1]?.headers as Headers;
    const metricsHeaders = fetchMock.mock.calls[1][1]?.headers as Headers;

    expect(listToolsHeaders.get("authorization")).toBe("Bearer secret-token");
    expect(metricsHeaders.get("authorization")).toBe("Bearer secret-token");
  });

  it("patchAgentConfig sends temperature in request body", async () => {
    setApiKey("secret-token");
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ status: "ok" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await patchAgentConfig("test-agent-id", {
      temperature: 1.5,
      max_tokens: 8192,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/agents/test-agent-id/config");
    expect(options.method).toBe("PATCH");
    const body = JSON.parse(options.body);
    expect(body.temperature).toBe(1.5);
    expect(body.max_tokens).toBe(8192);
  });
});
