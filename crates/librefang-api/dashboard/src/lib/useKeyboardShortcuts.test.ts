import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CREATE_EVENT, useKeyboardShortcuts } from "./useKeyboardShortcuts";

const navigate = vi.hoisted(() => vi.fn());

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
}));

afterEach(() => {
  navigate.mockReset();
  vi.restoreAllMocks();
});

const press = (key: string): KeyboardEvent => {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  act(() => window.dispatchEvent(event));
  return event;
};

describe("useKeyboardShortcuts", () => {
  it("resolves g+n as Channels navigation instead of create", () => {
    const onCreate = vi.fn();
    window.addEventListener(CREATE_EVENT, onCreate);
    const { unmount } = renderHook(() =>
      useKeyboardShortcuts({ onShowHelp: vi.fn() }),
    );

    press("g");
    const second = press("n");

    expect(second.defaultPrevented).toBe(true);
    expect(navigate).toHaveBeenCalledWith({ to: "/channels" });
    expect(onCreate).not.toHaveBeenCalled();

    unmount();
    window.removeEventListener(CREATE_EVENT, onCreate);
  });

  it("still dispatches create for a standalone n", () => {
    const onCreate = vi.fn();
    window.addEventListener(CREATE_EVENT, onCreate);
    const { unmount } = renderHook(() =>
      useKeyboardShortcuts({ onShowHelp: vi.fn() }),
    );

    const event = press("n");
    expect(event.defaultPrevented).toBe(true);
    expect(onCreate).toHaveBeenCalledOnce();
    expect(navigate).not.toHaveBeenCalled();

    unmount();
    window.removeEventListener(CREATE_EVENT, onCreate);
  });
});
