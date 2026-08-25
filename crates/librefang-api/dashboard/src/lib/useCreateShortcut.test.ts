import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CREATE_EVENT } from "./useKeyboardShortcuts";
import { useCreateShortcut } from "./useCreateShortcut";

afterEach(() => vi.restoreAllMocks());

describe("useCreateShortcut", () => {
  it("subscribes once across rerenders and invokes the latest handler", () => {
    const first = vi.fn();
    const second = vi.fn();
    const addEventListener = vi.spyOn(window, "addEventListener");
    const removeEventListener = vi.spyOn(window, "removeEventListener");
    const { rerender, unmount } = renderHook(
      ({ handler }) => useCreateShortcut(handler),
      { initialProps: { handler: first } },
    );

    const createAdds = () =>
      addEventListener.mock.calls.filter(([type]) => type === CREATE_EVENT);
    const createRemovals = () =>
      removeEventListener.mock.calls.filter(([type]) => type === CREATE_EVENT);

    expect(createAdds()).toHaveLength(1);
    rerender({ handler: second });
    expect(createAdds()).toHaveLength(1);
    expect(createRemovals()).toHaveLength(0);

    act(() => window.dispatchEvent(new Event(CREATE_EVENT)));
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledOnce();

    unmount();
    expect(createRemovals()).toHaveLength(1);
    expect(createRemovals()[0][1]).toBe(createAdds()[0][1]);
  });
});
