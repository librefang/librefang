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

  // Regression test for #4714: the picker → config flow.
  //
  // ProvidersPage and ChannelsPage both use a pattern where clicking an
  // item in an "Add" picker drawer closes the picker and opens a
  // configuration drawer in the same React commit:
  //
  //     handlePick = (item) => {
  //       setPickerOpen(false);     // → picker DrawerPanel isOpen=true → false
  //       setConfiguringItem(item); // → mounts config DrawerPanel (isOpen=true)
  //     };
  //
  // Before the ownership check, the picker's parent-driven close watcher
  // would unconditionally call `store.close()` after the config drawer
  // had already pushed its own body into the slot. The slot then went
  // `{isOpen=false, content=config_body}`, the existing external-close
  // watcher saw `!drawerOpen && wasOpen` for the config drawer and fired
  // ITS `onClose`, which made the parent unmount the config drawer.
  // Net visible effect: clicking a picker item closed the picker AND
  // immediately closed the configuration window the user was trying
  // to reach. See https://github.com/librefang/librefang/issues/4714.
  //
  // Fix: each DrawerPanel only fires `close()` when the slot's content
  // is still the body it pushed. A freshly-mounted DrawerPanel that
  // pushed in the same commit "wins" the slot.
  it("does not collateral-close the freshly-mounted drawer when a sibling drawer's parent flips isOpen=false in the same commit", () => {
    // Step 1: picker drawer is open and owns the slot.
    let pickerOnCloseCalls = 0;
    const PickerOnClose = () => {
      pickerOnCloseCalls += 1;
    };
    let configOnCloseCalls = 0;
    const ConfigOnClose = () => {
      configOnCloseCalls += 1;
    };

    function Harness({
      pickerOpen,
      configMounted,
    }: {
      pickerOpen: boolean;
      configMounted: boolean;
    }) {
      return (
        <>
          {/* Conditionally-mounted config drawer with isOpen literal true,
              same shape as ChannelsPage::ConfigDialog. */}
          {configMounted && (
            <DrawerPanel isOpen onClose={ConfigOnClose} title="config">
              <p data-testid="config-body">config body</p>
            </DrawerPanel>
          )}
          {/* Always-mounted picker drawer with toggling isOpen, same shape
              as ChannelsPage's Add picker. */}
          <DrawerPanel isOpen={pickerOpen} onClose={PickerOnClose} title="picker">
            <p data-testid="picker-body">picker body</p>
          </DrawerPanel>
        </>
      );
    }

    const { rerender } = render(<Harness pickerOpen={true} configMounted={false} />);
    expect(useDrawerStore.getState().isOpen).toBe(true);
    expect(useDrawerStore.getState().content?.title).toBe("picker");

    // Step 2: simulate the picker's onClick handler running:
    //   setPickerOpen(false);     // picker DrawerPanel isOpen → false
    //   setConfiguringItem(item); // mounts config DrawerPanel
    act(() => {
      rerender(<Harness pickerOpen={false} configMounted={true} />);
    });

    // Slot must hold the config drawer, not be closed by the picker's
    // parent-driven close watcher.
    expect(useDrawerStore.getState().isOpen).toBe(true);
    expect(useDrawerStore.getState().content?.title).toBe("config");

    // The config drawer's onClose must NOT have fired — the user
    // didn't dismiss it.
    expect(configOnCloseCalls).toBe(0);

    // The picker's onClose must NOT have fired either — its parent
    // flipped `isOpen=false` itself, so the parent-driven close path
    // applies (silent), not the external-close bubble-up.
    expect(pickerOnCloseCalls).toBe(0);
  });
});
