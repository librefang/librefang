import { create } from "zustand";
import { persist } from "zustand/middleware";

interface UIState {
  theme: "light" | "dark";
  isMobileMenuOpen: boolean;
  toggleTheme: () => void;
  setMobileMenuOpen: (open: boolean) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      theme: "dark",
      isMobileMenuOpen: false,
      toggleTheme: () =>
        set((state) => ({ theme: state.theme === "light" ? "dark" : "light" })),
      setMobileMenuOpen: (open) => set({ isMobileMenuOpen: open }),
    }),
    {
      name: "librefang-ui-storage",
      partialize: (state) => ({ theme: state.theme }), // Only persist theme
    }
  )
);
