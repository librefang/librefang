import { describe, it, expect, beforeEach } from "vitest";
import { act, render } from "@testing-library/react";
import { DrawerPanel } from "./DrawerPanel";
import { useDrawerStore } from "../../lib/drawerStore";

describe("DrawerPanel", () => {
  beforeEach(() => {
    // Reset the global drawer slot between tests so cross-test state
    // can't leak (the store is a singleton).
    useDrawerStore.setState({ isOpen: false, content: null });
  });

  it("pushes content into the global drawer store while isOpen is true", () => {
    render(
      <DrawerPanel isOpen={true} onClose={() => {}} title="Create agent">
        <p>body</p>
      </DrawerPanel>,
    );
    const state = useDrawerStore.getState();
    expect(state.isOpen).toBe(true);
    expect(state.content?.title).toBe("Create agent");
  });

  // Regression test for #4687: when the parent flips `isOpen` from true →
  // false (e.g. the create-agent mutation `onSuccess` calls
  // `setShowCreate(false)`, or the user clicks a Cancel button bound to
  // the same setter), the drawer must close. Before the fix, only Esc /
  // X / mobile backdrop / unmount could collapse the global slot, so
  // programmatic dismissals silently no-op'd and the form stayed visible
  // with a perpetually spinning submit button.
  it("closes the global drawer store when the parent flips isOpen from true to false", () => {
    const { rerender } = render(
      <DrawerPanel isOpen={true} onClose={() => {}}>
        <p>body</p>
      </DrawerPanel>,
    );
    expect(useDrawerStore.getState().isOpen).toBe(true);

    act(() => {
      rerender(
        <DrawerPanel isOpen={false} onClose={() => {}}>
          <p>body</p>
        </DrawerPanel>,
      );
    });

    expect(useDrawerStore.getState().isOpen).toBe(false);
  });

  // The parent-driven close path must NOT double-fire `onClose`. The
  // existing external-close watcher only invokes `onClose` while the
  // parent still thinks `isOpen=true`; by the time we tear the store
  // down here, `isOpen` is already false, so the watcher stays quiet.
  it("does not invoke onClose when the parent itself initiates the close", () => {
    let calls = 0;
    const onClose = () => {
      calls += 1;
    };
    const { rerender } = render(
      <DrawerPanel isOpen={true} onClose={onClose}>
        <p>body</p>
      </DrawerPanel>,
    );
    act(() => {
      rerender(
        <DrawerPanel isOpen={false} onClose={onClose}>
          <p>body</p>
        </DrawerPanel>,
      );
    });
    expect(calls).toBe(0);
  });

  it("bubbles up an external store close (Esc / X / backdrop) to the parent's onClose", () => {
    let calls = 0;
    const onClose = () => {
      calls += 1;
    };
    render(
      <DrawerPanel isOpen={true} onClose={onClose}>
        <p>body</p>
      </DrawerPanel>,
    );
    expect(useDrawerStore.getState().isOpen).toBe(true);

    act(() => {
      // Simulate the PushDrawer host calling `store.close()` (e.g.
      // the user pressed Escape or clicked the X button).
      useDrawerStore.getState().close();
    });

    expect(calls).toBe(1);
  });
});
