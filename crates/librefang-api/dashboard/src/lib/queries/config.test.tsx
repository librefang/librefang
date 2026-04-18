import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import type { RegistrySchema } from "../../api";
import { useRegistrySchema, useRawConfigToml } from "./config";
import * as client from "../http/client";
import { registryKeys, configKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

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
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
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

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useRegistrySchema("text/plain"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(registryKeys.schema("text/plain"))).toEqual(mockSchema);
    });
  });
});

describe("useRawConfigToml", () => {
  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useRawConfigToml(false), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
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
    const mockToml = "toml content";
    vi.mocked(client.getRawConfigToml).mockResolvedValue(mockToml);

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useRawConfigToml(true), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(configKeys.rawToml())).toEqual(mockToml);
    });
  });
});
