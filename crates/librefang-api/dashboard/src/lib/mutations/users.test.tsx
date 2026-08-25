import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { authzKeys, userKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useCreateUser, useRotateUserKey, useUpdateUser } from "./users";

vi.mock("../http/client", () => ({
  createUser: vi.fn(),
  updateUser: vi.fn(),
  deleteUser: vi.fn(),
  importUsers: vi.fn(),
  rotateUserKey: vi.fn(),
  updateUserPolicy: vi.fn(),
}));

const user = {
  name: "alice",
  role: "Operator",
  channel_bindings: {},
  has_api_key: false,
  has_policy: false,
  has_memory_access: false,
  has_budget: false,
};

describe("user identity mutation caches", () => {
  beforeEach(() => {
    vi.mocked(http.createUser).mockReset().mockResolvedValue(user);
    vi.mocked(http.updateUser).mockReset().mockResolvedValue(user);
    vi.mocked(http.rotateUserKey).mockReset().mockResolvedValue({
      status: "rotated",
      new_api_key: "one-time-key",
      sessions_invalidated: 0,
    });
  });

  it("refreshes prior detail and authz misses after create", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCreateUser(), { wrapper });

    await result.current.mutateAsync({ name: "alice", role: "Operator" });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: userKeys.lists() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: userKeys.detail("alice") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: authzKeys.effective("alice") });
  });

  it("evicts the old identity and refreshes the new identity after rename", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const removeSpy = vi.spyOn(queryClient, "removeQueries");
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateUser(), { wrapper });

    await result.current.mutateAsync({
      originalName: "alice",
      payload: { name: "alicia", role: "Operator" },
    });

    expect(removeSpy).toHaveBeenCalledWith({ queryKey: userKeys.detail("alice") });
    expect(removeSpy).toHaveBeenCalledWith({ queryKey: authzKeys.effective("alice") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: userKeys.detail("alicia") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: authzKeys.effective("alicia") });
  });

  it("invalidates the existing identity after a non-rename update", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const removeSpy = vi.spyOn(queryClient, "removeQueries");
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateUser(), { wrapper });

    await result.current.mutateAsync({
      originalName: "alice",
      payload: { name: "alice", role: "Admin" },
    });

    expect(removeSpy).not.toHaveBeenCalled();
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: userKeys.detail("alice") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: authzKeys.effective("alice") });
  });

  it("does not refresh permissions after credential-only key rotation", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRotateUserKey(), { wrapper });

    await result.current.mutateAsync("alice");

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: userKeys.detail("alice") });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: authzKeys.effective("alice") });
  });
});
