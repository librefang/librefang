import { afterEach, describe, expect, it, vi } from "vitest";

import { isApiPath, parseHttpBase, setupBundleMode } from "./bundleMode";

class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  readonly url: string;

  constructor(url: string | URL) {
    this.url = String(url);
  }
}

function stubTauriWindow(hash = "") {
  const fetch = vi.fn().mockResolvedValue(new Response());
  const fakeWindow = {
    location: {
      protocol: "tauri:",
      hash,
      pathname: "/index.html",
      search: "",
    },
    fetch,
    WebSocket: FakeWebSocket,
  } as unknown as Window & typeof globalThis;
  vi.stubGlobal("window", fakeWindow);
  return { fakeWindow, fetch };
}

afterEach(() => {
  localStorage.clear();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("bundle-mode base validation", () => {
  it("accepts only HTTP(S) daemon bases", () => {
    expect(parseHttpBase("https://daemon.example/")).toBe("https://daemon.example");
    expect(parseHttpBase("http://127.0.0.1:4545")).toBe("http://127.0.0.1:4545");
    expect(parseHttpBase("javascript:alert(1)")).toBeNull();
    expect(parseHttpBase("wss://daemon.example")).toBeNull();
    expect(parseHttpBase("not a url")).toBeNull();
  });

  it("does not classify the bare root as an API path", () => {
    expect(isApiPath("/")).toBe(false);
    expect(isApiPath("/api/health")).toBe(true);
  });

  it("does not persist or activate an invalid hash base", () => {
    const { fakeWindow, fetch } = stubTauriWindow("#api=javascript%3Aalert(1)");
    vi.spyOn(history, "replaceState").mockImplementation(() => undefined);

    setupBundleMode();

    expect(localStorage.getItem("librefang-api-base")).toBeNull();
    expect(fakeWindow.fetch).toBe(fetch);
  });

  it("rejects poisoned stored bases before patching globals", () => {
    localStorage.setItem("librefang-api-base", "wss://attacker.example");
    const { fakeWindow, fetch } = stubTauriWindow();

    setupBundleMode();

    expect(fakeWindow.fetch).toBe(fetch);
    expect(fakeWindow.WebSocket).toBe(FakeWebSocket);
  });

  it("rewrites API fetch and WebSocket URLs but leaves root untouched", async () => {
    localStorage.setItem("librefang-api-base", "https://daemon.example/");
    const { fakeWindow, fetch } = stubTauriWindow();

    setupBundleMode();

    await fakeWindow.fetch("/api/health");
    await fakeWindow.fetch("http://localhost:4545/api/agents?limit=1");
    await fakeWindow.fetch("/");

    expect(fetch).toHaveBeenNthCalledWith(1, "https://daemon.example/api/health", undefined);
    expect(fetch).toHaveBeenNthCalledWith(2, "https://daemon.example/api/agents?limit=1", undefined);
    expect(fetch).toHaveBeenNthCalledWith(3, "/", undefined);

    const relative = new fakeWindow.WebSocket("/api/events") as unknown as FakeWebSocket;
    const localhost = new fakeWindow.WebSocket("ws://localhost/api/events") as unknown as FakeWebSocket;
    const root = new fakeWindow.WebSocket("/") as unknown as FakeWebSocket;
    expect(relative.url).toBe("wss://daemon.example/api/events");
    expect(localhost.url).toBe("wss://daemon.example/api/events");
    expect(root.url).toBe("/");
  });
});
