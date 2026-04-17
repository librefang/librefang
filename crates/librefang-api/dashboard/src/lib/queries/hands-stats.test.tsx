import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { HandInstanceItem, HandInstanceStatus } from "../../api";
import {
  useHandStats,
  useHandStatsBatch,
  useHandSession,
  useHandInstanceStatus,
  useHandManifestToml,
  useActiveHandsWhen,
} from "./hands";
import * as client from "../http/client";
import { handKeys } from "./keys";

function createWrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

vi.mock("../http/client", () => ({
  getHandStats: vi.fn(),
  getHandSession: vi.fn(),
  getHandInstanceStatus: vi.fn(),
  getHandManifestToml: vi.fn(),
  listActiveHands: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useHandStats", () => {
  it("should be disabled when instanceId is empty string", () => {
    const { result } = renderHook(() => useHandStats(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandStats).not.toHaveBeenCalled();
  });

  it("should be enabled when instanceId is valid", async () => {
    const mockStats = {
      instance_id: "hand-1",
      hand_id: "my-hand",
      status: "active",
    };
    vi.mocked(client.getHandStats).mockResolvedValue(mockStats);

    const { result } = renderHook(() => useHandStats("hand-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockStats);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandStats).toHaveBeenCalledWith("hand-1");
  });

  it("should use handKeys.stats(instanceId) as queryKey", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    const mockStats = { instance_id: "hand-2" };
    vi.mocked(client.getHandStats).mockResolvedValue(mockStats);

    renderHook(() => useHandStats("hand-2"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryData(handKeys.stats("hand-2"))).toEqual(mockStats);
    });
  });
});

describe("useHandStatsBatch", () => {
  it("should be disabled when instanceIds is empty array", () => {
    const { result } = renderHook(() => useHandStatsBatch([]), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandStats).not.toHaveBeenCalled();
  });

  it("should be enabled when instanceIds has items", async () => {
    const stats1 = { instance_id: "h1", status: "active" };
    const stats2 = { instance_id: "h2", status: "paused" };
    vi.mocked(client.getHandStats).mockImplementation(async (id: string) => {
      if (id === "h1") return stats1;
      if (id === "h2") return stats2;
      throw new Error("unknown");
    });

    const { result } = renderHook(() => useHandStatsBatch(["h1", "h2"]), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);

    await waitFor(() => {
      expect(result.current.data).toEqual({ h1: stats1, h2: stats2 });
    });

    expect(client.getHandStats).toHaveBeenCalledTimes(2);
  });

  it("should use handKeys.statsBatch(instanceIds) as queryKey", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    vi.mocked(client.getHandStats).mockResolvedValue({});

    const ids = ["h1", "h2"] as const;
    renderHook(() => useHandStatsBatch(ids), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryData(handKeys.statsBatch(ids))).toBeDefined();
    });

    expect(qc.getQueryData(handKeys.statsBatch(ids))).toEqual({ h1: {}, h2: {} });
  });

  it("should skip failed requests gracefully", async () => {
    const stats1 = { instance_id: "h1" };
    vi.mocked(client.getHandStats).mockImplementation(async (id: string) => {
      if (id === "h1") return stats1;
      throw new Error("network error");
    });

    const { result } = renderHook(() => useHandStatsBatch(["h1", "h2"]), {
      wrapper: createWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toEqual({ h1: stats1 });
    });

    // h2 should not be in results because it failed
    expect(result.current.data).not.toHaveProperty("h2");
  });

  it("should fetch all requests in parallel", async () => {
    const resolveOrder: string[] = [];
    vi.mocked(client.getHandStats).mockImplementation(async (id: string) => {
      await new Promise((r) => setTimeout(r, Math.random() * 10));
      resolveOrder.push(id);
      return { instance_id: id };
    });

    const { result } = renderHook(() => useHandStatsBatch(["a", "b", "c"]), {
      wrapper: createWrapper(),
    });

    await waitFor(() => {
      expect(result.current.data).toBeDefined();
    });

    // All 3 calls should have been made (parallel, not sequential)
    expect(client.getHandStats).toHaveBeenCalledTimes(3);
  });
});

describe("useHandSession", () => {
  it("should be disabled when instanceId is empty string", () => {
    const { result } = renderHook(() => useHandSession(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandSession).not.toHaveBeenCalled();
  });

  it("should be enabled when instanceId is valid", async () => {
    const mockSession = {
      instance_id: "hand-1",
      session_id: "sess-123",
      messages: [],
    };
    vi.mocked(client.getHandSession).mockResolvedValue(mockSession);

    const { result } = renderHook(() => useHandSession("hand-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockSession);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandSession).toHaveBeenCalledWith("hand-1");
  });

  it("should use handKeys.session(instanceId) as queryKey", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    const mockSession = { instance_id: "hand-3", messages: [] };
    vi.mocked(client.getHandSession).mockResolvedValue(mockSession);

    renderHook(() => useHandSession("hand-3"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryData(handKeys.session("hand-3"))).toEqual(mockSession);
    });
  });
});

describe("useHandInstanceStatus", () => {
  it("should be disabled when instanceId is empty string", () => {
    const { result } = renderHook(() => useHandInstanceStatus(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandInstanceStatus).not.toHaveBeenCalled();
  });

  it("should be enabled when instanceId is valid", async () => {
    const mockStatus: HandInstanceStatus = {
      instance_id: "inst-1",
      hand_id: "hand-1",
      status: "running",
      activated_at: "2024-01-01T00:00:00Z",
      config: {},
    };
    vi.mocked(client.getHandInstanceStatus).mockResolvedValue(mockStatus);

    const { result } = renderHook(() => useHandInstanceStatus("inst-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockStatus);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandInstanceStatus).toHaveBeenCalledWith("inst-1");
  });

  it("should use handKeys.instanceStatus(instanceId) as queryKey", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    const mockStatus: HandInstanceStatus = { instance_id: "inst-2", hand_id: "hand-2", status: "running", activated_at: "2024-01-01T00:00:00Z", config: {} };
    vi.mocked(client.getHandInstanceStatus).mockResolvedValue(mockStatus);

    const { result } = renderHook(() => useHandInstanceStatus("inst-2"), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.data).toEqual(mockStatus);
    });

    expect(qc.getQueryData(handKeys.instanceStatus("inst-2"))).toEqual(
      mockStatus,
    );
  });
});

describe("useHandManifestToml", () => {
  it("should be disabled when handId is empty and enabled is true", () => {
    const { result } = renderHook(() => useHandManifestToml("", true), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandManifestToml).not.toHaveBeenCalled();
  });

  it("should be disabled when handId is valid and enabled is false", () => {
    const { result } = renderHook(
      () => useHandManifestToml("hand-x", false),
      { wrapper: createWrapper() },
    );

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandManifestToml).not.toHaveBeenCalled();
  });

  it("should be disabled when handId is empty and enabled is false", () => {
    const { result } = renderHook(
      () => useHandManifestToml("", false),
      { wrapper: createWrapper() },
    );

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandManifestToml).not.toHaveBeenCalled();
  });

  it("should be enabled when handId is valid and enabled is true", async () => {
    const mockToml = 'name = "my-hand"\nversion = "1.0"';
    vi.mocked(client.getHandManifestToml).mockResolvedValue(mockToml);

    const { result } = renderHook(
      () => useHandManifestToml("hand-x", true),
      { wrapper: createWrapper() },
    );

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockToml);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandManifestToml).toHaveBeenCalledWith("hand-x");
  });

  it("should use handKeys.manifest(handId) as queryKey", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    const mockToml = "name = 'y'";
    vi.mocked(client.getHandManifestToml).mockResolvedValue(mockToml);

    const { result } = renderHook(
      () => useHandManifestToml("hand-k", true),
      { wrapper },
    );

    await waitFor(() => {
      expect(result.current.data).toEqual(mockToml);
    });

    expect(qc.getQueryData(handKeys.manifest("hand-k"))).toEqual(mockToml);
  });
});

describe("useActiveHandsWhen", () => {
  it("should be disabled when enabled is false", () => {
    const { result } = renderHook(() => useActiveHandsWhen(false), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.listActiveHands).not.toHaveBeenCalled();
  });

  it("should be enabled when enabled is true", async () => {
    const mockHands: HandInstanceItem[] = [
      { instance_id: "inst-1", hand_id: "h1", status: "active" },
      { instance_id: "inst-2", hand_id: "h2", status: "active" },
    ];
    vi.mocked(client.listActiveHands).mockResolvedValue(mockHands);

    const { result } = renderHook(() => useActiveHandsWhen(true), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockHands);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.listActiveHands).toHaveBeenCalled();
  });

  it("should use handKeys.active() as queryKey", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    const mockHands: HandInstanceItem[] = [{ instance_id: "inst-1", hand_id: "h1" }];
    vi.mocked(client.listActiveHands).mockResolvedValue(mockHands);

    const { result } = renderHook(() => useActiveHandsWhen(true), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.data).toEqual(mockHands);
    });

    expect(qc.getQueryData(handKeys.active())).toEqual(mockHands);
  });
});
