import { afterEach, describe, expect, it, vi } from "vitest";
import { MutationObserver as QueryMutationObserver } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";
import { createDashboardQueryClient } from "./queryClient";
import { useUIStore } from "./store";

afterEach(() => {
  useUIStore.setState({ toasts: [] });
});

// A subscribed observer is what `useMutation()` is under the hood, and query-core runs `mutate(vars, { … })` callbacks only while the observer has a listener.
function subscribedObserver(client: QueryClient, message = "save failed") {
  const observer = new QueryMutationObserver<void, Error>(client, {
    mutationFn: async (): Promise<void> => {
      throw new Error(message);
    },
  });
  const unsubscribe = observer.subscribe(() => {});
  return { observer, unsubscribe };
}

// The fallback toast is decided one macrotask after the failure, so a test that asserts its absence has to let that turn run first.
function flushFallback(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("dashboard mutation errors", () => {
  it("shows an error toast when a mutation has no local handler", async () => {
    const client = createDashboardQueryClient();
    const mutation = client.getMutationCache().build(client, {
      mutationFn: async () => {
        throw new Error("save failed");
      },
    });

    await expect(mutation.execute(undefined)).rejects.toThrow("save failed");

    await vi.waitFor(() =>
      expect(useUIStore.getState().toasts).toMatchObject([
        { message: "save failed", type: "error" },
      ]));
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
    await flushFallback();

    expect(onError).toHaveBeenCalledOnce();
    expect(useUIStore.getState().toasts).toEqual([]);
  });

  it("defers to an error handler passed at the mutate() call site", async () => {
    const client = createDashboardQueryClient();
    const { observer, unsubscribe } = subscribedObserver(client);

    await expect(
      observer.mutate(undefined, {
        onError: () => useUIStore.getState().addToast("install failed", "error"),
      }),
    ).rejects.toThrow("save failed");
    await flushFallback();

    // One toast, the specific one — `mutation.options` never carries a call-site handler, so the generic fallback used to stack on top of it.
    expect(useUIStore.getState().toasts).toMatchObject([
      { message: "install failed", type: "error" },
    ]);
    unsubscribe();
  });

  it("defers to a mutateAsync caller that reports the failure from its own catch", async () => {
    const client = createDashboardQueryClient();
    const { observer, unsubscribe } = subscribedObserver(client);

    let rejected = false;
    try {
      await observer.mutate(undefined);
    } catch {
      rejected = true;
      useUIStore.getState().addToast("delete failed", "error");
    }
    await flushFallback();

    expect(rejected).toBe(true);
    expect(useUIStore.getState().toasts).toMatchObject([
      { message: "delete failed", type: "error" },
    ]);
    unsubscribe();
  });

  it("still toasts when a call-site handler leaves the failure invisible", async () => {
    const client = createDashboardQueryClient();
    const { observer, unsubscribe } = subscribedObserver(client);
    const onError = vi.fn();

    await expect(observer.mutate(undefined, { onError })).rejects.toThrow("save failed");

    expect(onError).toHaveBeenCalledOnce();
    await vi.waitFor(() =>
      expect(useUIStore.getState().toasts).toMatchObject([
        { message: "save failed", type: "error" },
      ]));
    unsubscribe();
  });

  it("keeps every message when several unreported failures land in the same tick", async () => {
    // ProvidersPage runs four concurrent `testMutation.mutateAsync` workers whose hook declares no `onError` and whose call site swallows the rejection, so the fallback owns all four messages — and they differ per provider.
    // The fallback must not read its own first toast as somebody else having reported the second failure.
    const client = createDashboardQueryClient();
    const first = subscribedObserver(client, "provider 1 unauthorized");
    const second = subscribedObserver(client, "provider 2 timed out");

    const settled = await Promise.allSettled([
      first.observer.mutate(undefined),
      second.observer.mutate(undefined),
    ]);
    expect(settled.map((result) => result.status)).toEqual(["rejected", "rejected"]);

    await vi.waitFor(() =>
      expect(useUIStore.getState().toasts).toMatchObject([
        { message: "provider 1 unauthorized", type: "error" },
        { message: "provider 2 timed out", type: "error" },
      ]));
    first.unsubscribe();
    second.unsubscribe();
  });
});
