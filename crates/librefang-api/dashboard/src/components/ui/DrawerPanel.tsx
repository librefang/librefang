import { useEffect, useRef, type ReactNode } from "react";
import { useDrawerStore, type DrawerSize } from "../../lib/drawerStore";

export interface DrawerPanelProps {
  isOpen: boolean;
  onClose: () => void;
  title?: string;
  /** Width cap on lg+. Defaults to "md". */
  size?: DrawerSize;
  /** Hide the default header X button (e.g. when the body supplies its own). */
  hideCloseButton?: boolean;
  children: ReactNode;
}

// Drop-in replacement for `<Modal variant="panel-right">`. Pushes its
// `children` into the global `<PushDrawer>` slot in App.tsx instead of
// rendering as a fixed overlay, so the main content adapts (like the
// sidebar collapse) instead of being covered.
//
// Sync model:
//   1. While `isOpen`, push content (incl. children) to the store on every
//      render — keeps the slot in sync with parent state changes.
//   2. PushDrawer dismissals (Esc, X, mobile backdrop) call `store.close()`.
//      The watcher below detects the external close while we still think
//      `isOpen=true` and calls `props.onClose()` so the parent flips its
//      own state. Single source of close-callback firing.
//   3. Parent-driven close: when the parent flips `isOpen` from true →
//      false (mutation `onSuccess`, Cancel button, etc.) we tear the
//      store back down ourselves; otherwise the global slot stays
//      mounted and the drawer never disappears (#4687).
//   4. Unmount while open → close the store, so a body referencing this
//      page's local state never lingers in the global slot.
export function DrawerPanel({
  isOpen,
  onClose,
  title,
  size = "md",
  hideCloseButton,
  children,
}: DrawerPanelProps) {
  const open = useDrawerStore((s) => s.open);
  const close = useDrawerStore((s) => s.close);
  const drawerOpen = useDrawerStore((s) => s.isOpen);

  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  const isOpenRef = useRef(isOpen);
  useEffect(() => {
    isOpenRef.current = isOpen;
  }, [isOpen]);

  // Push children into the slot whenever we're open. Re-runs on every
  // re-render that changes any of the deps — including `children`, which
  // gets a fresh identity each render. That's intended: the body should
  // mirror the parent's current state.
  useEffect(() => {
    if (!isOpen) return;
    open({
      title,
      size,
      hideCloseButton,
      body: children,
      onClose: () => onCloseRef.current(),
    });
  }, [isOpen, title, size, hideCloseButton, children, open]);

  // Parent-driven close: when the parent flips `isOpen` from true → false
  // (e.g. after a successful mutation in `onSuccess`, or when the Cancel
  // button calls `setOpen(false)`), tear the store back down so the
  // global drawer slot collapses. Without this, only Esc / X / mobile
  // backdrop / unmount could close the drawer — programmatic
  // dismissals from the parent silently no-op'd, leaving the form
  // visible with a perpetually spinning submit button (#4687).
  //
  // The watcher above guards on `isOpen && wasOpen && !drawerOpen` and
  // therefore won't double-fire `onClose` on this path: by the time the
  // store flip lands, `isOpen` is already false.
  const prevIsOpenRef = useRef(isOpen);
  useEffect(() => {
    const wasOpen = prevIsOpenRef.current;
    prevIsOpenRef.current = isOpen;
    if (wasOpen && !isOpen && drawerOpen) {
      close();
    }
  }, [isOpen, drawerOpen, close]);

  // External close → bubble up to the parent so it can flip its state.
  // Only fires on a real `true → false` transition. On first mount the
  // store is still false (effect order: ref-update → push-to-store →
  // this watcher), and treating that initial false as "the store was
  // closed externally" would call parent.onClose() before the drawer
  // ever rendered. This was the "drawer won't open" bug on pages that
  // mount DrawerPanel conditionally with `isOpen` hard-coded to true
  // (e.g. HandDetailPanel).
  const prevDrawerOpenRef = useRef(false);
  useEffect(() => {
    const wasOpen = prevDrawerOpenRef.current;
    prevDrawerOpenRef.current = drawerOpen;
    if (isOpen && wasOpen && !drawerOpen) {
      onCloseRef.current();
    }
  }, [drawerOpen, isOpen]);

  // Cleanup on unmount.
  useEffect(
    () => () => {
      if (isOpenRef.current) close();
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  return null;
}
