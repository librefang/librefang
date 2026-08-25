import { describe, expect, it, vi } from "vitest";
import { MockResizeObserver } from "./setupTests";

describe.sequential("dashboard browser mocks", () => {
  it("allows a test to populate storage", () => {
    localStorage.setItem("leaked", "value");
    sessionStorage.setItem("leaked", "value");
    expect(localStorage.getItem("leaked")).toBe("value");
  });

  it("starts the next test with empty writable storage", () => {
    expect(localStorage.getItem("leaked")).toBeNull();
    expect(sessionStorage.getItem("leaked")).toBeNull();
    expect(
      Object.getOwnPropertyDescriptor(globalThis, "localStorage")?.writable,
    ).toBe(true);
  });

  it("tracks ResizeObserver targets and only delivers observed entries", () => {
    const callback = vi.fn();
    const observer = new MockResizeObserver(callback);
    const observed = document.createElement("div");
    const ignored = document.createElement("div");
    const observedEntry = { target: observed } as unknown as ResizeObserverEntry;
    const ignoredEntry = { target: ignored } as unknown as ResizeObserverEntry;

    observer.observe(observed);
    observer.trigger([observedEntry, ignoredEntry]);
    expect(callback).toHaveBeenCalledWith([observedEntry], observer);

    observer.unobserve(observed);
    observer.trigger([observedEntry]);
    expect(callback).toHaveBeenCalledTimes(1);
  });

  it("reuses media queries and delivers modern and legacy change listeners", () => {
    const first = window.matchMedia("(max-width: 999px)");
    const second = window.matchMedia("(max-width: 999px)");
    const modern = vi.fn();
    const legacy = vi.fn();
    const onchange = vi.fn();
    first.addEventListener("change", modern);
    first.addListener(legacy);
    first.onchange = onchange;

    const event = new Event("change");
    first.dispatchEvent(event);

    expect(second).toBe(first);
    expect(modern).toHaveBeenCalledWith(event);
    expect(legacy).toHaveBeenCalledWith(event);
    expect(onchange).toHaveBeenCalledWith(event);
  });
});
