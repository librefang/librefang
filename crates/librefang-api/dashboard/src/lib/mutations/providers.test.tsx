import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { modelKeys, providerKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import {
  EveryApiConnectError,
  ProviderProbeError,
  useConnectEveryApi,
  useCreateRegistryContent,
  useValidateProviderKey,
} from "./providers";

vi.mock("../http/client", () => ({
  testProvider: vi.fn(),
  setProviderKey: vi.fn(),
  deleteProviderKey: vi.fn(),
  enableProvider: vi.fn(),
  setProviderUrl: vi.fn(),
  setProviderDiscovery: vi.fn(),
  setDefaultProvider: vi.fn(),
  createRegistryContent: vi.fn(),
}));

describe("provider mutation outcomes", () => {
  beforeEach(() => {
    vi.mocked(http.setProviderKey).mockReset().mockResolvedValue({ status: "ok" });
    vi.mocked(http.testProvider).mockReset().mockResolvedValue({
      status: "ok",
      message: "connected",
    });
    vi.mocked(http.createRegistryContent).mockReset().mockResolvedValue({
      ok: true,
      content_type: "provider",
      identifier: "everyapi",
      path: "providers/everyapi.toml",
    });
  });

  it("retains probe status and reconciles caches after a persisted key fails validation", async () => {
    vi.mocked(http.testProvider).mockResolvedValueOnce({
      status: "unauthorized",
      message: "invalid credential",
    });
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useValidateProviderKey(), { wrapper });

    const failure = await result.current.mutateAsync({
      providerId: "openai",
      apiKey: "secret",
    }).catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(ProviderProbeError);
    expect(failure).toMatchObject({ status: "unauthorized", message: "invalid credential" });
    expect(http.setProviderKey).toHaveBeenCalledWith("openai", "secret");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: providerKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: modelKeys.lists() });
  });

  it("distinguishes EveryAPI provider creation failures", async () => {
    vi.mocked(http.createRegistryContent).mockRejectedValueOnce(new Error("registry unavailable"));
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useConnectEveryApi(), { wrapper });

    const failure = await result.current.mutateAsync({ relayKey: "relay-key" })
      .catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(EveryApiConnectError);
    expect(failure).toMatchObject({ phase: "create" });
    expect(http.setProviderKey).not.toHaveBeenCalled();
  });

  it("distinguishes relay-key failures after EveryAPI creation", async () => {
    vi.mocked(http.setProviderKey).mockRejectedValueOnce(new Error("key store unavailable"));
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useConnectEveryApi(), { wrapper });

    const failure = await result.current.mutateAsync({ relayKey: "relay-key" })
      .catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(EveryApiConnectError);
    expect(failure).toMatchObject({ phase: "key_after_create" });
  });

  it("applies provider invalidation unconditionally to the provider-only registry hook", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCreateRegistryContent(), { wrapper });

    await result.current.mutateAsync({
      contentType: "provider",
      values: { id: "custom-provider" },
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: providerKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: modelKeys.lists() });
  });
});
