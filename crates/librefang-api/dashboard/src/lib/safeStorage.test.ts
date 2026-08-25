import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { safeStorageGet, safeStorageRead, safeStorageSet } from "./safeStorage";

// jsdom (v29) does not ship a working `localStorage` global, and the
// helper deliberately reads `globalThis.localStorage?.` so it degrades
// when storage is absent / throwing (Safari private mode,
// QuotaExceededError). Inject a controllable fake per test.

type FakeStore = {
  getItem: (k: string) => string | null;
  setItem: (k: string, v: string) => void;
};

const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");

function install(fake: FakeStore | undefined) {
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: fake,
  });
}

beforeEach(() => {
  const map = new Map<string, string>();
  install({
    getItem: (k) => (map.has(k) ? (map.get(k) as string) : null),
    setItem: (k, v) => {
      map.set(k, v);
    },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  if (original) Object.defineProperty(globalThis, "localStorage", original);
  else delete (globalThis as { localStorage?: unknown }).localStorage;
});

describe("safeStorageGet", () => {
  it("returns the stored value", () => {
    globalThis.localStorage.setItem("k", "v");
    expect(safeStorageGet("k")).toBe("v");
  });

  it("returns null for a missing key", () => {
    expect(safeStorageGet("missing")).toBeNull();
    expect(safeStorageRead("missing")).toEqual({ ok: true, value: null });
  });

  it("returns null when localStorage is absent (SSR / non-browser)", () => {
    install(undefined);
    expect(safeStorageGet("k")).toBeNull();
    expect(safeStorageRead("k")).toEqual({ ok: false, reason: "unavailable" });
  });

  it("returns null instead of throwing when getItem throws (Safari private mode)", () => {
    install({
      getItem: () => {
        throw new DOMException("blocked", "SecurityError");
      },
      setItem: () => {},
    });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(safeStorageGet("k")).toBeNull();
    expect(safeStorageRead("k")).toMatchObject({ ok: false, reason: "error" });
    expect(warn).toHaveBeenCalled();
  });
});

describe("safeStorageSet", () => {
  it("writes the value", () => {
    safeStorageSet("k", "v");
    expect(globalThis.localStorage.getItem("k")).toBe("v");
  });

  it("warns when a write is skipped because localStorage is absent", () => {
    install(undefined);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(() => safeStorageSet("k", "v")).not.toThrow();
    expect(warn).toHaveBeenCalledWith(
      'safeStorageSet("k") skipped: localStorage unavailable',
    );
  });

  it("swallows QuotaExceededError instead of throwing", () => {
    install({
      getItem: () => null,
      setItem: () => {
        throw new DOMException("full", "QuotaExceededError");
      },
    });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(() => safeStorageSet("k", "v")).not.toThrow();
    expect(warn).toHaveBeenCalled();
  });
});
