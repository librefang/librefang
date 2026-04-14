import { create } from "zustand";
import { persist } from "zustand/middleware";
import i18n from "./i18n";

interface Toast {
  id: string;
  message: string;
  type: "success" | "error" | "info";
}

interface SkillOutput {
  id: string;
  skillName: string;
  agentId?: string;
  agentName?: string;
  content: string;
  timestamp: number;
}

interface UIState {
  theme: "light" | "dark";
  language: string;
  isMobileMenuOpen: boolean;
  isSidebarCollapsed: boolean;
  navLayout: "grouped" | "collapsible";
  collapsedNavGroups: Record<string, boolean>;
  toasts: Toast[];
  skillOutputs: SkillOutput[];
  hiddenModelKeys: string[];
  deepThinking: boolean;
  showThinkingProcess: boolean;
  setDeepThinking: (value: boolean) => void;
  setShowThinkingProcess: (value: boolean) => void;
  toggleTheme: () => void;
  setLanguage: (lang: string) => void;
  setMobileMenuOpen: (open: boolean) => void;
  toggleSidebar: () => void;
  setNavLayout: (layout: "grouped" | "collapsible") => void;
  toggleNavGroup: (key: string) => void;
  addToast: (message: string, type?: "success" | "error" | "info") => void;
  removeToast: (id: string) => void;
  addSkillOutput: (output: Omit<SkillOutput, "id" | "timestamp">) => void;
  dismissSkillOutput: (id: string) => void;
  clearSkillOutputs: () => void;
  hideModel: (key: string) => void;
  unhideModel: (key: string) => void;
  pruneHiddenKeys: (validKeys: Set<string>) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      theme: (typeof window !== "undefined" && window.matchMedia?.("(prefers-color-scheme: light)").matches) ? "light" : "dark",
      language: i18n.language || "en",
      isMobileMenuOpen: false,
      isSidebarCollapsed: false,
      navLayout: "grouped",
      collapsedNavGroups: {},
      toasts: [],
      skillOutputs: [],
      hiddenModelKeys: [],
      deepThinking: false,
      showThinkingProcess: true,
      setDeepThinking: (value) => set({ deepThinking: value }),
      setShowThinkingProcess: (value) => set({ showThinkingProcess: value }),
      toggleTheme: () =>
        set((state) => ({ theme: state.theme === "light" ? "dark" : "light" })),
      setLanguage: (lang) => {
        void i18n.changeLanguage(lang);
        set({ language: lang });
      },
      setMobileMenuOpen: (open) => set({ isMobileMenuOpen: open }),
      toggleSidebar: () => set((state) => ({ isSidebarCollapsed: !state.isSidebarCollapsed })),
      setNavLayout: (layout) => set({ navLayout: layout }),
      toggleNavGroup: (key) => set((state) => ({ collapsedNavGroups: { ...state.collapsedNavGroups, [key]: !state.collapsedNavGroups[key] } })),
      addToast: (message, type = "info") =>
        set((state) => ({
          toasts: [...state.toasts, { id: Date.now().toString(), message, type }],
        })),
      removeToast: (id) =>
        set((state) => ({
          toasts: state.toasts.filter((t) => t.id !== id),
        })),
      addSkillOutput: (output) =>
        set((state) => ({
          skillOutputs: [
            { ...output, id: Date.now().toString(), timestamp: Date.now() },
            ...state.skillOutputs,
          ].slice(0, 50),
        })),
      dismissSkillOutput: (id) =>
        set((state) => ({
          skillOutputs: state.skillOutputs.filter((o) => o.id !== id),
        })),
      clearSkillOutputs: () => set({ skillOutputs: [] }),
      hideModel: (key) =>
        set((state) => ({
          hiddenModelKeys: state.hiddenModelKeys.includes(key)
            ? state.hiddenModelKeys
            : [...state.hiddenModelKeys, key],
        })),
      unhideModel: (key) =>
        set((state) => ({
          hiddenModelKeys: state.hiddenModelKeys.filter((k) => k !== key),
        })),
      pruneHiddenKeys: (validKeys) =>
        set((state) => ({
          hiddenModelKeys: state.hiddenModelKeys.filter((k) => validKeys.has(k)),
        })),
    }),
    {
      name: "librefang-ui-storage",
      partialize: (state) => ({
        theme: state.theme,
        language: state.language,
        navLayout: state.navLayout,
        hiddenModelKeys: state.hiddenModelKeys,
        deepThinking: state.deepThinking,
        showThinkingProcess: state.showThinkingProcess,
      }),
    }
  )
);
