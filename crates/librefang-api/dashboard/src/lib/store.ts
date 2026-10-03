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
 * strip too wide to use. `openChatTabs` preserves the order the user sees — a
 * revisited session stays where it is — and `chatTabRecency` records the
 * visits separately, so the eviction on the next overflow is LRU rather than
 * first-opened. Closing is still explicit — this only bounds the automatic
 * opening that happens on every visit.
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
  /**
   * Per-agent LRU lists of session ids, oldest visit first.
   *
   * Kept apart from `openChatTabs` because the strip renders that array in
   * order: moving a revisited tab would make the strip rotate under the
   * cursor. Only the eviction at `MAX_CHAT_TABS` reads this list.
   */
  chatTabRecency: Record<string, string[]>;
  openChatTab: (agentId: string, sessionId: string) => void;
  closeChatTab: (agentId: string, sessionId: string) => void;
  pruneChatTabs: (agentId: string, validSessionIds: Set<string>) => void;
  pruneChatTabAgents: (validAgentIds: Set<string>) => void;
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
  | "chatTabRecency"
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

/**
 * Drop ids matching `isDropped` from one agent's LRU list.
 *
 * Returns the same map when nothing changed, so callers that only close a tab
 * do not churn the persisted state.
 */
function removeFromRecency(
  recency: Record<string, string[]>,
  agentId: string,
  isDropped: (id: string) => boolean,
): Record<string, string[]> {
  const current = recency[agentId];
  if (!current) return recency;
  const next = current.filter((id) => !isDropped(id));
  if (next.length === current.length) return recency;
  const updated = { ...recency };
  if (next.length === 0) delete updated[agentId];
  else updated[agentId] = next;
  return updated;
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
    chatTabRecency: {},
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

  if (isRecord(persistedState.chatTabRecency)) {
    // Same untrusted-shape treatment as openChatTabs. Ids no longer present in
    // the (already sanitized) tab list are dropped here: the tabs themselves
    // are pruned elsewhere, but a stale recency list would outlive them.
    migrated.chatTabRecency = Object.fromEntries(
      Object.entries(persistedState.chatTabRecency)
        .map(([agentId, ids]) => [
          agentId,
          (Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : [])
            .filter((id) => (migrated.openChatTabs[agentId] ?? []).includes(id)),
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
      chatTabRecency: {},
      openChatTab: (agentId, sessionId) =>
        set((state) => {
          const current = state.openChatTabs[agentId] ?? [];
          const stored = state.chatTabRecency[agentId] ?? [];
          // Every open tab needs a slot in the LRU. Tabs that predate recency
          // (an older persisted store, or a hand-edited one) count as older
          // than any session with a recorded visit.
          const known = stored.filter((id) => current.includes(id));
          const unknown = current.filter((id) => !known.includes(id));
          const previous = [...unknown, ...known];
          // Already the most recent visit: return the same state so the effect
          // that opens a tab on every active-session change does not produce a
          // new object on each render.
          if (previous[previous.length - 1] === sessionId) return state;

          const isNew = !current.includes(sessionId);
          let nextTabs = current;
          if (isNew) {
            const visited = [...previous, sessionId];
            const overflow = visited.length - MAX_CHAT_TABS;
            if (overflow > 0) {
              // Evict by recency, not by display position: the leftmost tab
              // may be the one the operator returned to a minute ago.
              const evicted = new Set(visited.slice(0, overflow));
              nextTabs = current.filter((id) => !evicted.has(id));
            }
            // Append-only: new sessions go to the end, existing ones stay put.
            nextTabs = [...nextTabs, sessionId];
          }
          const recency = [...previous.filter((id) => id !== sessionId), sessionId].filter(
            (id) => nextTabs.includes(id),
          );
          if (nextTabs === current) {
            // Revisit: the strip order does not move, only recency.
            return { chatTabRecency: { ...state.chatTabRecency, [agentId]: recency } };
          }
          return {
            openChatTabs: { ...state.openChatTabs, [agentId]: nextTabs },
            chatTabRecency: { ...state.chatTabRecency, [agentId]: recency },
          };
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
          return {
            openChatTabs: tabs,
            chatTabRecency: removeFromRecency(
              state.chatTabRecency,
              agentId,
              (id) => id === sessionId,
            ),
          };
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
          return {
            openChatTabs: tabs,
            chatTabRecency: removeFromRecency(
              state.chatTabRecency,
              agentId,
              (id) => !validSessionIds.has(id),
            ),
          };
        }),
      pruneChatTabAgents: (validAgentIds) =>
        set((state) => {
          const stale = Object.keys(state.openChatTabs).filter((id) => !validAgentIds.has(id));
          if (stale.length === 0) return state;
          const openChatTabs = { ...state.openChatTabs };
          const chatTabRecency = { ...state.chatTabRecency };
          for (const agentId of stale) {
            delete openChatTabs[agentId];
            delete chatTabRecency[agentId];
          }
          return { openChatTabs, chatTabRecency };
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
        chatTabRecency: state.chatTabRecency,
      }),
    }
  )
);

// Two windows on the same origin share localStorage but not memory. Without
// this listener the second window's next write replaces the first window's
// whole persisted record, including its tab strip. `storage` only fires in
// the *other* windows, so rehydrating there keeps them in sync; the record is
// still written whole, so simultaneous edits remain last-write-wins.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key !== "librefang-ui-storage") return;
    void useUIStore.persist.rehydrate();
  });
}
