import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildAuthenticatedWebSocketUrl,
  getAgentTools,
  getMetricsText,
  listTools,
  patchAgentConfig,
  patchHandAgentRuntimeConfig,
  setApiKey,
  updateAgentTools,
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

  it("patchHandAgentRuntimeConfig trims tri-state string fields before sending", async () => {
    setApiKey("secret-token");
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ status: "ok" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await patchHandAgentRuntimeConfig("test-hand-agent-id", {
      model: "gpt-4o",
      api_key_env: "  OPENAI_KEY  ",
      base_url: "   ",
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/agents/test-hand-agent-id/hand-runtime-config");
    expect(options.method).toBe("PATCH");
    expect(JSON.parse(options.body)).toEqual({
      model: "gpt-4o",
      api_key_env: "OPENAI_KEY",
      base_url: "",
    });
  });

  it("getAgentTools requests the agent tools endpoint", async () => {
    setApiKey("secret-token");
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ tool_allowlist: ["bash"], tool_blocklist: ["rm"], disabled: false }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await expect(getAgentTools("agent-123")).resolves.toEqual({
      tool_allowlist: ["bash"],
      tool_blocklist: ["rm"],
      disabled: false,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/agents/agent-123/tools");
    const headers = options?.headers as Headers;
    expect(headers.get("authorization")).toBe("Bearer secret-token");
  });

  it("updateAgentTools sends both allowlist and blocklist", async () => {
    setApiKey("secret-token");
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ status: "ok" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await updateAgentTools("agent-123", {
      tool_allowlist: ["bash", "webfetch"],
      tool_blocklist: ["rm"],
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/agents/agent-123/tools");
    expect(options.method).toBe("PUT");
    expect(JSON.parse(options.body)).toEqual({
      tool_allowlist: ["bash", "webfetch"],
      tool_blocklist: ["rm"],
    });
  });

  it("listTools supports both wrapped and direct array responses", async () => {
    setApiKey("secret-token");
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ tools: [{ name: "bash" }] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify([{ name: "webfetch" }]), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await expect(listTools()).resolves.toEqual([{ name: "bash" }]);
    await expect(listTools()).resolves.toEqual([{ name: "webfetch" }]);
  });
});
