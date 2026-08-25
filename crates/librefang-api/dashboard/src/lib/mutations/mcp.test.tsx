import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { mcpKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import {
  useAddMcpServer,
  useDeleteMcpServer,
  useReconnectMcpServer,
  useUpdateMcpServer,
  useUpdateMcpTaintPolicy,
} from "./mcp";

vi.mock("../http/client", () => ({
  addMcpServer: vi.fn().mockResolvedValue({}),
  updateMcpServer: vi.fn().mockResolvedValue({}),
  patchMcpServerTaint: vi.fn().mockResolvedValue({}),
  deleteMcpServer: vi.fn().mockResolvedValue({}),
  reconnectMcpServer: vi.fn().mockResolvedValue({}),
  reloadMcp: vi.fn().mockResolvedValue({}),
  startMcpAuth: vi.fn().mockResolvedValue({}),
  revokeMcpAuth: vi.fn().mockResolvedValue({}),
}));

function expectServerInvalidation(
  invalidateSpy: ReturnType<typeof vi.spyOn>,
  id: string,
) {
  for (const queryKey of [
    mcpKeys.servers(),
    mcpKeys.server(id),
    mcpKeys.authStatus(id),
    mcpKeys.health(),
  ]) {
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey });
  }
}

describe("MCP mutations", () => {
  it("refreshes server lists and health after add", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useAddMcpServer(), { wrapper });

    await result.current.mutateAsync({ template_id: "docs" });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: mcpKeys.servers() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: mcpKeys.health() });
  });

  it("uses complete per-server invalidation after update and reconnect", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const update = renderHook(() => useUpdateMcpServer(), { wrapper });
    const reconnect = renderHook(() => useReconnectMcpServer(), { wrapper });

    await update.result.current.mutateAsync({
      id: "server-1",
      server: { name: "docs" },
    });
    await reconnect.result.current.mutateAsync("server-1");

    expectServerInvalidation(invalidateSpy, "server-1");
  });

  it("removes orphaned detail and auth caches after delete", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const removeSpy = vi.spyOn(queryClient, "removeQueries");
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteMcpServer(), { wrapper });

    await result.current.mutateAsync("server-1");

    expect(removeSpy).toHaveBeenCalledWith({ queryKey: mcpKeys.server("server-1") });
    expect(removeSpy).toHaveBeenCalledWith({ queryKey: mcpKeys.authStatus("server-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: mcpKeys.servers() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: mcpKeys.health() });
  });

  it("rejects empty taint updates before the network", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useUpdateMcpTaintPolicy(), { wrapper });

    await expect(result.current.mutateAsync({ id: "server-1" })).rejects.toThrow(
      "requires at least one changed field",
    );
    expect(http.patchMcpServerTaint).not.toHaveBeenCalled();
  });
});
