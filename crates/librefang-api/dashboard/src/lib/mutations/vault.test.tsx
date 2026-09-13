import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import * as api from "../../api";
import { createQueryClientWrapper } from "../test/query-client";
import { vaultKeys } from "../queries/keys";
import { useDeleteVaultKey, useSetVaultKey } from "./vault";

vi.mock("../../api", async () => {
  const actual = await vi.importActual<typeof import("../../api")>("../../api");
  return {
    ...actual,
    setVaultKey: vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: true }),
    deleteVaultKey: vi
      .fn()
      .mockResolvedValue({ key: "GITHUB_TOKEN", set: false, removed: true }),
  };
});

describe("vault mutations", () => {
  it("invalidates the vault listing after a write", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useSetVaultKey(), { wrapper });

    await result.current.mutateAsync({ key: "GITHUB_TOKEN", value: "ghp_fixture" });

    await waitFor(() => expect(invalidateSpy).toHaveBeenCalledOnce());
    expect(api.setVaultKey).toHaveBeenCalledWith("GITHUB_TOKEN", "ghp_fixture");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: vaultKeys.lists() });
  });

  it("invalidates the vault listing after a delete", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteVaultKey(), { wrapper });

    await result.current.mutateAsync({ key: "GITHUB_TOKEN" });

    await waitFor(() => expect(invalidateSpy).toHaveBeenCalledOnce());
    expect(api.deleteVaultKey).toHaveBeenCalledWith("GITHUB_TOKEN");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: vaultKeys.lists() });
  });
});
