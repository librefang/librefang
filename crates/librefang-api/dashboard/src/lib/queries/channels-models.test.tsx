import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useCommsEvents } from "./channels";
import { useModels, useModelOverrides } from "./models";
import * as httpClient from "../http/client";
import { commsKeys, modelKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  listCommsEvents: vi.fn(),
  listModels: vi.fn(),
  getModelOverrides: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useCommsEvents", () => {
  it("should fetch when enabled is undefined (default)", async () => {
    const mockEvents = [{ id: "evt-1", kind: "message" }];
    vi.mocked(httpClient.listCommsEvents).mockResolvedValue(mockEvents);

    const { result } = renderHook(() => useCommsEvents(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockEvents);
    expect(httpClient.listCommsEvents).toHaveBeenCalled();
  });

  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useCommsEvents(50, { enabled: false }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.listCommsEvents).not.toHaveBeenCalled();
  });

  it("should fetch when enabled is true", async () => {
    const mockEvents = [{ id: "evt-1", kind: "message" }];
    vi.mocked(httpClient.listCommsEvents).mockResolvedValue(mockEvents);

    const { result } = renderHook(() => useCommsEvents(50, { enabled: true }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockEvents);
    expect(httpClient.listCommsEvents).toHaveBeenCalledWith(50);
  });

  it("should use the correct queryKey", async () => {
    const mockEvents: Array<{ id: string; kind: string }> = [];
    vi.mocked(httpClient.listCommsEvents).mockResolvedValue(mockEvents);

    const { queryClient, wrapper } = createQueryClientWrapper();
    renderHook(() => useCommsEvents(100, { enabled: true }), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(commsKeys.events(100))).toEqual(mockEvents);
    });
  });
});

describe("useModels", () => {
  it("should fetch when enabled is undefined (default)", async () => {
    const mockResponse = { models: [{ id: "gpt-4", provider: "openai" }], total: 1, available: 1 };
    vi.mocked(httpClient.listModels).mockResolvedValue(mockResponse);

    const { result } = renderHook(() => useModels(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockResponse);
    expect(httpClient.listModels).toHaveBeenCalled();
  });

  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useModels({}, { enabled: false }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.listModels).not.toHaveBeenCalled();
  });

  it("should fetch when enabled is true", async () => {
    const mockResponse = { models: [{ id: "gpt-4", provider: "openai" }], total: 1, available: 1 };
    vi.mocked(httpClient.listModels).mockResolvedValue(mockResponse);

    const { result } = renderHook(() => useModels({}, { enabled: true }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockResponse);
    expect(httpClient.listModels).toHaveBeenCalledWith({});
  });

  it("should pass filters to the API call", async () => {
    const mockResponse = { models: [{ id: "claude-3", provider: "anthropic" }], total: 1, available: 1 };
    vi.mocked(httpClient.listModels).mockResolvedValue(mockResponse);

    const filters = { provider: "anthropic", tier: "premium" };
    const { result } = renderHook(() => useModels(filters, { enabled: true }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(httpClient.listModels).toHaveBeenCalledWith(filters);
  });

  it("should use the correct queryKey", async () => {
    const mockResponse = { models: [], total: 0, available: 0 };
    vi.mocked(httpClient.listModels).mockResolvedValue(mockResponse);

    const filters = { provider: "openai" };
    const { queryClient, wrapper } = createQueryClientWrapper();
    renderHook(() => useModels(filters, { enabled: true }), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(modelKeys.list(filters))).toEqual(mockResponse);
    });
  });
});

describe("useModelOverrides", () => {
  it("should be disabled when modelKey is empty string", () => {
    const { result } = renderHook(() => useModelOverrides(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getModelOverrides).not.toHaveBeenCalled();
  });

  it("should fetch when modelKey is valid", async () => {
    const mockOverrides = { temperature: 0.7, max_tokens: 4096 };
    vi.mocked(httpClient.getModelOverrides).mockResolvedValue(mockOverrides);

    const { result } = renderHook(() => useModelOverrides("gpt-4"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockOverrides);
    expect(httpClient.getModelOverrides).toHaveBeenCalledWith("gpt-4");
  });

  it("should use the correct queryKey", async () => {
    const mockOverrides = {};
    vi.mocked(httpClient.getModelOverrides).mockResolvedValue(mockOverrides);

    const { queryClient, wrapper } = createQueryClientWrapper();
    renderHook(() => useModelOverrides("claude-3"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(modelKeys.overrides("claude-3"))).toEqual(mockOverrides);
    });
  });
});
