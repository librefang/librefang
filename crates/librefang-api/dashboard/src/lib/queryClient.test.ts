import { afterEach, describe, expect, it, vi } from "vitest";
import { createDashboardQueryClient } from "./queryClient";
import { useUIStore } from "./store";

afterEach(() => {
  useUIStore.setState({ toasts: [] });
});

describe("dashboard mutation errors", () => {
  it("shows an error toast when a mutation has no local handler", async () => {
    const client = createDashboardQueryClient();
    const mutation = client.getMutationCache().build(client, {
      mutationFn: async () => {
        throw new Error("save failed");
      },
    });

    await expect(mutation.execute(undefined)).rejects.toThrow("save failed");

    expect(useUIStore.getState().toasts).toMatchObject([
      { message: "save failed", type: "error" },
    ]);
  });

  it("defers to a mutation-specific error handler", async () => {
    const client = createDashboardQueryClient();
    const onError = vi.fn();
    const mutation = client.getMutationCache().build(client, {
      mutationFn: async () => {
        throw new Error("save failed");
      },
      onError,
    });

    await expect(mutation.execute(undefined)).rejects.toThrow("save failed");

    expect(onError).toHaveBeenCalledOnce();
    expect(useUIStore.getState().toasts).toEqual([]);
  });
});
