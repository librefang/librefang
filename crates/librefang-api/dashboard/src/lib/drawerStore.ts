import type { ReactNode } from "react";
import { create } from "zustand";

export type DrawerSize = "sm" | "md" | "lg" | "xl";

export interface DrawerContent {
  title?: string;
  size?: DrawerSize;
  body: ReactNode;
}

interface DrawerState {
  isOpen: boolean;
  content: DrawerContent | null;
  openDrawer: (content: DrawerContent) => void;
  closeDrawer: () => void;
}

// Single global push-drawer slot. Page components call openDrawer with the
// body to show; the slot in App.tsx is a flex sibling of the main column,
// so its width animation pushes the main content like the sidebar collapse
// instead of overlaying it. Only one drawer can be open at a time — opening
// a new one replaces whatever was there. See PushDrawer.tsx for the host.
export const useDrawerStore = create<DrawerState>((set) => ({
  isOpen: false,
  content: null,
  openDrawer: (content) => set({ isOpen: true, content }),
  closeDrawer: () => set({ isOpen: false }),
}));
