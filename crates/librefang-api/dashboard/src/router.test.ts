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
  it("falls back when storage reads throw", () => {
    const storage = { getItem: vi.fn(() => { throw new DOMException("blocked", "SecurityError"); }) };
    expect(readReloadTimestamp(storage)).toBe(0);
  });

  it("ignores storage write failures", () => {
    const storage = { setItem: vi.fn(() => { throw new DOMException("full", "QuotaExceededError"); }) };
    expect(() => writeReloadTimestamp(123, storage)).not.toThrow();
  });
});
