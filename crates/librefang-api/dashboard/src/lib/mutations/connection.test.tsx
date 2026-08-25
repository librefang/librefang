import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { storeCredentials } from "../tauri";
import { createQueryClientWrapper } from "../test/query-client";
import { useConnectManual, useConnectViaQr } from "./connection";

vi.mock("../tauri", () => ({ storeCredentials: vi.fn().mockResolvedValue(undefined) }));

describe("connection mutations", () => {
  beforeEach(() => {
    vi.mocked(storeCredentials).mockClear();
    vi.stubGlobal("fetch", vi.fn());
  });

  const renderConnectionHook = <T,>(hook: () => T) => {
    const { wrapper } = createQueryClientWrapper();
    return renderHook(hook, { wrapper });
  };

  it("rejects unsafe URL shapes before sending credentials", async () => {
    const { result } = renderConnectionHook(() => useConnectManual());

    await expect(result.current.mutateAsync({
      baseUrl: "https://user:secret@example.com",
      apiKey: "key",
    })).rejects.toThrow("must not include credentials");
    expect(fetch).not.toHaveBeenCalled();
  });

  it("stores manual credentials but returns only the non-sensitive URL", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(new Response(null, { status: 200 }));
    const { result } = renderConnectionHook(() => useConnectManual());

    await expect(result.current.mutateAsync({
      baseUrl: "http://192.168.1.10:4545/",
      apiKey: "secret",
    })).resolves.toEqual({ baseUrl: "http://192.168.1.10:4545" });
    expect(storeCredentials).toHaveBeenCalledWith({
      base_url: "http://192.168.1.10:4545",
      api_key: "secret",
    });
  });

  it("maps transport failures to connection guidance", async () => {
    vi.mocked(fetch).mockRejectedValueOnce(new TypeError("Failed to fetch"));
    const { result } = renderConnectionHook(() => useConnectManual());

    await expect(result.current.mutateAsync({
      baseUrl: "https://example.com",
      apiKey: "key",
    })).rejects.toThrow("Could not reach the server");
  });

  it("rejects malformed pairing responses without storing credentials", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(new Response("{}", {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }));
    const { result } = renderConnectionHook(() => useConnectViaQr());

    await expect(result.current.mutateAsync({
      baseUrl: "https://example.com",
      token: "token",
      displayName: "phone",
      platform: "ios",
    })).rejects.toThrow("invalid pairing response");
    expect(storeCredentials).not.toHaveBeenCalled();
  });

  it("does not retain the paired API key in mutation data", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(new Response(JSON.stringify({ api_key: "paired-secret" }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }));
    const { result } = renderConnectionHook(() => useConnectViaQr());

    await expect(result.current.mutateAsync({
      baseUrl: "https://example.com/",
      token: "token",
      displayName: "phone",
      platform: "ios",
    })).resolves.toEqual({ baseUrl: "https://example.com" });
    expect(storeCredentials).toHaveBeenCalledWith({
      base_url: "https://example.com",
      api_key: "paired-secret",
    });
  });
});
