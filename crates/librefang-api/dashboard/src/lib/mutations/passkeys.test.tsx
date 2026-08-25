import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import * as api from "../../api";
import { createQueryClientWrapper } from "../test/query-client";
import { passkeyKeys } from "../queries/keys";
import { useRegisterPasskey, useRevokePasskey } from "./passkeys";

vi.mock("../../api", async () => {
  const actual = await vi.importActual<typeof import("../../api")>("../../api");
  return {
    ...actual,
    registerPasskey: vi.fn().mockResolvedValue({}),
    revokePasskey: vi.fn().mockResolvedValue({}),
  };
});

describe("passkey mutations", () => {
  it("invalidates only passkey list queries after registration", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRegisterPasskey(), { wrapper });

    await result.current.mutateAsync("laptop");

    await waitFor(() => expect(invalidateSpy).toHaveBeenCalledOnce());
    expect(api.registerPasskey).toHaveBeenCalledWith("laptop");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: passkeyKeys.lists() });
  });

  it("invalidates only passkey list queries after revocation", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRevokePasskey(), { wrapper });

    await result.current.mutateAsync("credential-1");

    await waitFor(() => expect(invalidateSpy).toHaveBeenCalledOnce());
    expect(api.revokePasskey).toHaveBeenCalledWith("credential-1");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: passkeyKeys.lists() });
  });
});
