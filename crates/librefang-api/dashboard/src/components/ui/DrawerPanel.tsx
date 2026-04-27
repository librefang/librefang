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
//   3. Unmount while open → close the store, so a body referencing this
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

  // External close → bubble up to the parent so it can flip its state.
  useEffect(() => {
    if (isOpen && !drawerOpen) {
      onCloseRef.current();
    }
  }, [drawerOpen, isOpen]);

  // Cleanup on unmount.
  useEffect(
    () => () => {
      if (isOpen) close();
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  return null;
}
