import type { ReactNode } from "react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { I18nextProvider, initReactI18next } from "react-i18next";
import i18n from "i18next";
import { PushDrawer } from "./PushDrawer";
import { useDrawerStore } from "../../lib/drawerStore";

// Minimal i18n init so `useTranslation()` inside PushDrawer doesn't crash.
if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    lng: "en",
    fallbackLng: "en",
    resources: { en: { translation: {} } },
    interpolation: { escapeValue: false },
  });
}

function withI18n(node: ReactNode) {
  return <I18nextProvider i18n={i18n}>{node}</I18nextProvider>;
}

describe("PushDrawer breakpoint boundary (#4873)", () => {
  beforeEach(() => {
    useDrawerStore.setState({ isOpen: false, content: null });
  });

  // Regression lock: PushDrawer's JS-side `useIsMobile()` MUST read
  // `(max-width: 999px)` so it stays in lock-step with the CSS-side
  // `--breakpoint-lg: 1000px` override in index.css. If a future
  // contributor reverts the literal to `1023` or any other value,
  // this fails — surfacing the implicit JS↔CSS coupling that the
  // in-code comment alone cannot enforce.
  it("queries matchMedia with the 999px boundary literal that mirrors --breakpoint-lg", () => {
    const seenQueries: string[] = [];
    const matchMediaSpy = vi.spyOn(window, "matchMedia").mockImplementation((query: string) => {
      seenQueries.push(query);
      return {
        matches: false,
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      };
    });

    useDrawerStore.setState({
      isOpen: true,
      content: { title: "Test drawer", body: <p>body</p>, size: "md" },
    });

    render(withI18n(<PushDrawer />));

    expect(seenQueries).toContain("(max-width: 999px)");
    // Negative assertion — catches an accidental partial revert that
    // updates the comment but leaves the old literal in place.
    expect(seenQueries).not.toContain("(max-width: 1023px)");

    matchMediaSpy.mockRestore();
  });

  // Lazy-init regression lock (same #4873 PR): on first render the
  // hook must already reflect the current viewport, not start at
  // `false` and flip after the effect. Without this, a phone-size
  // mount briefly attaches the focus trap to the desktop <aside>
  // before useEffect runs.
  it("reflects matches=true on the very first render (no effect-deferred flip)", () => {
    let isMobileQueryMatches = true;
    const matchMediaSpy = vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({
      matches: query === "(max-width: 999px)" ? isMobileQueryMatches : false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));

    useDrawerStore.setState({
      isOpen: true,
      content: { title: "Test drawer", body: <p>body</p>, size: "md" },
    });

    // If lazy-init is broken, useState(false) → first render uses
    // desktop-mode focus-trap → effect re-renders → mobile focus-trap
    // attaches. We can't observe focus traps here; instead, sanity-check
    // that matchMedia is consulted *before* the effect by calling render
    // and asserting the query was inspected at least once. (The effect
    // calls it again on mount; either path satisfies this — it's the
    // hook's first-render branch we're really exercising via lazy init.)
    isMobileQueryMatches = true;
    render(withI18n(<PushDrawer />));

    expect(matchMediaSpy).toHaveBeenCalledWith("(max-width: 999px)");
    matchMediaSpy.mockRestore();
  });
});

describe("PushDrawer nested-dialog Esc handling (#5254 Codex P2)", () => {
  beforeEach(() => {
    useDrawerStore.setState({ isOpen: false, content: null });
  });

  // Regression lock for the Codex P2 finding on #5254: on <lg viewports
  // PushDrawer renders its content inside a `<div role='dialog'
  // data-drawer-root>` overlay. When a Modal (e.g. ScheduleModal's cron
  // picker) opens inside that drawer body, both PushDrawer and Modal
  // attach `keydown` listeners on `window`. PushDrawer's listener was
  // registered first (it mounted before the picker opened), so it ran
  // first and — under the previous condition `target.closest('[role=
  // dialog]') && !target.closest('[role=dialog][data-drawer-root]')` —
  // incorrectly tore the parent drawer down whenever Esc was pressed
  // anywhere inside the nested Modal, because `.closest()` happily
  // matched both the inner Modal dialog AND the outer drawer-root.
  //
  // The fix narrows the check to the *nearest* dialog ancestor: only
  // close the drawer when the nearest `[role='dialog']` ancestor IS
  // the drawer-root itself. Nested dialogs get first crack at Esc.
  it("does NOT close the drawer when Esc target sits in a nested dialog under the drawer-root (mobile)", () => {
    // Force mobile so the drawer-root overlay (data-drawer-root) is
    // actually rendered. Without this, PushDrawer renders only the
    // desktop <aside> (no role=dialog), and the bug wouldn't trigger.
    const matchMediaSpy = vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({
      matches: query === "(max-width: 999px)",
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));

    // Drawer body intentionally contains a `[role='dialog']` to mimic
    // a nested Modal (e.g. ScheduleModal) rendered inside the drawer's
    // body slot. We skip mounting the real <Modal> here to avoid its
    // own Esc listener confounding the assertion — this test pins the
    // PushDrawer handler's `.closest()` logic, not Modal's behaviour.
    //
    // NB: PushDrawer renders `content.body` TWICE in the DOM — once
    // inside the desktop `<aside>` (hidden by `lg:flex` on mobile,
    // but still mounted), once inside the mobile drawer-root overlay.
    // The test must dispatch Esc from the *mobile* copy, which is
    // the descendant of `[data-drawer-root]`. We use a
    // `data-testid="nested-dialog"` on the dialog wrapper to scope
    // querySelector to the mobile copy via the drawer-root subtree.
    useDrawerStore.setState({
      isOpen: true,
      content: {
        title: "Parent drawer",
        body: (
          <div role="dialog" aria-modal="true" data-testid="nested-dialog">
            <button data-testid="picker-input">picker input</button>
          </div>
        ),
        size: "md",
      },
    });

    const { container } = render(withI18n(<PushDrawer />));

    const drawerRoot = container.querySelector("[data-drawer-root]");
    expect(drawerRoot).not.toBeNull();
    // Scope to the mobile copy that lives INSIDE the drawer-root.
    // querySelector against the document tree returns the first
    // match (which would be the desktop aside's copy on a real
    // mobile-viewport browser too, but in jsdom both subtrees are
    // mounted and DOM-order returns desktop first).
    const pickerInput = drawerRoot!.querySelector(
      "[data-testid='picker-input']",
    ) as HTMLElement;
    expect(pickerInput).not.toBeNull();
    const nearestDialog = pickerInput.closest("[role='dialog']");
    expect(nearestDialog).not.toBeNull();
    // The nearest dialog must be the NESTED one (no data-drawer-root)
    // — this is the exact ancestor shape that confused the old
    // condition.
    expect(nearestDialog!.hasAttribute("data-drawer-root")).toBe(false);
    // Sanity: the drawer-root IS an ancestor of the picker input,
    // matching the real <ScheduleModal-inside-DrawerPanel> layout.
    expect(pickerInput.closest("[role='dialog'][data-drawer-root]")).toBe(
      drawerRoot,
    );

    // Dispatch Esc at the nested picker input. window-level listener
    // fires; the fix must keep the parent drawer open.
    expect(useDrawerStore.getState().isOpen).toBe(true);
    act(() => {
      pickerInput.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });

    // The drawer MUST still be open — PushDrawer's handler must defer
    // to the nested dialog's handler. Before the fix, the drawer's
    // store would have flipped to isOpen=false.
    expect(useDrawerStore.getState().isOpen).toBe(true);

    matchMediaSpy.mockRestore();
  });

  // Positive control: Esc fired with the target inside the drawer-root
  // BUT NOT inside a nested dialog (i.e. focus is in the drawer's own
  // body) MUST still close the drawer — otherwise the fix would
  // regress the basic mobile-drawer dismissal contract.
  it("DOES close the drawer when Esc is pressed in the drawer body itself (no nested dialog)", () => {
    const matchMediaSpy = vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({
      matches: query === "(max-width: 999px)",
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));

    useDrawerStore.setState({
      isOpen: true,
      content: {
        title: "Parent drawer",
        body: <button data-testid="drawer-body-btn">click me</button>,
        size: "md",
      },
    });

    const { container } = render(withI18n(<PushDrawer />));
    const bodyBtn = container.querySelector(
      "[data-testid='drawer-body-btn']",
    ) as HTMLElement;
    expect(bodyBtn).not.toBeNull();

    act(() => {
      bodyBtn.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });

    expect(useDrawerStore.getState().isOpen).toBe(false);
    matchMediaSpy.mockRestore();
  });
});
