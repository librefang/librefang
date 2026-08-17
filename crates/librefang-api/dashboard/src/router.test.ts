import { describe, expect, it, vi } from "vitest";
import {
  classifyRouteError,
  parseCanvasSearch,
  readReloadTimestamp,
  writeReloadTimestamp,
} from "./router";

describe("parseCanvasSearch", () => {
  it("accepts finite numeric timestamps and string workflow IDs", () => {
    expect(parseCanvasSearch({ t: "1712345678", wf: "daily-report" })).toEqual({
      t: 1712345678,
      wf: "daily-report",
    });
    expect(parseCanvasSearch({ t: 42, wf: "workflow" })).toEqual({ t: 42, wf: "workflow" });
  });

  it("drops invalid search values", () => {
    expect(parseCanvasSearch({ t: "not-a-number", wf: 42 })).toEqual({});
    expect(parseCanvasSearch({ t: Number.POSITIVE_INFINITY })).toEqual({});
    expect(parseCanvasSearch({ t: "" })).toEqual({});
  });
});

describe("classifyRouteError", () => {
  it("recognizes stale chunks", () => {
    expect(classifyRouteError(new Error("Failed to fetch dynamically imported module: /assets/page.js"))).toBe("chunk");
  });

  it("only recognizes the exact development dispatcher-null signature", () => {
    expect(classifyRouteError(new TypeError("Cannot read properties of null (reading 'useState')"))).toBe("dispatcher");
    expect(classifyRouteError(new Error("Component failed while reading 'useState' data"))).toBe("other");
    expect(classifyRouteError(new Error("Cannot read properties of undefined (reading 'useState')"))).toBe("other");
  });
});

describe("reload timestamp storage", () => {
  function createHistory(initialState: unknown = null) {
    let state = initialState;
    return {
      get state() { return state; },
      replaceState: vi.fn((nextState: unknown) => { state = nextState; }),
    };
  }

  it("falls back to history state when storage reads throw", () => {
    const storage = { getItem: vi.fn(() => { throw new DOMException("blocked", "SecurityError"); }) };
    const history = createHistory({ __librefang_chunk_reload: 123 });
    expect(readReloadTimestamp(storage, history)).toBe(123);
  });

  it("falls back when accessing session storage itself throws", () => {
    const storage = vi.spyOn(window, "sessionStorage", "get").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    const history = createHistory({ __librefang_chunk_reload: 123 });
    try {
      expect(readReloadTimestamp(undefined, history)).toBe(123);
    } finally {
      storage.mockRestore();
    }
  });

  it("keeps the storage timestamp when history state access throws", () => {
    const history = {
      get state(): never { throw new DOMException("blocked", "SecurityError"); },
      replaceState: vi.fn(),
    };
    expect(readReloadTimestamp({ getItem: vi.fn(() => "123") }, history)).toBe(123);
  });

  it("persists a reload guard in history when storage writes fail", () => {
    const storage = { setItem: vi.fn(() => { throw new DOMException("full", "QuotaExceededError"); }) };
    const history = createHistory({ router: "state" });
    expect(writeReloadTimestamp(123, storage, history)).toBe(true);
    expect(readReloadTimestamp({ getItem: vi.fn(() => null) }, history)).toBe(123);
    expect(history.state).toEqual({ router: "state", __librefang_chunk_reload: 123 });
  });

  it("persists in history when accessing session storage itself throws", () => {
    const storage = vi.spyOn(window, "sessionStorage", "get").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    const history = createHistory({ router: "state" });
    try {
      expect(writeReloadTimestamp(123, undefined, history)).toBe(true);
      expect(history.state).toEqual({ router: "state", __librefang_chunk_reload: 123 });
    } finally {
      storage.mockRestore();
    }
  });

  it("declines recovery when neither storage nor history can persist the guard", () => {
    const storage = { setItem: vi.fn(() => { throw new DOMException("blocked", "SecurityError"); }) };
    const history = {
      state: null,
      replaceState: vi.fn(() => { throw new DOMException("blocked", "SecurityError"); }),
    };
    expect(writeReloadTimestamp(123, storage, history)).toBe(false);
  });
});
