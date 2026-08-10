import type { ReactNode } from "react";
import { create } from "zustand";

export type DrawerSize = "sm" | "md" | "lg" | "xl" | "2xl" | "3xl" | "4xl" | "5xl";

export interface DrawerContent {
  title?: string;
  size?: DrawerSize;
  hideCloseButton?: boolean;
  body: ReactNode;
  /** Called when the drawer is dismissed via Esc / X / mobile backdrop.
   *  Parents typically use this to flip their own `isOpen` state. */
  onClose?: () => void;
  /** Opaque identity used by DrawerPanel to avoid closing a newer owner. */
  owner?: object;
}

interface DrawerState {
  isOpen: boolean;
  content: DrawerContent | null;
  open: (content: DrawerContent) => void;
  close: (owner?: object) => void;
}

// Single global push-drawer slot. The `<DrawerPanel>` adapter is the primary
// caller — it pushes its `children` here on every render so the slot stays
// in sync with parent state. The `<PushDrawer>` host in App.tsx is a flex
// sibling of the main column, so its width animation pushes the main
// content like the sidebar collapse instead of overlaying it.
//
// Only one drawer can be open at a time. Opening a new one replaces
// whatever was there.
export const useDrawerStore = create<DrawerState>((set) => ({
  isOpen: false,
  content: null,
  open: (content) => set({ isOpen: true, content }),
  close: (owner) => set((state) => {
    if (owner !== undefined && state.content?.owner !== owner) return state;
    return { isOpen: false, content: null };
  }),
}));
