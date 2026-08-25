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
