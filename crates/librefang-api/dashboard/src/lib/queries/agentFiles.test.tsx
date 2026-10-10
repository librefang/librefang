import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

import { agentFileQueries, useAgentFile } from "./agentFiles";
import * as httpClient from "../http/client";
import { ApiError } from "../http/errors";
import { agentFileKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  getAgentFile: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

const FILE = {
  name: "IDENTITY.md",
  content: "---\nvibe: helpful\n---\n# Identity\n",
  size_bytes: 31,
};

describe("useAgentFile", () => {
  it("stays idle when the agent id is empty", () => {
    const { result } = renderHook(() => useAgentFile("", "IDENTITY.md"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getAgentFile).not.toHaveBeenCalled();
  });

  it("stays idle when the filename is empty", () => {
    const { result } = renderHook(() => useAgentFile("agent-1", ""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getAgentFile).not.toHaveBeenCalled();
  });

  it("fetches the file and caches it under agentFileKeys.detail", async () => {
    vi.mocked(httpClient.getAgentFile).mockResolvedValue(FILE);
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useAgentFile("agent-1", "IDENTITY.md"), { wrapper });

    await waitFor(() => {
      expect(
        queryClient.getQueryData(agentFileKeys.detail("agent-1", "IDENTITY.md")),
      ).toEqual(FILE);
    });
    expect(httpClient.getAgentFile).toHaveBeenCalledWith("agent-1", "IDENTITY.md");
  });
});

describe("agentFileQueries.detail retry policy", () => {
  const options = agentFileQueries.detail("agent-1", "IDENTITY.md");
  const retry = options.retry as (failureCount: number, error: unknown) => boolean;

  it("does not retry a 404 — 'no such file yet' is a stable answer, not a failure", () => {
    expect(retry(1, new ApiError(404, "HTTP_404", "file not found"))).toBe(false);
  });

  it("keeps the client-wide one-retry budget for every other status", () => {
    expect(retry(1, new ApiError(500, "HTTP_500", "boom"))).toBe(true);
    expect(retry(2, new ApiError(500, "HTTP_500", "boom"))).toBe(false);
    expect(retry(1, new Error("network"))).toBe(true);
  });
});
