import { beforeEach, describe, expect, it, vi } from "vitest";

function setUserAgent(value: string) {
  Object.defineProperty(navigator, "userAgent", { value, configurable: true });
}

describe("tauri bridge", () => {
  beforeEach(() => {
    vi.resetModules();
    sessionStorage.clear();
    setUserAgent("Desktop Browser");
    delete window.__TAURI__;
  });

  it("keeps browser credentials in memory and removes legacy plaintext storage", async () => {
    sessionStorage.setItem("lf_creds", "{corrupted-json");
    const { getCredentials, storeCredentials } = await import("./tauri");
    await storeCredentials({ base_url: "https://example.test", api_key: "secret" });

    expect(sessionStorage.getItem("lf_creds")).toBeNull();
    expect(await getCredentials()).toEqual({
      base_url: "https://example.test",
      api_key: "secret",
    });
    expect(sessionStorage.getItem("lf_creds")).toBeNull();
  });

  it("propagates mobile credential storage failures consistently", async () => {
    setUserAgent("Android");
    const invoke = vi.fn().mockRejectedValue(new Error("keyring unavailable"));
    window.__TAURI__ = { core: { invoke } };
    const { clearCredentials, getCredentials, storeCredentials } = await import("./tauri");

    await expect(
      storeCredentials({ base_url: "https://example.test", api_key: "secret" }),
    ).rejects.toThrow("keyring unavailable");
    await expect(getCredentials()).rejects.toThrow("keyring unavailable");
    await expect(clearCredentials()).rejects.toThrow("keyring unavailable");
  });

  it("distinguishes unsupported, cancelled, failed, and successful QR scans", async () => {
    const desktop = await import("./tauri");
    await expect(desktop.scanQrCode()).resolves.toEqual({ status: "unsupported" });

    vi.resetModules();
    setUserAgent("Android");
    const invoke = vi.fn()
      .mockResolvedValueOnce({})
      .mockRejectedValueOnce(new Error("camera denied"))
      .mockResolvedValueOnce({ content: "payload" });
    window.__TAURI__ = { core: { invoke } };
    const mobile = await import("./tauri");
    await expect(mobile.scanQrCode()).resolves.toEqual({ status: "cancelled" });
    await expect(mobile.scanQrCode()).resolves.toMatchObject({ status: "error" });
    await expect(mobile.scanQrCode()).resolves.toEqual({
      status: "success",
      content: "payload",
    });
  });
});
