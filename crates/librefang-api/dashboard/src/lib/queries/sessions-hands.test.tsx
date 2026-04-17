import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { HandSettingsResponse, SessionDetailResponse } from "../../api";
import { useSessionDetails } from "./sessions";
import { useHandDetail, useHandSettings } from "./hands";
import { sessionKeys, handKeys } from "./keys";
import * as client from "../http/client";

function createWrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

vi.mock("../http/client", () => ({
  getSessionDetails: vi.fn(),
  getHandDetail: vi.fn(),
  getHandSettings: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useSessionDetails", () => {
  it("should be disabled when sessionId is empty string", () => {
    const { result } = renderHook(() => useSessionDetails(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getSessionDetails).not.toHaveBeenCalled();
  });

  it("should be enabled when sessionId is valid", async () => {
    const mockSession: SessionDetailResponse = { session_id: "sess-1" };
    vi.mocked(client.getSessionDetails).mockResolvedValue(mockSession);

    const { result } = renderHook(() => useSessionDetails("sess-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockSession);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getSessionDetails).toHaveBeenCalledWith("sess-1");
  });

  it("should use sessionKeys.detail(sessionId) as queryKey", async () => {
    vi.mocked(client.getSessionDetails).mockResolvedValue({ session_id: "sess-2" });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    renderHook(() => useSessionDetails("sess-2"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find({ queryKey: sessionKeys.detail("sess-2") })).toBeDefined();
    });
    expect(
      qc.getQueryCache().find({ queryKey: sessionKeys.detail("sess-2") })?.queryKey,
    ).toEqual(sessionKeys.detail("sess-2"));
  });
});

describe("useHandDetail", () => {
  it("should be disabled when handId is empty string", () => {
    const { result } = renderHook(() => useHandDetail(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandDetail).not.toHaveBeenCalled();
  });

  it("should be enabled when handId is valid", async () => {
    const mockHand = { id: "hand-1", name: "Test Hand" };
    vi.mocked(client.getHandDetail).mockResolvedValue(mockHand);

    const { result } = renderHook(() => useHandDetail("hand-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockHand);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandDetail).toHaveBeenCalledWith("hand-1");
  });

  it("should use handKeys.detail(handId) as queryKey", async () => {
    vi.mocked(client.getHandDetail).mockResolvedValue({ id: "hand-2" });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    renderHook(() => useHandDetail("hand-2"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find({ queryKey: handKeys.detail("hand-2") })).toBeDefined();
    });
    expect(
      qc.getQueryCache().find({ queryKey: handKeys.detail("hand-2") })?.queryKey,
    ).toEqual(handKeys.detail("hand-2"));
  });
});

describe("useHandSettings", () => {
  it("should be disabled when handId is empty string", () => {
    const { result } = renderHook(() => useHandSettings(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandSettings).not.toHaveBeenCalled();
  });

  it("should be enabled when handId is valid", async () => {
    const mockSettings: HandSettingsResponse = { hand_id: "hand-3", current_values: { theme: "dark" } };
    vi.mocked(client.getHandSettings).mockResolvedValue(mockSettings);

    const { result } = renderHook(() => useHandSettings("hand-3"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockSettings);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getHandSettings).toHaveBeenCalledWith("hand-3");
  });

  it("should use handKeys.settings(handId) as queryKey", async () => {
    vi.mocked(client.getHandSettings).mockResolvedValue({ hand_id: "hand-4" });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    renderHook(() => useHandSettings("hand-4"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find({ queryKey: handKeys.settings("hand-4") })).toBeDefined();
    });
    expect(
      qc.getQueryCache().find({ queryKey: handKeys.settings("hand-4") })?.queryKey,
    ).toEqual(handKeys.settings("hand-4"));
  });
});
