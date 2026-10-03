import { beforeEach, describe, expect, it, vi } from "vitest";

const changeLanguage = vi.hoisted(() => vi.fn<() => Promise<void>>());

vi.mock("./i18n", () => ({
  default: {
    language: "en",
    changeLanguage,
  },
}));

beforeEach(() => {
  localStorage.clear();
  changeLanguage.mockReset().mockResolvedValue(undefined);
  vi.resetModules();
});

describe("UI store persistence", () => {
  it("sanitizes legacy persisted state before merging defaults", async () => {
    localStorage.setItem(
      "librefang-ui-storage",
      JSON.stringify({
        version: 0,
        state: {
          theme: "removed-theme",
          language: "zh",
          navLayout: "removed-layout",
          isSidebarCollapsed: true,
          collapsedNavGroups: { runtime: true, invalid: "yes" },
          hiddenModelKeys: ["valid", 42],
        },
      }),
    );

    const { useUIStore } = await import("./store");
    const state = useUIStore.getState();
    expect(state.theme).toBe("dark");
    expect(state.language).toBe("zh");
    expect(state.navLayout).toBe("grouped");
    expect(state.isSidebarCollapsed).toBe(true);
    expect(state.collapsedNavGroups).toEqual({ runtime: true });
    expect(state.hiddenModelKeys).toEqual(["valid"]);
  });

  it("syncs i18n after persisted language rehydration", async () => {
    localStorage.setItem(
      "librefang-ui-storage",
      JSON.stringify({ version: 1, state: { language: "uk" } }),
    );

    await import("./store");
    expect(changeLanguage).toHaveBeenCalledWith("uk");
  });

  it("commits a language only after i18n succeeds", async () => {
    const { useUIStore } = await import("./store");
    useUIStore.setState({ language: "en" });

    changeLanguage.mockRejectedValueOnce(new Error("unavailable"));
    await expect(useUIStore.getState().setLanguage("ko")).resolves.toBe(false);
    expect(useUIStore.getState().language).toBe("en");

    await expect(useUIStore.getState().setLanguage("pl")).resolves.toBe(true);
    expect(useUIStore.getState().language).toBe("pl");
  });

  it("prunes stale collapsed navigation keys", async () => {
    const { useUIStore } = await import("./store");
    useUIStore.setState({
      collapsedNavGroups: { primary: true, runtime: false, removed: true },
    });

    useUIStore.getState().pruneCollapsedNavGroups(new Set(["primary", "runtime"]));
    expect(useUIStore.getState().collapsedNavGroups).toEqual({
      primary: true,
      runtime: false,
    });
  });
});

describe("chat transcript scale", () => {
  it("clamps a persisted value instead of trusting it", async () => {
    const { clampChatScale, MIN_CHAT_SCALE, MAX_CHAT_SCALE, DEFAULT_CHAT_SCALE } =
      await import("./store");

    // localStorage is user-editable, and a persisted 0 would collapse the
    // transcript to nothing with no visible control left to recover it.
    expect(clampChatScale(0)).toBe(MIN_CHAT_SCALE);
    expect(clampChatScale(-5)).toBe(MIN_CHAT_SCALE);
    expect(clampChatScale(99)).toBe(MAX_CHAT_SCALE);
    expect(clampChatScale(Number.NaN)).toBe(DEFAULT_CHAT_SCALE);
    expect(clampChatScale(Number.POSITIVE_INFINITY)).toBe(DEFAULT_CHAT_SCALE);
    expect(clampChatScale("1.1")).toBe(DEFAULT_CHAT_SCALE);
    expect(clampChatScale(undefined)).toBe(DEFAULT_CHAT_SCALE);
    // A value inside the range passes through untouched.
    expect(clampChatScale(1)).toBe(1);
  });

  it("carries a stored scale through the migration, clamped", async () => {
    const { migratePersistedUIState, MAX_CHAT_SCALE, DEFAULT_CHAT_SCALE } =
      await import("./store");

    expect(migratePersistedUIState({ chatScale: 1.1 }).chatScale).toBe(1.1);
    expect(migratePersistedUIState({ chatScale: 40 }).chatScale).toBe(MAX_CHAT_SCALE);
    // A profile saved before this setting existed gets the default, not `undefined`.
    expect(migratePersistedUIState({ theme: "light" }).chatScale).toBe(DEFAULT_CHAT_SCALE);
  });

  it("clamps through the setter too, not only on load", async () => {
    const { useUIStore, MIN_CHAT_SCALE, MAX_CHAT_SCALE } = await import("./store");

    useUIStore.getState().setChatScale(10);
    expect(useUIStore.getState().chatScale).toBe(MAX_CHAT_SCALE);
    useUIStore.getState().setChatScale(0.1);
    expect(useUIStore.getState().chatScale).toBe(MIN_CHAT_SCALE);
  });
});

describe("chat session tabs", () => {
  it("keeps a session out of the strip twice and bounds how many accumulate", async () => {
    const { useUIStore, MAX_CHAT_TABS } = await import("./store");
    useUIStore.setState({ openChatTabs: {} });

    useUIStore.getState().openChatTab("agent-a", "s1");
    useUIStore.getState().openChatTab("agent-a", "s1");
    expect(useUIStore.getState().openChatTabs["agent-a"]).toEqual(["s1"]);

    // Every session ever visited would make a strip too wide to use, and the
    // oldest is the one least likely to be wanted back.
    for (let i = 0; i < MAX_CHAT_TABS + 5; i += 1) {
      useUIStore.getState().openChatTab("agent-a", `bulk-${i}`);
    }
    const tabs = useUIStore.getState().openChatTabs["agent-a"];
    expect(tabs).toHaveLength(MAX_CHAT_TABS);
    expect(tabs).not.toContain("s1");
    expect(tabs[tabs.length - 1]).toBe(`bulk-${MAX_CHAT_TABS + 4}`);
  });

  it("forgets an agent entirely once its last tab closes", async () => {
    const { useUIStore } = await import("./store");
    useUIStore.setState({ openChatTabs: {} });

    useUIStore.getState().openChatTab("agent-b", "s1");
    useUIStore.getState().closeChatTab("agent-b", "s1");
    // Not an empty array: the persisted object would otherwise grow one entry
    // per agent ever opened and never shrink.
    expect(useUIStore.getState().openChatTabs).not.toHaveProperty("agent-b");
  });

  it("drops tabs for sessions the server no longer has", async () => {
    const { useUIStore } = await import("./store");
    useUIStore.setState({ openChatTabs: { "agent-c": ["alive", "deleted"] } });

    useUIStore.getState().pruneChatTabs("agent-c", new Set(["alive"]));
    expect(useUIStore.getState().openChatTabs["agent-c"]).toEqual(["alive"]);
  });

  it("refuses a persisted shape that is not a list of strings", async () => {
    const { migratePersistedUIState } = await import("./store");
    // localStorage is user-editable; a non-array or a list of objects would
    // reach the tab strip's `.map` and render `undefined` keys.
    const migrated = migratePersistedUIState({
      openChatTabs: { a: "not-an-array", b: ["ok", 42, null], c: [] },
    });
    expect(migrated.openChatTabs).toEqual({ b: ["ok"] });
  });
});
