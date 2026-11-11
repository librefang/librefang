import { afterEach, describe, expect, it, vi } from "vitest";
import {
  getContextEngineHealth,
  getContextEngineMetrics,
  getContextEngineSandboxPolicy,
  getContextEngineTraces,
} from "../../api";

afterEach(() => {
  vi.unstubAllGlobals();
});

function stubFetch(body: unknown, status: number): ReturnType<typeof vi.fn> {
  const fetchMock = vi
    .fn()
    .mockResolvedValue(
      new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { "Content-Type": "application/json" },
      }),
    );
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("context engine client", () => {
  it("maps a 204 empty body to null on every nullable route", async () => {
    const fetchMock = stubFetch(undefined, 204);

    await expect(getContextEngineHealth()).resolves.toBeNull();
    await expect(getContextEngineMetrics()).resolves.toBeNull();
    await expect(getContextEngineTraces()).resolves.toBeNull();
    await expect(getContextEngineSandboxPolicy()).resolves.toBeNull();

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      "/api/context-engine/health",
      "/api/context-engine/metrics",
      "/api/context-engine/traces",
      "/api/context-engine/sandbox-policy",
    ]);
  });

  it("parses a 503 health body as data instead of throwing", async () => {
    stubFetch(
      {
        healthy: false,
        active_plugins: ["memory-lens"],
        issues: [{ plugin: "memory-lens", errors: ["missing ingest hook"] }],
      },
      503,
    );

    await expect(getContextEngineHealth()).resolves.toEqual({
      healthy: false,
      active_plugins: ["memory-lens"],
      issues: [{ plugin: "memory-lens", errors: ["missing ingest hook"] }],
    });
  });

  it("still throws for a non-503 health failure", async () => {
    stubFetch({ error: "boom" }, 500);

    await expect(getContextEngineHealth()).rejects.toThrow();
  });
});
