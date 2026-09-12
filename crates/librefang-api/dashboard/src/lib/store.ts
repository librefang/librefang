import { create } from "zustand";
import { persist } from "zustand/middleware";
import i18n from "./i18n";

export const UI_STORE_VERSION = 1;
export const MAX_TOASTS = 50;
export const MAX_SKILL_OUTPUTS = 50;
/**
 * Tabs kept per agent.
 *
 * A cap rather than none: every session ever visited would accumulate into a
 * strip too wide to use, and the oldest tab is the one least likely to be
 * wanted. Closing is still explicit — this only bounds the automatic opening
 * that happens on every visit.
 */
export const MAX_CHAT_TABS = 12;

export function createClientId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }

  return `${Date.now()}-${Math.random().toString(36).slice(2, 11)}`;
}

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
  terminalEnabled: boolean | null;
  modelsAvailableOnly: boolean;
  deepThinking: boolean;
  showThinkingProcess: boolean;
  /**
   * Scale applied to the chat transcript, as a multiplier.
   *
   * Readable type is a per-person, per-display setting — the size that is
   * comfortable on a 27" panel wastes a 13" one — so this is a live control
   * rather than a default someone picked once. Kept as a number so the steps
   * are the UI's business and a value from an older build still applies.
   */
  chatScale: number;
  setChatScale: (value: number) => void;
  /**
   * Sessions kept open as tabs, per agent, oldest first.
   *
   * A UI list, not server state: a session exists whether or not it has a tab,
   * and closing a tab must never delete one. It is persisted so a reload does
   * not silently drop the four conversations someone had going.
   */
  openChatTabs: Record<string, string[]>;
  openChatTab: (agentId: string, sessionId: string) => void;
  closeChatTab: (agentId: string, sessionId: string) => void;
  pruneChatTabs: (agentId: string, validSessionIds: Set<string>) => void;
  setModelsAvailableOnly: (value: boolean) => void;
  setDeepThinking: (value: boolean) => void;
  setShowThinkingProcess: (value: boolean) => void;
  toggleTheme: () => void;
  setLanguage: (lang: string) => Promise<boolean>;
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
  pruneCollapsedNavGroups: (validKeys: Set<string>) => void;
  setTerminalEnabled: (enabled: boolean) => void;
}

type PersistedUIState = Pick<
  UIState,
  | "theme"
  | "language"
  | "isSidebarCollapsed"
  | "navLayout"
  | "collapsedNavGroups"
  | "hiddenModelKeys"
  | "modelsAvailableOnly"
  | "deepThinking"
  | "showThinkingProcess"
  | "chatScale"
  | "openChatTabs"
>;

/**
 * Scale bounds for the chat transcript.
 *
 * Clamped rather than free: a persisted `0` would collapse the transcript to
 * nothing with no visible control left to recover it, and localStorage is
 * user-editable, so the value is treated as untrusted on the way in.
 */
export const MIN_CHAT_SCALE = 0.75;
export const MAX_CHAT_SCALE = 1.25;
export const DEFAULT_CHAT_SCALE = 0.9;
/** One press of the smaller/larger control. */
export const CHAT_SCALE_STEP = 0.05;

export function clampChatScale(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_CHAT_SCALE;
  return Math.min(MAX_CHAT_SCALE, Math.max(MIN_CHAT_SCALE, value));
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export function migratePersistedUIState(
  persistedState: unknown,
): PersistedUIState {
  const migrated: PersistedUIState = {
    theme: "dark",
    language: i18n.language || "en",
    isSidebarCollapsed: false,
    navLayout: "grouped",
    collapsedNavGroups: {},
    hiddenModelKeys: [],
    modelsAvailableOnly: true,
    deepThinking: false,
    showThinkingProcess: true,
    chatScale: DEFAULT_CHAT_SCALE,
    openChatTabs: {},
  };
  if (!isRecord(persistedState)) return migrated;

  if (persistedState.theme === "light" || persistedState.theme === "dark") {
    migrated.theme = persistedState.theme;
  }
  if (typeof persistedState.language === "string" && persistedState.language.trim()) {
    migrated.language = persistedState.language;
  }
  if (typeof persistedState.isSidebarCollapsed === "boolean") {
    migrated.isSidebarCollapsed = persistedState.isSidebarCollapsed;
  }
  if (
    persistedState.navLayout === "grouped" ||
    persistedState.navLayout === "collapsible"
  ) {
    migrated.navLayout = persistedState.navLayout;
  }
  if (isRecord(persistedState.collapsedNavGroups)) {
    migrated.collapsedNavGroups = Object.fromEntries(
      Object.entries(persistedState.collapsedNavGroups).filter(
        (entry): entry is [string, boolean] => typeof entry[1] === "boolean",
      ),
    );
  }
  if (Array.isArray(persistedState.hiddenModelKeys)) {
    migrated.hiddenModelKeys = persistedState.hiddenModelKeys.filter(
      (key): key is string => typeof key === "string",
    );
  }
  for (const key of [
    "modelsAvailableOnly",
    "deepThinking",
    "showThinkingProcess",
  ] as const) {
    if (typeof persistedState[key] === "boolean") {
      migrated[key] = persistedState[key];
    }
  }
  if (persistedState.chatScale !== undefined) {
    migrated.chatScale = clampChatScale(persistedState.chatScale);
  }

  if (isRecord(persistedState.openChatTabs)) {
    // localStorage is user-editable and survives across versions, so the shape
    // is checked rather than trusted: a non-array or a list with non-strings
    // in it would reach `.map` in the tab strip as `undefined` keys.
    migrated.openChatTabs = Object.fromEntries(
      Object.entries(persistedState.openChatTabs)
        .map(([agentId, ids]) => [
          agentId,
          Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : [],
        ])
        .filter(([, ids]) => (ids as string[]).length > 0),
    ) as Record<string, string[]>;
  }

  return migrated;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      theme: "dark",
      language: i18n.language || "en",
      isMobileMenuOpen: false,
      isSidebarCollapsed: false,
      navLayout: "grouped",
      collapsedNavGroups: {},
      toasts: [],
      skillOutputs: [],
      hiddenModelKeys: [],
      terminalEnabled: null,
      modelsAvailableOnly: true,
      deepThinking: false,
      showThinkingProcess: true,
      chatScale: DEFAULT_CHAT_SCALE,
      setChatScale: (value) => set({ chatScale: clampChatScale(value) }),
      openChatTabs: {},
      openChatTab: (agentId, sessionId) =>
        set((state) => {
          const current = state.openChatTabs[agentId] ?? [];
          if (current.includes(sessionId)) return state;
          const next = [...current, sessionId].slice(-MAX_CHAT_TABS);
          return { openChatTabs: { ...state.openChatTabs, [agentId]: next } };
        }),
      closeChatTab: (agentId, sessionId) =>
        set((state) => {
          const next = (state.openChatTabs[agentId] ?? []).filter((id) => id !== sessionId);
          const tabs = { ...state.openChatTabs };
          // Drop the agent's entry entirely when its last tab goes, so the
          // persisted object does not accumulate empty arrays for every agent
          // ever opened.
          if (next.length === 0) delete tabs[agentId];
          else tabs[agentId] = next;
          return { openChatTabs: tabs };
        }),
      pruneChatTabs: (agentId, validSessionIds) =>
        set((state) => {
          const current = state.openChatTabs[agentId];
          if (!current) return state;
          const next = current.filter((id) => validSessionIds.has(id));
          if (next.length === current.length) return state;
          const tabs = { ...state.openChatTabs };
          if (next.length === 0) delete tabs[agentId];
          else tabs[agentId] = next;
          return { openChatTabs: tabs };
        }),
      setModelsAvailableOnly: (value) => set({ modelsAvailableOnly: value }),
      setDeepThinking: (value) => set({ deepThinking: value }),
      setShowThinkingProcess: (value) => set({ showThinkingProcess: value }),
      toggleTheme: () =>
        set((state) => ({ theme: state.theme === "light" ? "dark" : "light" })),
      setLanguage: async (lang) => {
        try {
          await i18n.changeLanguage(lang);
          set({ language: lang });
          return true;
        } catch (err) {
          console.error("Failed to change language:", err);
          return false;
        }
      },
      setMobileMenuOpen: (open) => set({ isMobileMenuOpen: open }),
      toggleSidebar: () => set((state) => ({ isSidebarCollapsed: !state.isSidebarCollapsed })),
      setNavLayout: (layout) => set({ navLayout: layout }),
      toggleNavGroup: (key) => set((state) => ({ collapsedNavGroups: { ...state.collapsedNavGroups, [key]: !state.collapsedNavGroups[key] } })),
      addToast: (message, type = "info") =>
        set((state) => {
          const next = [...state.toasts, { id: createClientId(), message, type }];
          return {
            toasts: next.length > MAX_TOASTS ? next.slice(-MAX_TOASTS) : next,
          };
        }),
      removeToast: (id) =>
        set((state) => ({
          toasts: state.toasts.filter((t) => t.id !== id),
        })),
      addSkillOutput: (output) =>
        set((state) => ({
          skillOutputs: [
            { ...output, id: createClientId(), timestamp: Date.now() },
            ...state.skillOutputs,
          ].slice(0, MAX_SKILL_OUTPUTS),
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
      pruneCollapsedNavGroups: (validKeys) =>
        set((state) => ({
          collapsedNavGroups: Object.fromEntries(
            Object.entries(state.collapsedNavGroups).filter(([key]) =>
              validKeys.has(key),
            ),
          ),
        })),
      setTerminalEnabled: (enabled) => set({ terminalEnabled: enabled }),
    }),
    {
      name: "librefang-ui-storage",
      version: UI_STORE_VERSION,
      migrate: (persistedState) => migratePersistedUIState(persistedState),
      onRehydrateStorage: () => (state) => {
        if (!state?.language) return;
        void i18n.changeLanguage(state.language).catch((err) => {
          console.error("Failed to restore persisted language:", err);
        });
      },
      partialize: (state) => ({
        theme: state.theme,
        language: state.language,
        isSidebarCollapsed: state.isSidebarCollapsed,
        navLayout: state.navLayout,
        collapsedNavGroups: state.collapsedNavGroups,
        hiddenModelKeys: state.hiddenModelKeys,
        modelsAvailableOnly: state.modelsAvailableOnly,
        deepThinking: state.deepThinking,
        showThinkingProcess: state.showThinkingProcess,
        chatScale: state.chatScale,
        openChatTabs: state.openChatTabs,
      }),
    }
  )
);
