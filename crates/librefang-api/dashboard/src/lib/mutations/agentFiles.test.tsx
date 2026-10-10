import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";

import { useSetAgentFile } from "./agentFiles";
import * as httpClient from "../http/client";
import { agentFileKeys, agentKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  setAgentFile: vi.fn().mockResolvedValue({
    status: "ok",
    name: "IDENTITY.md",
    size_bytes: 31,
  }),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useSetAgentFile", () => {
  it("writes the whole file through setAgentFile", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(
      () => useSetAgentFile("agent-1", "IDENTITY.md"),
      { wrapper },
    );

    const content = "---\nvibe: helpful\n---\n# Identity\n";
    await result.current.mutateAsync(content);

    expect(httpClient.setAgentFile).toHaveBeenCalledWith(
      "agent-1",
      "IDENTITY.md",
      content,
    );
  });

  it("invalidates this file's read and the agent's file listing", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(
      () => useSetAgentFile("agent-1", "IDENTITY.md"),
      { wrapper },
    );

    await result.current.mutateAsync("---\n---\n");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentFileKeys.detail("agent-1", "IDENTITY.md"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentFileKeys.lists(),
    });
  });

  // The workspace file and the agent manifest are different stores: a save
  // here must not refetch the agent detail (and everything nested under it).
  it("does not invalidate anything under agentKeys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(
      () => useSetAgentFile("agent-1", "IDENTITY.md"),
      { wrapper },
    );

    await result.current.mutateAsync("---\n---\n");

    const touchedAgentKeys = invalidateSpy.mock.calls
      .map(([filters]) => filters?.queryKey?.[0])
      .filter((prefix) => prefix === "agents");
    expect(touchedAgentKeys).toEqual([]);
    expect(agentFileKeys.all[0]).not.toBe(agentKeys.all[0]);
  });
});
