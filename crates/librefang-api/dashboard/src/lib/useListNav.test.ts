import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useListNav } from "./useListNav";

function fireKey(key: string, opts: { shift?: boolean } = {}) {
  const event = new KeyboardEvent("keydown", {
    key,
    shiftKey: !!opts.shift,
    bubbles: true,
    cancelable: true,
  });
  window.dispatchEvent(event);
  return event;
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

  it("does not suppress Enter on a native interactive element outside the list", () => {
    const button = document.createElement("button");
    document.body.appendChild(button);
    button.focus();
    const onActivate = vi.fn();
    renderHook(() => useListNav({ items: ["a"], onActivate }));

    const event = fireKey("Enter");
    expect(event.defaultPrevented).toBe(false);
    expect(onActivate).not.toHaveBeenCalled();
    button.remove();
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

  it("keeps the global listener stable while selection changes", () => {
    const add = vi.spyOn(window, "addEventListener");
    const remove = vi.spyOn(window, "removeEventListener");
    const { unmount } = renderHook(() => useListNav({ items: ["a", "b"] }));
    const keydownAdds = () => add.mock.calls.filter(([type]) => type === "keydown");
    const keydownRemovals = () => remove.mock.calls.filter(([type]) => type === "keydown");

    expect(keydownAdds()).toHaveLength(1);
    act(() => fireKey("j"));
    act(() => fireKey("j"));
    expect(keydownAdds()).toHaveLength(1);
    expect(keydownRemovals()).toHaveLength(0);
    unmount();
    expect(keydownRemovals()).toHaveLength(1);
    vi.restoreAllMocks();
  });

  it("uses stable item handlers and keeps clicks selection-only", () => {
    const onActivate = vi.fn();
    const { result } = renderHook(() =>
      useListNav({ items: ["a", "b"], onActivate }),
    );
    const getItemProps = result.current.getItemProps;
    const before = getItemProps(1);

    act(() => before.onClick());
    const after = result.current.getItemProps(1);
    expect(result.current.getItemProps).toBe(getItemProps);
    expect(after.ref).toBe(before.ref);
    expect(after.onMouseEnter).toBe(before.onMouseEnter);
    expect(after.onClick).toBe(before.onClick);
    expect(result.current.selectedIndex).toBe(1);
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("scrolls selected rows immediately to the nearest edge", () => {
    const { result } = renderHook(() => useListNav({ items: ["a"] }));
    const element = document.createElement("button");
    element.dataset.listnavIndex = "0";
    element.scrollIntoView = vi.fn();
    result.current.getItemProps(0).ref(element);

    act(() => result.current.setSelectedIndex(0));
    expect(element.scrollIntoView).toHaveBeenCalledWith({
      block: "nearest",
      behavior: "auto",
    });
  });
});
