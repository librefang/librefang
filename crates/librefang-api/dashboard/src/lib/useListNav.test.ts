import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useListNav } from "./useListNav";

function fireKey(key: string, opts: { shift?: boolean } = {}) {
  window.dispatchEvent(
    new KeyboardEvent("keydown", { key, shiftKey: !!opts.shift, bubbles: true }),
  );
}

beforeEach(() => {
  // Make sure each test starts with focus on body so isTypingTarget=false.
  document.body.focus();
});

describe("useListNav", () => {
  it("starts with no selection", () => {
    const { result } = renderHook(() => useListNav({ items: ["a", "b", "c"] }));
    expect(result.current.selectedIndex).toBe(-1);
  });

  it("j/ArrowDown advances; k/ArrowUp retreats; first j selects index 0", () => {
    const { result } = renderHook(() => useListNav({ items: ["a", "b", "c"] }));
    act(() => fireKey("j"));
    expect(result.current.selectedIndex).toBe(0);
    act(() => fireKey("ArrowDown"));
    expect(result.current.selectedIndex).toBe(1);
    act(() => fireKey("k"));
    expect(result.current.selectedIndex).toBe(0);
    // Clamp at top.
    act(() => fireKey("ArrowUp"));
    expect(result.current.selectedIndex).toBe(0);
  });

  it("j clamps at the bottom", () => {
    const { result } = renderHook(() => useListNav({ items: ["a", "b"] }));
    act(() => fireKey("j"));
    act(() => fireKey("j"));
    act(() => fireKey("j"));
    expect(result.current.selectedIndex).toBe(1);
  });

  it("Shift+G jumps to bottom", () => {
    const { result } = renderHook(() => useListNav({ items: ["a", "b", "c", "d"] }));
    act(() => fireKey("G", { shift: true }));
    expect(result.current.selectedIndex).toBe(3);
  });

  it("gg jumps to top within the 1500ms window", () => {
    const { result } = renderHook(() => useListNav({ items: ["a", "b", "c"] }));
    act(() => fireKey("G", { shift: true })); // jump to bottom first
    expect(result.current.selectedIndex).toBe(2);
    act(() => {
      fireKey("g");
      fireKey("g");
    });
    expect(result.current.selectedIndex).toBe(0);
  });

  it("Enter calls onActivate with the selected item", () => {
    const onActivate = vi.fn();
    const { result } = renderHook(() =>
      useListNav({ items: [{ id: "a" }, { id: "b" }], onActivate }),
    );
    act(() => fireKey("j"));
    act(() => fireKey("j"));
    expect(result.current.selectedIndex).toBe(1);
    act(() => fireKey("Enter"));
    expect(onActivate).toHaveBeenCalledWith({ id: "b" }, 1);
  });

  it("Enter falls back to index 0 when nothing is selected", () => {
    const onActivate = vi.fn();
    renderHook(() => useListNav({ items: [{ id: "x" }], onActivate }));
    act(() => fireKey("Enter"));
    expect(onActivate).toHaveBeenCalledWith({ id: "x" }, 0);
  });

  it("Escape clears selection and invokes onEscape", () => {
    const onEscape = vi.fn();
    const { result } = renderHook(() => useListNav({ items: ["a", "b"], onEscape }));
    act(() => fireKey("j"));
    expect(result.current.selectedIndex).toBe(0);
    act(() => fireKey("Escape"));
    expect(result.current.selectedIndex).toBe(-1);
    expect(onEscape).toHaveBeenCalled();
  });

  it("ignores keys while typing into an input", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    const { result } = renderHook(() => useListNav({ items: ["a", "b"] }));
    // Dispatch on the input, not window — the listener still fires on window
    // but reads e.target. We dispatch via the input.
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "j", bubbles: true }));
    expect(result.current.selectedIndex).toBe(-1);
    document.body.removeChild(input);
  });

  it("clamps selection back when items shrink", () => {
    const { result, rerender } = renderHook(
      ({ items }: { items: string[] }) => useListNav({ items }),
      { initialProps: { items: ["a", "b", "c", "d"] } },
    );
    act(() => fireKey("G", { shift: true }));
    expect(result.current.selectedIndex).toBe(3);
    rerender({ items: ["a", "b"] });
    expect(result.current.selectedIndex).toBe(1);
  });

  it("does nothing when disabled", () => {
    const { result } = renderHook(() =>
      useListNav({ items: ["a", "b"], disabled: true }),
    );
    act(() => fireKey("j"));
    expect(result.current.selectedIndex).toBe(-1);
  });
});
