import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { RegistrySchema } from "../../api";
import { useRegistrySchema, useRawConfigToml } from "./config";
import * as client from "../http/client";
import { registryKeys, configKeys } from "./keys";

function createWrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

vi.mock("../http/client", () => ({
  fetchRegistrySchema: vi.fn(),
  getRawConfigToml: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useRegistrySchema", () => {
  it("should be disabled when contentType is empty string", () => {
    const { result } = renderHook(() => useRegistrySchema(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.fetchRegistrySchema).not.toHaveBeenCalled();
  });

  it("should be enabled when contentType is valid", async () => {
    const mockSchema: RegistrySchema = { fields: {} };
    vi.mocked(client.fetchRegistrySchema).mockResolvedValue(mockSchema);

    const { result } = renderHook(() => useRegistrySchema("application/json"), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockSchema);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.fetchRegistrySchema).toHaveBeenCalledWith("application/json");
  });

  it("should use registryKeys.schema(contentType) as queryKey", async () => {
    const mockSchema: RegistrySchema = { sections: {} };
    vi.mocked(client.fetchRegistrySchema).mockResolvedValue(mockSchema);

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => useRegistrySchema("text/plain"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find({ queryKey: registryKeys.schema("text/plain") })).toBeDefined();
    });

    expect(
      qc.getQueryCache().find({ queryKey: registryKeys.schema("text/plain") })?.queryKey,
    ).toEqual(registryKeys.schema("text/plain"));
  });
});

describe("useRawConfigToml", () => {
  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useRawConfigToml(false), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getRawConfigToml).not.toHaveBeenCalled();
  });

  it("should fetch when enabled is true", async () => {
    const mockToml = "[kernel]\nlog_level = \"info\"";
    vi.mocked(client.getRawConfigToml).mockResolvedValue(mockToml);

    const { result } = renderHook(() => useRawConfigToml(true), {
      wrapper: createWrapper(),
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockToml);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getRawConfigToml).toHaveBeenCalled();
  });

  it("should use configKeys.rawToml() as queryKey", async () => {
    vi.mocked(client.getRawConfigToml).mockResolvedValue("toml content");

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => useRawConfigToml(true), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find({ queryKey: configKeys.rawToml() })).toBeDefined();
    });

    expect(
      qc.getQueryCache().find({ queryKey: configKeys.rawToml() })?.queryKey,
    ).toEqual(configKeys.rawToml());
  });
});
