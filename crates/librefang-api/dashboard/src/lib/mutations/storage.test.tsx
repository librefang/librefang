import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  useUpdateStorageConfig,
  useMigrateStorage,
  useLinkUarStorage,
  useUnlinkUarStorage,
} from "./storage";
import { storageKeys, overviewKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  updateStorageConfig: vi.fn().mockResolvedValue({ ok: true }),
  migrateStorage: vi.fn().mockResolvedValue({
    dry_run: true,
    source: "sqlite",
    target: "surreal",
    copied: { audit_entries: 5 },
    errors: {},
    started_at: "2026-01-01T00:00:00Z",
    finished_at: "2026-01-01T00:00:01Z",
  }),
  linkUarStorage: vi.fn().mockResolvedValue({
    ok: true,
    namespace: "librefang_prod",
    app_user: "uar_app",
    memory_linked: true,
  }),
  unlinkUarStorage: vi.fn().mockResolvedValue({ ok: true }),
}));

describe("useUpdateStorageConfig", () => {
  it("invalidates storageKeys.all and overviewKeys.snapshot on success", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUpdateStorageConfig(), { wrapper });
    await result.current.mutateAsync({ backend_kind: "embedded" });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: storageKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useMigrateStorage", () => {
  it("invalidates storageKeys.all on success", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useMigrateStorage(), { wrapper });
    await result.current.mutateAsync({ from: "sqlite", dry_run: true });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: storageKeys.all });
  });
});

describe("useLinkUarStorage", () => {
  it("invalidates storageKeys.all and overviewKeys.snapshot on success", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useLinkUarStorage(), { wrapper });
    await result.current.mutateAsync({
      remote_url: "ws://surreal:8000",
      root_user: "root",
      root_pass_ref: "SURREAL_ROOT_PASS",
      app_pass_ref: "SURREAL_APP_PASS",
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: storageKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useUnlinkUarStorage", () => {
  it("invalidates storageKeys.all and overviewKeys.snapshot on success", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUnlinkUarStorage(), { wrapper });
    await result.current.mutateAsync(undefined);

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: storageKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});
